# Background Collectors

## Overview

The background collector framework replaces the synchronous pull-based model with independent polling threads that feed a shared snapshot. This keeps the UI thread non-blocking and allows each data source to poll at its own interval.

## Threading Model

The application uses `std::thread` (not tokio). All existing collectors are synchronous (`rusqlite`, `reqwest::blocking`), so a full async runtime would add complexity without benefit.

```text
main thread (UI / TUI)
  ├─ spawn OpenCodeCollector   (interval: 30s, default enabled)
  ├─ spawn JournalCollector    (interval: 60s, default enabled)
  └─ spawn ZenPricingCollector (interval: 3600s, default disabled)
       │
       ▼
  Arc<RwLock<CollectorState>>  ← UI reads snapshot via .snapshot()
  Arc<AtomicBool> shutdown     ← UI or SIGINT sets to true
```

## Core Types

### Collector Trait

Every data source implements `Collector`:

```rust
pub trait Collector: Send + 'static {
    fn name(&self) -> &str;
    fn interval(&self) -> Duration;
    fn poll(&mut self) -> Result<Vec<Usage>>;
}
```

- `poll()` returns a fresh batch of `Usage` events (may be empty).
- `poll()` must not panic; errors are captured into `CollectorState::errors`.
- Collectors that produce side effects only (e.g., Zen pricing cache refresh) return an empty `Vec` and log a status message.

### CollectorState

Shared state written by collector threads, read by the UI:

```rust
pub struct CollectorState {
    pub usages: Vec<Usage>,
    pub sources: Vec<String>,
    pub last_poll: HashMap<String, Instant>,
    pub errors: HashMap<String, String>,
}
```

### CollectorHandle

Returned by `spawn()`, owned by the UI:

```rust
pub struct CollectorHandle {
    state: Arc<RwLock<CollectorState>>,
    shutdown: Arc<AtomicBool>,
}

impl CollectorHandle {
    pub fn spawn(collectors: Vec<Box<dyn Collector>>) -> Self;
    pub fn snapshot(&self) -> Vec<Usage>;
    pub fn status(&self) -> String;
    pub fn shutdown(&self);
}
```

## Built-in Collectors

| Collector | Wraps | Default Interval | Default Enabled |
|-----------|-------|-----------------|-----------------|
| `OpenCodeCollector` | `load_opencode()` | 30s | yes |
| `JournalCollector` | `load_journal()` | 60s | yes |
| `ZenPricingCollector` | `refresh_pricing()` | 3600s | no |

Each built-in wraps existing logic — no rewrite of `load_opencode` / `load_journal`. The trait adds polling cadence and error capture.

## Polling Loop (per thread)

1. Check `shutdown` flag → if true, exit.
2. Call `poll()`.
3. On success: merge results into `CollectorState`, update `last_poll`, clear error.
4. On error: write error message to `CollectorState::errors`.
5. Sleep for `interval()` (check `shutdown` flag on wake).
6. Loop.

## Dedup and Pricing

Deduplication and estimated pricing are applied in the collector thread (not the UI thread) after merging results from all collectors. This keeps `snapshot()` reads instant.

## Shutdown

- UI `q`/`Esc`/`Ctrl-C` → `CollectorHandle::shutdown()` sets `AtomicBool`.
- Each collector thread checks the flag at the top of every poll cycle and after sleep.
- Maximum shutdown latency = longest collector interval.
- `--once`/`--json`/`--csv` modes bypass background collectors entirely (synchronous path).

## Config Schema

```toml
[collectors.opencode]
enabled = true
interval = 30

[collectors.journal]
enabled = true
interval = 60

[collectors.zen_pricing]
enabled = false
interval = 3600
```

## Fallback Behavior

| Scenario | Behavior |
|----------|----------|
| OpenCode DB missing | Collector returns empty vec, status shows "No OpenCode database" |
| Journal not initialized | Collector returns empty vec, status shows "journal: not initialized" |
| Zen pricing fetch fails | Error logged to `CollectorState::errors`, bundled pricing used |
| Collector thread panics | `catch_unwind` captures panic, writes to errors map |
| `--once` / `--json` / `--csv` | Synchronous `load_usage()`, no background threads |