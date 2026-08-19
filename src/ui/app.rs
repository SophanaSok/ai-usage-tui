//! Dashboard state and the derived views rendered from it.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::budget::{Alert, BudgetEngine};
use crate::collector::background::CollectorHandle;
use crate::model::{
    BurnRate, Category, CostStatus, DayTotals, ProjectTotals, Range, RoutingAggregates,
    SessionTotals, Totals, Usage,
};
use crate::utils::format_clock;

use super::aggregate::{burn_rate, coverage, daily_totals, project_totals, session_totals};

/// Trailing window for the burn-rate panel. One hour is long enough to smooth out a single
/// large request and short enough to reflect what you are doing now.
const BURN_WINDOW_SECS: i64 = 3600;

pub struct App {
    pub range: Range,
    pub usages: Vec<Usage>,
    pub selected: usize,
    pub status: String,
    /// Whether a collector is failing, restarting, dead, or stale. A monitor that goes quiet
    /// looks exactly like a monitor with nothing to report, so this is rendered, not logged.
    pub degraded: bool,
    pub last_refresh: String,
    pub pulse: u64,
    pub refresh_interval: Duration,
    pub refreshed_at: Instant,
    pub db_path: Option<PathBuf>,
    pub journal_path: PathBuf,
    pub claude_dir: Option<PathBuf>,
    pub provider_filter: Option<String>,
    pub model_filter: Option<String>,
    pub collector: Option<CollectorHandle>,
    /// Which view occupies the right-hand pane. This was two independent booleans, so
    /// "budgets on" and "routing on" could both be true and one silently won.
    pub panel: Panel,
    pub budget_engine: BudgetEngine,
    pub alerts: Vec<Alert>,
    /// Alerts are handed to a worker thread; the webhook POST is blocking and must never
    /// happen on the render path.
    ///
    /// `pub(super)` so the ui module's own tests can construct an `App` without a collector or
    /// a webhook worker. Deliberately not `pub(crate)`: nothing outside `ui` should reach in.
    pub(super) alert_sink: Option<Sender<Vec<Alert>>>,
    /// Derived views, recomputed only when the data or the filters change.
    ///
    /// These were previously rebuilt inside `draw`, which cloned every `Usage` roughly eight
    /// times per frame at 4fps and re-read the routing table from SQLite on the render thread.
    pub(super) view: DerivedView,
}

/// The right-hand pane's contents. Exactly one at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    #[default]
    Models,
    Budgets,
    Routing,
    Projects,
    TimeSeries,
    Burn,
    Sessions,
}

#[derive(Default)]
pub struct DerivedView {
    filtered: Vec<Usage>,
    rows: Vec<Usage>,
    totals: Totals,
    category_totals: Vec<(Category, Totals)>,
    routing: Vec<RoutingAggregates>,
    projects: Vec<ProjectTotals>,
    daily: Vec<DayTotals>,
    burn: BurnRate,
    sessions: Vec<SessionTotals>,
    coverage: Coverage,
}

/// How much of the visible usage the pricing engine could actually price.
///
/// Provenance is the project's differentiator but it lived entirely in an internal enum: a
/// user could read a total without knowing it covered two thirds of their requests.
#[derive(Clone, Copy, Debug, Default)]
pub struct Coverage {
    pub priced_requests: u64,
    pub billable_requests: u64,
}

impl Coverage {
    /// `None` when nothing billable is in range — 100% of nothing is not a useful claim.
    pub fn pct(&self) -> Option<f64> {
        if self.billable_requests == 0 {
            return None;
        }
        Some((self.priced_requests as f64 / self.billable_requests as f64) * 100.0)
    }
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db_path: Option<PathBuf>,
        journal_path: PathBuf,
        claude_dir: Option<PathBuf>,
        range: Range,
        refresh_interval: Duration,
        provider_filter: Option<String>,
        model_filter: Option<String>,
        collector: Option<CollectorHandle>,
        budget_engine: BudgetEngine,
        alert_sink: Option<Sender<Vec<Alert>>>,
    ) -> Self {
        let mut app = Self {
            range,
            usages: Vec::new(),
            selected: 0,
            status: String::new(),
            degraded: false,
            last_refresh: String::from("never"),
            pulse: 0,
            refresh_interval,
            refreshed_at: Instant::now(),
            db_path,
            journal_path,
            claude_dir,
            provider_filter,
            model_filter,
            collector,
            panel: Panel::Models,
            budget_engine,
            alerts: Vec::new(),
            alert_sink,
            view: DerivedView::default(),
        };
        app.refresh();
        app
    }

    /// Rebuild every derived view from `usages`. Call after data or filters change.
    pub fn recompute(&mut self) {
        let cutoff = self.range.cutoff();
        let is_all = self.range == Range::All;
        let provider = self.provider_filter.as_deref();
        let model = self.model_filter.as_deref();

        self.view.filtered = self
            .usages
            .iter()
            .filter(|u| is_all || u.created >= cutoff)
            .filter(|u| provider.is_none_or(|p| u.provider.eq_ignore_ascii_case(p)))
            .filter(|u| model.is_none_or(|m| u.model.eq_ignore_ascii_case(m)))
            .cloned()
            .collect();

        self.view.totals = self
            .view
            .filtered
            .iter()
            .fold(Totals::default(), |mut t, u| {
                t.add(u);
                t
            });

        self.view.category_totals = [
            Category::Local,
            Category::Free,
            Category::Paid,
            Category::Cloud,
            Category::Unknown,
        ]
        .into_iter()
        .map(|category| {
            let totals = self
                .view
                .filtered
                .iter()
                .filter(|u| u.category == category)
                .fold(Totals::default(), |mut t, u| {
                    t.add(u);
                    t
                });
            (category, totals)
        })
        .collect();

        self.view.projects = project_totals(&self.view.filtered);
        self.view.daily = daily_totals(&self.view.filtered);
        self.view.sessions = session_totals(&self.view.filtered);
        // Computed here, once per refresh, because `burn_rate` needs the clock and the render
        // path must not read it.
        self.view.burn = burn_rate(&self.usages, BURN_WINDOW_SECS, crate::utils::now());
        self.view.coverage = coverage(&self.view.filtered);

        let mut grouped = BTreeMap::<(String, String, Category, CostStatus), Usage>::new();
        for u in &self.view.filtered {
            let key = (
                u.provider.clone(),
                u.model.clone(),
                u.category,
                u.cost_status,
            );
            let entry = grouped.entry(key).or_insert_with(|| Usage {
                provider: u.provider.clone(),
                model: u.model.clone(),
                category: u.category,
                cost_status: u.cost_status,
                ..Default::default()
            });
            entry.requests += u.requests;
            entry.input += u.input;
            entry.output += u.output;
            entry.reasoning += u.reasoning;
            entry.cache_read += u.cache_read;
            entry.cache_write += u.cache_write;
            if u.cost_status.is_billable() {
                if let Some(cost) = u.cost {
                    entry.cost = Some(entry.cost.unwrap_or(0.0) + cost);
                }
            }
        }
        self.view.rows = grouped.into_values().collect();
        self.view.rows.sort_by_key(|u| Reverse(u.total_tokens()));

        self.selected = self.selected.min(self.view.rows.len().saturating_sub(1));
    }

    pub fn projects(&self) -> &[ProjectTotals] {
        &self.view.projects
    }

    pub fn daily(&self) -> &[DayTotals] {
        &self.view.daily
    }

    pub fn sessions(&self) -> &[SessionTotals] {
        &self.view.sessions
    }

    /// How many rows the visible panel has, for clamping the selection.
    ///
    /// The selection used to clamp to the model table's length unconditionally, so on any other
    /// table panel it either stopped short or ran past the end.
    pub fn visible_rows(&self) -> usize {
        match self.panel {
            Panel::Sessions => self.view.sessions.len(),
            _ => self.view.rows.len(),
        }
    }

    pub fn burn(&self) -> &BurnRate {
        &self.view.burn
    }

    pub fn alerts(&self) -> &[Alert] {
        &self.alerts
    }

    pub fn coverage(&self) -> Coverage {
        self.view.coverage
    }

    /// Toggle a panel on, or back to the model list if it is already showing.
    pub fn toggle_panel(&mut self, panel: Panel) {
        self.panel = if self.panel == panel {
            Panel::Models
        } else {
            panel
        };
    }

    /// Change the visible range and rebuild derived views.
    pub fn set_range(&mut self, range: Range) {
        self.range = range;
        self.recompute();
    }
    pub fn refresh(&mut self) {
        if let Some(ref collector) = self.collector {
            self.usages = collector.snapshot();
            self.status = collector.status();
            self.degraded = collector.is_degraded();
        } else {
            match crate::collector::load_usage(
                self.db_path.as_deref(),
                &self.journal_path,
                self.claude_dir.as_deref(),
            ) {
                Ok((usages, source)) => {
                    self.usages = usages;
                    self.status = source;
                    self.degraded = false;
                }
                Err(error) => {
                    self.usages.clear();
                    self.status = format!("OpenCode unavailable: {}", error);
                    self.degraded = true;
                }
            }
        }
        self.last_refresh = format_clock();
        self.refreshed_at = Instant::now();
        self.recompute();
        // Routing lives in SQLite; read it here, not inside `draw`.
        self.view.routing = match crate::collector::journal::load_routing(&self.journal_path) {
            Ok(events) => crate::routing::aggregate(&events),
            Err(error) => {
                // An empty routing panel used to be the rendering for both "no events yet"
                // and "the journal could not be read". Only one of those is fine.
                crate::logging::error("routing", &format!("journal read failed: {}", error));
                self.status = format!("{} | routing unavailable: {}", self.status, error);
                self.degraded = true;
                Vec::new()
            }
        };
        if !self.budget_engine.is_empty() {
            self.alerts = self.budget_engine.check(&self.usages);
            if let Some(sink) = &self.alert_sink {
                if self.alerts.iter().any(Alert::is_actionable) {
                    // Best-effort: a dead worker must not take the dashboard down with it.
                    let _ = sink.send(self.alerts.clone());
                }
            }
        }
    }
    pub fn refresh_if_due(&mut self) {
        if self.refreshed_at.elapsed() >= self.refresh_interval {
            self.refresh();
        }
    }
    pub fn filtered(&self) -> &[Usage] {
        &self.view.filtered
    }
    pub fn rows(&self) -> &[Usage] {
        &self.view.rows
    }
    pub fn totals(&self) -> &Totals {
        &self.view.totals
    }
    pub fn category_totals(&self) -> &[(Category, Totals)] {
        &self.view.category_totals
    }
    #[cfg(test)]
    pub(super) fn set_routing_for_test(&mut self, routing: Vec<RoutingAggregates>) {
        self.view.routing = routing;
    }

    pub fn routing(&self) -> &[RoutingAggregates] {
        &self.view.routing
    }
}
