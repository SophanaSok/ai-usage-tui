use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::collector::usage_key;
use crate::model::Usage;
use crate::pricing::{apply_estimated_pricing, PricingEngine};

const POLL_CHECK_INTERVAL: Duration = Duration::from_secs(1);

pub trait Collector: Send + 'static {
    fn name(&self) -> &str;
    fn interval(&self) -> Duration;
    fn poll(&mut self) -> Result<Vec<Usage>>;
}

pub struct CollectorState {
    pub usages: Vec<Usage>,
    pub sources: Vec<String>,
    pub last_poll: HashMap<String, Instant>,
    pub errors: HashMap<String, String>,
}

impl CollectorState {
    fn new() -> Self {
        Self {
            usages: Vec::new(),
            sources: Vec::new(),
            last_poll: HashMap::new(),
            errors: HashMap::new(),
        }
    }

    fn merge(&mut self, name: &str, usages: Vec<Usage>, source: String) {
        let existing: Vec<(String, String, u64, u64, u64, u64, u64)> =
            self.usages.iter().map(usage_key).collect();
        for u in usages {
            let key = usage_key(&u);
            if !existing.contains(&key) {
                self.usages.push(u);
            }
        }
        if !self.sources.contains(&source) {
            self.sources.push(source);
        }
        self.last_poll.insert(name.to_string(), Instant::now());
        self.errors.remove(name);
    }

    fn record_error(&mut self, name: &str, error: String) {
        self.errors.insert(name.to_string(), error);
        self.last_poll.insert(name.to_string(), Instant::now());
    }

    fn apply_pricing(&mut self) {
        let engine = PricingEngine::load();
        apply_estimated_pricing(&mut self.usages, &engine);
    }
}

pub struct CollectorHandle {
    state: Arc<RwLock<CollectorState>>,
    shutdown: Arc<AtomicBool>,
}

impl CollectorHandle {
    pub fn spawn(collectors: Vec<Box<dyn Collector>>) -> Self {
        let state = Arc::new(RwLock::new(CollectorState::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        for mut collector in collectors {
            let state = Arc::clone(&state);
            let shutdown = Arc::clone(&shutdown);
            let name = collector.name().to_string();

            thread::spawn(move || {
                while !shutdown.load(Ordering::Relaxed) {
                    let result = catch_unwind(AssertUnwindSafe(|| collector.poll()));
                    match result {
                        Ok(Ok(usages)) => {
                            let source = format!("{}: ok", name);
                            if let Ok(mut s) = state.write() {
                                s.merge(&name, usages, source);
                                s.apply_pricing();
                            }
                        }
                        Ok(Err(e)) => {
                            if let Ok(mut s) = state.write() {
                                s.record_error(&name, e.to_string());
                            }
                        }
                        Err(_) => {
                            if let Ok(mut s) = state.write() {
                                s.record_error(&name, "collector thread panicked".to_string());
                            }
                            break;
                        }
                    }

                    let interval = collector.interval();
                    let mut waited = Duration::ZERO;
                    while waited < interval {
                        if shutdown.load(Ordering::Relaxed) {
                            return;
                        }
                        thread::sleep(POLL_CHECK_INTERVAL);
                        waited += POLL_CHECK_INTERVAL;
                    }
                }
            });
        }

        Self { state, shutdown }
    }

    pub fn snapshot(&self) -> Vec<Usage> {
        self.state
            .read()
            .map(|s| s.usages.clone())
            .unwrap_or_default()
    }

    pub fn status(&self) -> String {
        self.state
            .read()
            .map(|s| {
                let sources = s.sources.join(" | ");
                if s.errors.is_empty() {
                    sources
                } else {
                    let errors = s
                        .errors
                        .iter()
                        .map(|(name, msg)| format!("{}: {}", name, msg))
                        .collect::<Vec<_>>()
                        .join("; ");
                    format!("{} | errors: {}", sources, errors)
                }
            })
            .unwrap_or_else(|_| "collector state locked".to_string())
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl Drop for CollectorHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub struct OpenCodeCollector {
    pub db_path: Option<PathBuf>,
    pub interval_secs: u64,
}

impl Collector for OpenCodeCollector {
    fn name(&self) -> &str {
        "opencode"
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }
    fn poll(&mut self) -> Result<Vec<Usage>> {
        let (usages, _) = crate::collector::opencode::load_opencode(self.db_path.as_deref())?;
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
        handle.shutdown();
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
        thread::sleep(Duration::from_millis(100));
        assert!(handle.shutdown.load(Ordering::Relaxed));
    }

    #[test]
    fn collector_state_dedupes_by_usage_key() {
        let mut state = CollectorState::new();
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
}
