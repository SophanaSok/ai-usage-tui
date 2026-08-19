use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::collector::{usage_key, UsageKey};
use crate::logging;
use crate::model::Usage;
use crate::pricing::{apply_estimated_pricing, PricingEngine};

/// How many times a collector may panic before it is given up on. A collector that panics on
/// every poll is a bug, not a transient fault; retrying it forever would burn a core and spam
/// the log without ever producing data.
const MAX_RESTARTS: u32 = 5;
const RESTART_BACKOFF_BASE: Duration = Duration::from_secs(2);
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(60);
/// A collector is called stale once it has missed this many consecutive polls.
const STALE_INTERVALS: u32 = 3;

pub trait Collector: Send + 'static {
    fn name(&self) -> &str;
    fn interval(&self) -> Duration;
    fn poll(&mut self) -> Result<Vec<Usage>>;
}

/// What a collector thread is doing, as far as the dashboard is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Liveness {
    /// Spawned, no poll has completed yet.
    Starting,
    /// Last poll succeeded.
    Live,
    /// Still polling, but the last attempt returned an error.
    Failing,
    /// Panicked; waiting out a backoff before the next attempt.
    Restarting,
    /// Panicked past `MAX_RESTARTS`. Nothing will ever update this source again.
    Dead,
}

impl Liveness {
    pub fn label(self) -> &'static str {
        match self {
            Liveness::Starting => "starting",
            Liveness::Live => "ok",
            Liveness::Failing => "failing",
            Liveness::Restarting => "restarting",
            Liveness::Dead => "dead",
        }
    }

    /// Whether this state means the numbers on screen may be silently incomplete.
    pub fn is_degraded(self) -> bool {
        matches!(
            self,
            Liveness::Failing | Liveness::Restarting | Liveness::Dead
        )
    }
}

#[derive(Clone, Debug)]
pub struct Health {
    pub liveness: Liveness,
    pub interval: Duration,
    pub last_ok: Option<Instant>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub restarts: u32,
}

impl Health {
    fn new(interval: Duration) -> Self {
        Self {
            liveness: Liveness::Starting,
            interval,
            last_ok: None,
            last_error: None,
            consecutive_failures: 0,
            restarts: 0,
        }
    }

    /// A collector that is nominally live but has not produced a successful poll in several
    /// intervals. Distinct from `Failing`: a poll can hang without ever returning an error.
    pub fn is_stale(&self, now: Instant) -> bool {
        if self.liveness != Liveness::Live {
            return false;
        }
        let limit = self.interval.saturating_mul(STALE_INTERVALS);
        match self.last_ok {
            Some(at) => now.duration_since(at) > limit,
            None => true,
        }
    }

    pub fn is_degraded(&self, now: Instant) -> bool {
        self.liveness.is_degraded() || self.is_stale(now)
    }
}

/// Recover a poisoned lock instead of skipping the write.
///
/// This used to be `if let Ok(mut s) = state.write()`, which meant a single panic anywhere
/// under the lock froze the dashboard permanently: every later write became a no-op and the UI
/// went on rendering the last good snapshot as if it were current. Recovering can expose a
/// partially applied merge — at worst one usage row present in the dedup index but missing
/// from the list — which is a bounded, one-row loss rather than an unbounded, silent one.
fn write_state(state: &RwLock<CollectorState>) -> RwLockWriteGuard<'_, CollectorState> {
    state.write().unwrap_or_else(|poison| poison.into_inner())
}

fn read_state(state: &RwLock<CollectorState>) -> RwLockReadGuard<'_, CollectorState> {
    state.read().unwrap_or_else(|poison| poison.into_inner())
}

pub struct CollectorState {
    pub usages: Vec<Usage>,
    /// Membership index over `usages`, maintained alongside it. Rebuilding this per poll and
    /// scanning it linearly made merges quadratic in total history, on every poll, forever.
    seen: HashSet<UsageKey>,
    pub sources: Vec<String>,
    pub health: HashMap<String, Health>,
    /// Pricing is immutable per process; loading it inside the write lock on every poll
    /// blocked `snapshot()` on a TOML parse.
    pricing: Arc<PricingEngine>,
}

impl CollectorState {
    fn new(pricing: Arc<PricingEngine>) -> Self {
        Self {
            usages: Vec::new(),
            seen: HashSet::new(),
            sources: Vec::new(),
            health: HashMap::new(),
            pricing,
        }
    }

    #[cfg(test)]
    fn for_test() -> Self {
        Self::new(Arc::new(PricingEngine::bundled()))
    }

    fn register(&mut self, name: &str, interval: Duration) {
        self.health
            .entry(name.to_string())
            .or_insert_with(|| Health::new(interval));
    }

    fn entry(&mut self, name: &str) -> &mut Health {
        self.health
            .entry(name.to_string())
            .or_insert_with(|| Health::new(Duration::from_secs(30)))
    }

    fn merge(&mut self, name: &str, usages: Vec<Usage>, source: String) {
        for u in usages {
            if self.seen.insert(usage_key(&u)) {
                self.usages.push(u);
            }
        }
        if !self.sources.contains(&source) {
            self.sources.push(source);
        }
        let health = self.entry(name);
        health.liveness = Liveness::Live;
        health.last_ok = Some(Instant::now());
        health.last_error = None;
        health.consecutive_failures = 0;
    }

    fn record_error(&mut self, name: &str, error: String) {
        let health = self.entry(name);
        health.liveness = Liveness::Failing;
        health.consecutive_failures = health.consecutive_failures.saturating_add(1);
        health.last_error = Some(error);
    }

    fn record_panic(&mut self, name: &str, liveness: Liveness) -> u32 {
        let health = self.entry(name);
        health.liveness = liveness;
        health.restarts = health.restarts.saturating_add(1);
        health.last_error = Some("collector thread panicked".to_string());
        health.restarts
    }

    fn apply_pricing(&mut self) {
        let pricing = Arc::clone(&self.pricing);
        apply_estimated_pricing(&mut self.usages, &pricing);
    }
}

/// Shutdown signal that a sleeping collector can be woken from.
///
/// The old version polled an `AtomicBool` once a second, so `shutdown()` could take up to a
/// second to take effect — and nothing joined the threads, so they could outlive the handle
/// and keep touching state after the caller believed they were gone.
struct Shutdown {
    flag: Mutex<bool>,
    signal: Condvar,
}

impl Shutdown {
    fn new() -> Self {
        Self {
            flag: Mutex::new(false),
            signal: Condvar::new(),
        }
    }

    fn trigger(&self) {
        let mut flag = self.flag.lock().unwrap_or_else(|p| p.into_inner());
        *flag = true;
        self.signal.notify_all();
    }

    fn is_set(&self) -> bool {
        *self.flag.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Sleep for `duration`, returning `true` if shutdown was signalled instead.
    fn sleep(&self, duration: Duration) -> bool {
        let flag = self.flag.lock().unwrap_or_else(|p| p.into_inner());
        if *flag {
            return true;
        }
        let (flag, _) = self
            .signal
            .wait_timeout(flag, duration)
            .unwrap_or_else(|p| p.into_inner());
        *flag
    }
}

pub struct CollectorHandle {
    state: Arc<RwLock<CollectorState>>,
    shutdown: Arc<Shutdown>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl CollectorHandle {
    pub fn spawn(collectors: Vec<Box<dyn Collector>>) -> Self {
        // Load pricing once, not once per poll inside the write lock.
        let pricing = Arc::new(PricingEngine::load());
        let state = Arc::new(RwLock::new(CollectorState::new(pricing)));
        let shutdown = Arc::new(Shutdown::new());
        let mut threads = Vec::new();

        for mut collector in collectors {
            let state = Arc::clone(&state);
            let shutdown = Arc::clone(&shutdown);
            let name = collector.name().to_string();
            let interval = collector.interval();
            write_state(&state).register(&name, interval);

            threads.push(thread::spawn(move || {
                while !shutdown.is_set() {
                    let result = catch_unwind(AssertUnwindSafe(|| collector.poll()));
                    match result {
                        Ok(Ok(usages)) => {
                            let source = format!("{}: ok", name);
                            let count = usages.len();
                            let mut s = write_state(&state);
                            s.merge(&name, usages, source);
                            s.apply_pricing();
                            drop(s);
                            logging::info(&name, &format!("poll ok, {} usage rows", count));
                        }
                        Ok(Err(e)) => {
                            let message = e.to_string();
                            write_state(&state).record_error(&name, message.clone());
                            logging::error(&name, &format!("poll failed: {}", message));
                        }
                        Err(_) => {
                            // A panicking collector used to `break` here, silently retiring
                            // that source for the life of the process while the UI kept
                            // showing its last numbers as though they were still updating.
                            let restarts =
                                write_state(&state).record_panic(&name, Liveness::Restarting);
                            if restarts > MAX_RESTARTS {
                                write_state(&state).entry(&name).liveness = Liveness::Dead;
                                logging::error(
                                    &name,
                                    &format!("panicked {} times; giving up", restarts),
                                );
                                return;
                            }
                            let backoff = backoff_for(restarts);
                            logging::warn(
                                &name,
                                &format!(
                                    "panicked (attempt {}); restarting in {}s",
                                    restarts,
                                    backoff.as_secs()
                                ),
                            );
                            if shutdown.sleep(backoff) {
                                return;
                            }
                            write_state(&state).entry(&name).liveness = Liveness::Starting;
                            continue;
                        }
                    }

                    if shutdown.sleep(collector.interval()) {
                        return;
                    }
                }
            }));
        }

        Self {
            state,
            shutdown,
            threads: Mutex::new(threads),
        }
    }

    pub fn snapshot(&self) -> Vec<Usage> {
        read_state(&self.state).usages.clone()
    }

    /// Per-collector health, sorted by name so the status line does not reshuffle each frame.
    pub fn health(&self) -> Vec<(String, Health)> {
        let mut health: Vec<(String, Health)> = read_state(&self.state)
            .health
            .iter()
            .map(|(name, health)| (name.clone(), health.clone()))
            .collect();
        health.sort_by(|a, b| a.0.cmp(&b.0));
        health
    }

    /// Whether any collector is failing, restarting, dead, or stale — i.e. whether the totals
    /// on screen might be missing data without saying so.
    pub fn is_degraded(&self) -> bool {
        let now = Instant::now();
        read_state(&self.state)
            .health
            .values()
            .any(|health| health.is_degraded(now))
    }

    pub fn status(&self) -> String {
        let now = Instant::now();
        let state = read_state(&self.state);
        let mut problems: Vec<String> = state
            .health
            .iter()
            .filter(|(_, health)| health.is_degraded(now))
            .map(|(name, health)| {
                let label = if health.is_stale(now) {
                    "stale"
                } else {
                    health.liveness.label()
                };
                match &health.last_error {
                    Some(error) => format!("{} {}: {}", name, label, error),
                    None => format!("{} {}", name, label),
                }
            })
            .collect();
        problems.sort();
        let sources = state.sources.join(" | ");
        if problems.is_empty() {
            sources
        } else {
            format!("{} | DEGRADED: {}", sources, problems.join("; "))
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.trigger();
    }

    /// Signal shutdown and wait for every collector thread to exit.
    ///
    /// Without this, `Drop` only set a flag: threads could still be inside a poll — holding a
    /// read-only SQLite handle, or mid-write to the shared state — after the handle was gone.
    pub fn join(&self) {
        self.shutdown();
        let handles: Vec<JoinHandle<()>> = {
            let mut threads = self.threads.lock().unwrap_or_else(|p| p.into_inner());
            std::mem::take(&mut *threads)
        };
        for handle in handles {
            let _ = handle.join();
        }
    }
}

/// Exponential backoff, capped. `restarts` is 1-based.
fn backoff_for(restarts: u32) -> Duration {
    let exponent = restarts.saturating_sub(1).min(6);
    RESTART_BACKOFF_BASE
        .saturating_mul(1u32 << exponent)
        .min(RESTART_BACKOFF_MAX)
}

impl Drop for CollectorHandle {
    fn drop(&mut self) {
        self.join();
    }
}

pub struct OpenCodeCollector {
    pub db_path: Option<PathBuf>,
    pub interval_secs: u64,
    /// Resume point, so each poll reads only what arrived since the last one.
    pub cursor: crate::collector::opencode::Cursor,
}

impl Collector for OpenCodeCollector {
    fn name(&self) -> &str {
        "opencode"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }
    fn poll(&mut self) -> Result<Vec<Usage>> {
        let (usages, _, cursor) =
            crate::collector::opencode::load_opencode_since(self.db_path.as_deref(), self.cursor)?;
        self.cursor = cursor;
        Ok(usages)
    }
}

pub struct ClaudeCodeCollector {
    pub root: Option<PathBuf>,
    pub interval_secs: u64,
    /// Per-file byte offsets, so each poll tails only what was appended.
    pub offsets: crate::collector::claude_code::Offsets,
}

impl Collector for ClaudeCodeCollector {
    fn name(&self) -> &str {
        "claude_code"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }
    fn poll(&mut self) -> Result<Vec<Usage>> {
        let (usages, _) = crate::collector::claude_code::load_claude_code(
            self.root.as_deref(),
            &mut self.offsets,
        )?;
        Ok(usages)
    }
}

pub struct JournalCollector {
    pub journal_path: PathBuf,
    pub interval_secs: u64,
}

impl Collector for JournalCollector {
    fn name(&self) -> &str {
        "journal"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }
    fn poll(&mut self) -> Result<Vec<Usage>> {
        crate::collector::journal::load_journal(&self.journal_path)
    }
}

pub struct ZenPricingCollector {
    pub interval_secs: u64,
}

impl Collector for ZenPricingCollector {
    fn name(&self) -> &str {
        "zen_pricing"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }
    fn poll(&mut self) -> Result<Vec<Usage>> {
        crate::collector::pricing_refresh::refresh_pricing()?;
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubCollector {
        name: String,
        interval_secs: u64,
        usages: Vec<Usage>,
    }

    impl Collector for StubCollector {
        fn name(&self) -> &str {
            &self.name
        }
        fn interval(&self) -> Duration {
            Duration::from_secs(self.interval_secs)
        }
        fn poll(&mut self) -> Result<Vec<Usage>> {
            Ok(self.usages.clone())
        }
    }

    #[test]
    fn snapshot_returns_merged_usages() {
        let collectors: Vec<Box<dyn Collector>> = vec![
            Box::new(StubCollector {
                name: "a".into(),
                interval_secs: 999,
                usages: vec![Usage {
                    provider: "p".into(),
                    model: "m".into(),
                    input: 100,
                    ..Default::default()
                }],
            }),
            Box::new(StubCollector {
                name: "b".into(),
                interval_secs: 999,
                usages: vec![Usage {
                    provider: "p".into(),
                    model: "m2".into(),
                    input: 200,
                    ..Default::default()
                }],
            }),
        ];
        let handle = CollectorHandle::spawn(collectors);
        thread::sleep(Duration::from_millis(200));
        let snap = handle.snapshot();
        handle.join();
        assert!(snap.len() >= 2);
    }

    #[test]
    fn shutdown_stops_collectors() {
        let handle = CollectorHandle::spawn(vec![Box::new(StubCollector {
            name: "test".into(),
            interval_secs: 1,
            usages: vec![],
        })]);
        handle.shutdown();
        assert!(handle.shutdown.is_set());
        handle.join();
    }

    #[test]
    fn collector_state_dedupes_by_usage_key() {
        let mut state = CollectorState::for_test();
        let u1 = Usage {
            provider: "p".into(),
            model: "m".into(),
            input: 100,
            ..Default::default()
        };
        state.merge("a", vec![u1.clone()], "a: ok".into());
        state.merge("a", vec![u1], "a: ok".into());
        assert_eq!(state.usages.len(), 1);
    }

    #[test]
    fn drop_triggers_shutdown() {
        let handle = CollectorHandle::spawn(vec![Box::new(StubCollector {
            name: "test".into(),
            interval_secs: 1,
            usages: vec![],
        })]);
        drop(handle);
    }

    #[test]
    fn two_distinct_requests_with_identical_token_counts_both_survive() {
        // Agent loops produce many requests with byte-identical token counts (repeated tool
        // calls, retries, short confirmations). Collapsing them under-reports real spend.
        let mut state = CollectorState::for_test();
        let first = Usage {
            event_id: Some("msg_001".into()),
            provider: "anthropic".into(),
            model: "claude-sonnet-4.6".into(),
            requests: 1,
            input: 100,
            output: 50,
            created: 1_700_000_000,
            ..Default::default()
        };
        let second = Usage {
            event_id: Some("msg_002".into()),
            created: 1_700_000_060,
            ..first.clone()
        };
        // Separate merges on purpose: the old key was compared against a snapshot taken at
        // the top of each merge, so collapsing only showed up ACROSS polls, not within one.
        state.merge("opencode", vec![first], "opencode: ok".into());
        state.merge("opencode", vec![second], "opencode: ok".into());
        assert_eq!(
            state.usages.len(),
            2,
            "two distinct requests were collapsed into one"
        );
    }

    #[test]
    fn identical_untagged_requests_are_kept_apart_by_timestamp() {
        let mut state = CollectorState::for_test();
        let first = Usage {
            provider: "opencode".into(),
            model: "m".into(),
            requests: 1,
            input: 100,
            created: 1_700_000_000,
            ..Default::default()
        };
        let second = Usage {
            created: 1_700_000_060,
            ..first.clone()
        };
        state.merge("opencode", vec![first], "opencode: ok".into());
        state.merge("opencode", vec![second], "opencode: ok".into());
        assert_eq!(state.usages.len(), 2);
    }

    #[test]
    fn a_replayed_poll_does_not_double_count() {
        let mut state = CollectorState::for_test();
        let usage = Usage {
            event_id: Some("msg_001".into()),
            provider: "opencode".into(),
            model: "m".into(),
            requests: 1,
            input: 100,
            created: 1_700_000_000,
            ..Default::default()
        };
        // The collector re-reads the same rows every poll; replays must be idempotent.
        state.merge("opencode", vec![usage.clone()], "opencode: ok".into());
        state.merge("opencode", vec![usage.clone()], "opencode: ok".into());
        state.merge("opencode", vec![usage], "opencode: ok".into());
        assert_eq!(state.usages.len(), 1);
    }

    /// Panics on its first `n` polls, then succeeds. Models a collector hitting a transient
    /// bad row rather than a permanently broken one.
    struct FlakyCollector {
        name: String,
        panics_left: std::sync::Arc<std::sync::atomic::AtomicU32>,
        polls: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }

    impl Collector for FlakyCollector {
        fn name(&self) -> &str {
            &self.name
        }
        fn interval(&self) -> Duration {
            Duration::from_millis(10)
        }
        fn poll(&mut self) -> Result<Vec<Usage>> {
            use std::sync::atomic::Ordering;
            self.polls.fetch_add(1, Ordering::SeqCst);
            if self
                .panics_left
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                    n.checked_sub(1).or(Some(0))
                })
                .unwrap_or(0)
                > 0
            {
                panic!("collector exploded");
            }
            Ok(vec![Usage {
                event_id: Some("after-restart".into()),
                provider: "p".into(),
                model: "m".into(),
                input: 1,
                ..Default::default()
            }])
        }
    }

    #[test]
    fn a_panicking_collector_is_restarted_and_recovers() {
        use std::sync::atomic::{AtomicU32, Ordering};
        // The old supervisor `break`s on the first panic: this source would never produce a
        // row again, and the UI would keep showing its stale numbers with no indication.
        let polls = std::sync::Arc::new(AtomicU32::new(0));
        let handle = CollectorHandle::spawn(vec![Box::new(FlakyCollector {
            name: "flaky".into(),
            panics_left: std::sync::Arc::new(AtomicU32::new(1)),
            polls: std::sync::Arc::clone(&polls),
        })]);
        // One backoff is RESTART_BACKOFF_BASE; wait past it.
        let deadline = Instant::now() + Duration::from_secs(10);
        while handle.snapshot().is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        let snapshot = handle.snapshot();
        let health = handle.health();
        handle.join();
        assert!(
            polls.load(Ordering::SeqCst) >= 2,
            "collector was never retried"
        );
        assert_eq!(snapshot.len(), 1, "no data after the restart");
        assert_eq!(health[0].1.restarts, 1);
        assert_eq!(health[0].1.liveness, Liveness::Live);
    }

    #[test]
    fn a_poisoned_state_lock_still_accepts_writes() {
        // A panic while the write lock is held used to poison it permanently: every later
        // `state.write()` returned Err and was skipped, so the dashboard froze silently.
        let state = Arc::new(RwLock::new(CollectorState::for_test()));
        let poisoner = Arc::clone(&state);
        let _ = thread::spawn(move || {
            let _guard = write_state(&poisoner);
            panic!("poison the lock");
        })
        .join();
        assert!(state.is_poisoned());

        write_state(&state).merge(
            "opencode",
            vec![Usage {
                event_id: Some("after-poison".into()),
                provider: "p".into(),
                model: "m".into(),
                ..Default::default()
            }],
            "opencode: ok".into(),
        );
        assert_eq!(read_state(&state).usages.len(), 1);
    }

    #[test]
    fn degraded_collectors_are_named_in_the_status_line() {
        let mut state = CollectorState::for_test();
        state.register("opencode", Duration::from_secs(30));
        state.merge("opencode", vec![], "opencode: ok".into());
        state.register("claude_code", Duration::from_secs(30));
        state.record_error("claude_code", "permission denied".into());

        let now = Instant::now();
        assert!(state.health["claude_code"].is_degraded(now));
        assert!(!state.health["opencode"].is_degraded(now));
    }

    #[test]
    fn a_collector_that_stops_polling_is_reported_stale() {
        let mut health = Health::new(Duration::from_secs(30));
        health.liveness = Liveness::Live;
        let now = Instant::now();
        health.last_ok = Some(now);
        assert!(!health.is_stale(now));
        // Three missed intervals with no error is indistinguishable from a hung poll, which
        // is exactly the case an error-only status line cannot report.
        assert!(health.is_stale(now + Duration::from_secs(31 * 3)));
    }

    #[test]
    fn restart_backoff_grows_and_is_capped() {
        assert_eq!(backoff_for(1), RESTART_BACKOFF_BASE);
        assert_eq!(backoff_for(2), RESTART_BACKOFF_BASE * 2);
        assert_eq!(backoff_for(3), RESTART_BACKOFF_BASE * 4);
        assert!(backoff_for(20) <= RESTART_BACKOFF_MAX);
    }

    #[test]
    fn shutdown_wakes_a_sleeping_collector_immediately() {
        // The old loop woke once a second to check an AtomicBool, so quitting the dashboard
        // could hang for up to a full poll-check interval.
        let shutdown = Arc::new(Shutdown::new());
        let waker = Arc::clone(&shutdown);
        let started = Instant::now();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            waker.trigger();
        });
        assert!(shutdown.sleep(Duration::from_secs(30)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
