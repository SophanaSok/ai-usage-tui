//! Dashboard state and the derived views rendered from it.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::budget::{Alert, BudgetEngine};
use crate::collector::background::CollectorHandle;
use crate::collector::SourceRoots;
use crate::escalation::{self, Escalations};
use crate::model::{
    BurnRate, Category, CostStatus, DayTotals, ProjectTotals, Range, RoutingAggregates,
    SessionTotals, Totals, Usage,
};
use crate::omarchy::{self, LimitsReport};
use crate::pricing::PricingEngine;
use crate::utils::format_clock;

use super::aggregate::{
    burn_rate, coverage, daily_totals, project_totals, session_totals, UNATTRIBUTED,
};

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
    /// Every source path and billing setting, for the collector-less refresh path.
    pub roots: SourceRoots,
    /// Whether the absence of Omarchy's records has been logged. Once is information; every
    /// thirty seconds is noise.
    pub(super) limits_absence_logged: bool,
    pub provider_filter: Option<String>,
    pub model_filter: Option<String>,
    pub collector: Option<CollectorHandle>,
    /// Which view occupies the right-hand pane. This was two independent booleans, so
    /// "budgets on" and "routing on" could both be true and one silently won.
    pub panel: Panel,
    /// The project the sessions view is scoped to, when the user drilled in from Projects.
    ///
    /// The first piece of *navigational* state any panel has had: every other panel answers
    /// "show me X" and is stateless, while this one answers "show me X, inside Y". Kept on the
    /// App rather than in the panel so `recompute` can narrow the session list once per refresh
    /// instead of the draw call filtering on every frame.
    pub(super) drilldown: Option<Drilldown>,
    /// The `/` row filter. See [`Search`].
    pub(super) search: Search,
    /// Sort per panel, defaulting to the order each list has always had.
    pub(super) sorts: std::collections::HashMap<Panel, Sort>,
    /// Whether the key reference is open. There are more bindings than fit on one footer line,
    /// and a truncated footer hides them without saying so.
    pub show_help: bool,
    pub budget_engine: BudgetEngine,
    /// Held to rank models by list price when deriving escalations. Loaded once: it is
    /// immutable data, and re-reading the table per refresh would put file I/O behind the
    /// render loop.
    pub(super) pricing: PricingEngine,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Panel {
    #[default]
    Models,
    Budgets,
    Routing,
    Projects,
    TimeSeries,
    Burn,
    Sessions,
    Limits,
}

/// Order two costs, where `None` means "not known" rather than zero.
///
/// The direction is applied *here* rather than by reversing the sorted list, because an unknown
/// is not a point on the scale and reversing must not move it. Let it move and ascending presents
/// the unknowns as the cheapest rows while descending presents them as the most expensive; the
/// data supports neither claim. They cluster at the end of whichever order is in force instead,
/// which is what keeps "unknown cost stays unknown" true of the ordering and not just of the cell.
fn cost_order(a: Option<f64>, b: Option<f64>, descending: bool) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            let order = a.total_cmp(&b);
            if descending {
                order.reverse()
            } else {
                order
            }
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

/// The cost a project or session rollup sorts by, or `None` where its panel shows no figure.
///
/// Dollars survive partial coverage: the cell reads `≥ $x.xx` and that floor is a real number,
/// incomplete but not invented. A rollup with no dollars *and* quota- or unpriced-billed requests
/// behind it renders as `quota` or `unpriced`, and sorting it as `0.0` would file it below every
/// genuinely free row while its own cell said the cost was unknown.
///
/// This is not a corner case. `accrue` returns before touching `cost` for quota-billed usage, so
/// on a subscription every project can be `0.0` at once — a plain numeric sort makes the whole
/// panel a single tie group and orders it by nothing at all.
fn rollup_cost(cost: f64, unpriced_requests: u64, quota_requests: u64) -> Option<f64> {
    if cost == 0.0 && (unpriced_requests > 0 || quota_requests > 0) {
        None
    } else {
        Some(cost)
    }
}

/// Sort by `order`, applying the direction inside the comparator.
///
/// Deliberately not `sort_by(..)` followed by `reverse()`. Rust's sorts are stable, so reversing
/// the sorted vector also reverses every group of equal rows, which silently inverts whatever
/// tiebreak the aggregator already applied. `project_totals` documents "sorted by cost, then
/// tokens"; reversing turned equal-cost projects into tokens-ascending, and on a subscription —
/// where every project's cost is `0.0` — that inverted the entire panel rather than a stray pair.
fn sort_rows<T, F>(rows: &mut [T], descending: bool, mut order: F)
where
    F: FnMut(&T, &T) -> std::cmp::Ordering,
{
    if descending {
        rows.sort_by(|a, b| order(a, b).reverse());
    } else {
        rows.sort_by(|a, b| order(a, b));
    }
}

/// Sort by a cost key, which `cost_order` places itself.
///
/// Takes a key rather than a comparator so that a caller cannot route a cost column through
/// `sort_rows` and reverse the direction twice, undoing the placement `cost_order` exists to make.
fn sort_rows_by_cost<T, F>(rows: &mut [T], descending: bool, mut key: F)
where
    F: FnMut(&T) -> Option<f64>,
{
    rows.sort_by(|a, b| cost_order(key(a), key(b), descending));
}

/// Which column a panel is sorted by, and which way.
///
/// Per panel, not global: the four tabular panels have different columns, and carrying a column
/// index across a panel switch would mean sorting by whatever happened to be at that index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub column: usize,
    pub descending: bool,
}

/// The sortable columns of a panel, in the order they are rendered.
///
/// Kept beside the panel's own sort function rather than in one table, because the comparators
/// operate on four different row types. The names here must match the rendered headers — the
/// marker that shows which column is sorted is drawn from the same index.
impl Panel {
    pub fn sort_columns(self) -> &'static [&'static str] {
        match self {
            Panel::Models => &["PROVIDER / MODEL", "CLASS", "TOKENS", "COST", "REQS"],
            Panel::Projects => &["PROJECT", "TOKENS", "COST", "REQS", "SESS"],
            Panel::Sessions => &[
                "STARTED", "RAN", "PROJECT", "MODEL", "TOKENS", "COST", "REQS",
            ],
            Panel::Routing => &[
                "AGENT",
                "MODEL",
                "$/SUCCESS",
                "PASS",
                "RETRY",
                "ESC",
                "DEFECT",
                "TOKENS",
                "TASKS",
            ],
            // Budgets, the time series, the burn projection and the limits panel are not tables
            // of comparable rows; there is nothing to sort them by.
            _ => &[],
        }
    }

    /// The column each panel sorts by until the user says otherwise, and its direction.
    ///
    /// These reproduce the orders the lists have always had, so nothing moves until a key is
    /// pressed: models and routing by tokens, projects by cost, sessions by most recent.
    #[cfg(test)]
    pub(crate) fn default_sort_for_test(self) -> Sort {
        self.default_sort()
    }

    fn default_sort(self) -> Sort {
        match self {
            Panel::Models => Sort {
                column: 2,
                descending: true,
            },
            Panel::Projects => Sort {
                column: 2,
                descending: true,
            },
            Panel::Sessions => Sort {
                column: 0,
                descending: true,
            },
            // `$/SUCCESS` ascending: the order the routing panel already showed, which it
            // produced by re-sorting inside its draw call. That ordering *is* the answer the
            // panel exists to give, so it is the default rather than the aggregate's token order.
            Panel::Routing => Sort {
                column: 2,
                descending: false,
            },
            _ => Sort {
                column: 0,
                descending: true,
            },
        }
    }
}

/// The row filter typed with `/`.
///
/// Filters what the visible panel *lists*; it never narrows the data the totals are computed
/// from. Those are two different questions — "which rows am I looking at" and "what did I
/// spend" — and answering the second with the first is how a header and a panel end up
/// disagreeing about the same range. The footer says how many rows of how many are showing, so
/// no count is ever presented without its scope.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Search {
    pub query: String,
    /// While true, every printable key appends here instead of acting on the dashboard.
    pub typing: bool,
}

impl Search {
    fn matches(&self, haystack: &str) -> bool {
        self.query.is_empty()
            || haystack
                .to_ascii_lowercase()
                .contains(&self.query.to_ascii_lowercase())
    }

    fn is_filtering(&self) -> bool {
        !self.query.is_empty()
    }
}

/// Row counts before the `/` filter, so the footer can say "3 of 12" rather than just "3".
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct SearchTotals {
    pub rows: usize,
    pub projects: usize,
    pub sessions: usize,
    pub routing: usize,
}

/// Where the sessions view is scoped, and where to return to.
#[derive(Clone, Debug, PartialEq)]
pub struct Drilldown {
    /// The raw project value, as `Usage::project` records it. `project_totals` keys on the same
    /// value — the display labels are applied at render time — so this matches directly.
    pub project: String,
    /// The Projects row the user came from, so leaving lands where they left rather than at the
    /// top of a list they have to find their place in again.
    ///
    /// A hint, not an address: the row is looked up by project name on the way back and this is
    /// only the fallback. Sorting, a `/` filter, or a refresh that adds a project can all move a
    /// project to a different index while the user is inside it, and returning them to whatever
    /// now sits at the old number would be worse than returning them to the top.
    pub from_row: usize,
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
    escalations: Escalations,
    limits: LimitsReport,
    search_totals: SearchTotals,
}

/// How much of the visible usage the pricing engine could actually price.
///
/// Provenance is the project's differentiator but it lived entirely in an internal enum: a
/// user could read a total without knowing it covered two thirds of their requests.
#[derive(Clone, Copy, Debug, Default)]
pub struct Coverage {
    pub priced_requests: u64,
    pub billable_requests: u64,
    /// Requests billed against a plan quota, kept out of both sides of the ratio because there
    /// is no per-request price for them to be missing. Reported alongside the percentage so the
    /// volume cannot silently disappear from the figure.
    pub quota_requests: u64,
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
        roots: SourceRoots,
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
            roots,
            limits_absence_logged: false,
            provider_filter,
            model_filter,
            collector,
            panel: Panel::Models,
            budget_engine,
            drilldown: None,
            search: Search::default(),
            sorts: std::collections::HashMap::new(),
            show_help: false,
            pricing: PricingEngine::load(),
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
        // Narrowed here, not in the draw call: the dashboard redraws several times a second and
        // nothing on the render path may compute.
        if let Some(drilldown) = &self.drilldown {
            let wanted = drilldown.project.as_str();
            self.view.sessions.retain(|session| match &session.project {
                Some(project) => project == wanted,
                // `project_totals` files usage with no project under this label, so a drilldown
                // into it must match the sessions that have none.
                None => wanted == UNATTRIBUTED,
            });
        }
        // Computed here, once per refresh, because `burn_rate` needs the clock and the render
        // path must not read it.
        self.view.burn = burn_rate(&self.usages, BURN_WINDOW_SECS, crate::utils::now());
        self.view.coverage = coverage(&self.view.filtered);
        // Derived from usage already collected, never merged into recorded routing events.
        let escalations =
            escalation::derive(&self.view.filtered, |model| self.pricing.input_rate(model));
        self.view.escalations = escalations;

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
            if let Some(equivalent) = u.api_equivalent_cost {
                entry.api_equivalent_cost =
                    Some(entry.api_equivalent_cost.unwrap_or(0.0) + equivalent);
            }
        }
        self.view.rows = grouped.into_values().collect();
        self.view.rows.sort_by_key(|u| Reverse(u.total_tokens()));

        self.apply_sorts();
        self.apply_search();

        // Clamped against the panel that is actually showing, not the model table. This read
        // `self.view.rows.len()` unconditionally, so on a machine with three model groups and
        // ten projects the cursor could never reach the fourth project — and once Enter acts on
        // the row under it, a cursor clamped to the wrong list is not just cosmetic.
        self.selected = self.selected.min(self.visible_rows().saturating_sub(1));
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
            // Bounded by the project list, not the model table. Without this the cursor can sit
            // past the last project — harmless while nothing acted on the row, and wrong the
            // moment Enter drills into whatever is under it.
            Panel::Projects => self.view.projects.len(),
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

    /// Order each panel's list by its current sort.
    ///
    /// Runs before `apply_search`, which uses `retain` and so preserves whatever order this
    /// produced. Floats are compared with `total_cmp`: a `partial_cmp().unwrap()` panics on a
    /// NaN, and while no rate should produce one, a dashboard is the wrong place to find out.
    fn apply_sorts(&mut self) {
        let sort = self.sort_for(Panel::Models);
        let rows = &mut self.view.rows;
        match sort.column {
            0 => sort_rows(rows, sort.descending, |a, b| {
                (a.provider.to_lowercase(), a.model.to_lowercase())
                    .cmp(&(b.provider.to_lowercase(), b.model.to_lowercase()))
            }),
            1 => sort_rows(rows, sort.descending, |a, b| {
                a.category.label().cmp(b.category.label())
            }),
            // By what the COST cell shows, not by `cost`: a quota-billed row reads `ON QUOTA`
            // whatever number happens to sit in the field beside it.
            3 => sort_rows_by_cost(rows, sort.descending, crate::ui::theme::cost_sort_key),
            4 => sort_rows(rows, sort.descending, |a, b| a.requests.cmp(&b.requests)),
            _ => sort_rows(rows, sort.descending, |a, b| {
                a.total_tokens().cmp(&b.total_tokens())
            }),
        }

        let sort = self.sort_for(Panel::Projects);
        let projects = &mut self.view.projects;
        match sort.column {
            0 => sort_rows(projects, sort.descending, |a, b| {
                a.project.to_lowercase().cmp(&b.project.to_lowercase())
            }),
            2 => sort_rows_by_cost(projects, sort.descending, |p| {
                rollup_cost(p.cost, p.unpriced_requests, p.quota_requests)
            }),
            3 => sort_rows(projects, sort.descending, |a, b| {
                a.requests.cmp(&b.requests)
            }),
            4 => sort_rows(projects, sort.descending, |a, b| {
                a.sessions.cmp(&b.sessions)
            }),
            _ => sort_rows(projects, sort.descending, |a, b| a.tokens.cmp(&b.tokens)),
        }

        let sort = self.sort_for(Panel::Sessions);
        let sessions = &mut self.view.sessions;
        match sort.column {
            1 => sort_rows(sessions, sort.descending, |a, b| {
                a.duration_secs().cmp(&b.duration_secs())
            }),
            2 => sort_rows(sessions, sort.descending, |a, b| {
                a.project
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .cmp(&b.project.as_deref().unwrap_or_default().to_lowercase())
            }),
            3 => sort_rows(sessions, sort.descending, |a, b| {
                a.models.len().cmp(&b.models.len())
            }),
            4 => sort_rows(sessions, sort.descending, |a, b| a.tokens.cmp(&b.tokens)),
            5 => sort_rows_by_cost(sessions, sort.descending, |s| {
                rollup_cost(s.cost, s.unpriced_requests, s.quota_requests)
            }),
            6 => sort_rows(sessions, sort.descending, |a, b| {
                a.requests.cmp(&b.requests)
            }),
            // STARTED shows `first_seen`, so that is what sorting by STARTED does. The list
            // previously ordered by `last_seen` while displaying `first_seen`, which meant the
            // column a reader saw was not the column the rows were in. Marking a column and
            // sorting by a different one would have kept that, silently.
            _ => sort_rows(sessions, sort.descending, |a, b| {
                a.first_seen.cmp(&b.first_seen)
            }),
        }

        let sort = self.sort_for(Panel::Routing);
        let routing = &mut self.view.routing;
        match sort.column {
            0 => sort_rows(routing, sort.descending, |a, b| {
                a.agent.to_lowercase().cmp(&b.agent.to_lowercase())
            }),
            1 => sort_rows(routing, sort.descending, |a, b| {
                a.model.to_lowercase().cmp(&b.model.to_lowercase())
            }),
            // Cheapest per delivered result. Agents with nothing passing have no figure at all,
            // and `cost_order` holds them at the end in both directions rather than letting them
            // appear free -- which is why this is not `a.cost`.
            2 => sort_rows_by_cost(routing, sort.descending, crate::routing::cost_per_success),
            3 => sort_rows(routing, sort.descending, |a, b| {
                a.test_passes.cmp(&b.test_passes)
            }),
            4 => sort_rows(routing, sort.descending, |a, b| a.retries.cmp(&b.retries)),
            5 => sort_rows(routing, sort.descending, |a, b| {
                a.escalations.cmp(&b.escalations)
            }),
            6 => sort_rows(routing, sort.descending, |a, b| {
                a.review_defects.cmp(&b.review_defects)
            }),
            8 => sort_rows(routing, sort.descending, |a, b| a.tasks.cmp(&b.tasks)),
            _ => sort_rows(routing, sort.descending, |a, b| a.tokens.cmp(&b.tokens)),
        }
    }

    /// The sort in force for a panel, defaulting to the order that list has always had.
    pub fn sort_for(&self, panel: Panel) -> Sort {
        self.sorts
            .get(&panel)
            .copied()
            .unwrap_or_else(|| panel.default_sort())
    }

    /// Move the sort to the next or previous column of the visible panel.
    ///
    /// A panel with nothing to sort ignores it rather than storing a column it cannot use.
    pub fn cycle_sort_column(&mut self, forward: bool) {
        let columns = self.panel.sort_columns().len();
        if columns == 0 {
            return;
        }
        let current = self.sort_for(self.panel);
        let column = if forward {
            (current.column + 1) % columns
        } else {
            (current.column + columns - 1) % columns
        };
        self.sorts.insert(self.panel, Sort { column, ..current });
        self.recompute();
    }

    /// Reverse the visible panel's sort.
    pub fn reverse_sort(&mut self) {
        if self.panel.sort_columns().is_empty() {
            return;
        }
        let current = self.sort_for(self.panel);
        self.sorts.insert(
            self.panel,
            Sort {
                descending: !current.descending,
                ..current
            },
        );
        self.recompute();
    }

    /// The marker drawn beside a column header, when that column is the one being sorted.
    pub fn sort_marker(&self, panel: Panel, column: usize) -> &'static str {
        let sort = self.sort_for(panel);
        if sort.column != column || panel.sort_columns().is_empty() {
            return "";
        }
        if sort.descending {
            " v"
        } else {
            " ^"
        }
    }

    /// Narrow what each panel lists to the `/` query, recording what each had before.
    ///
    /// Applied to the display lists only. `view.filtered` — which every total, the coverage
    /// figure and the escalations are computed from — is deliberately untouched: `/` answers
    /// "which rows am I looking at", not "what did I spend".
    fn apply_search(&mut self) {
        let totals = SearchTotals {
            rows: self.view.rows.len(),
            projects: self.view.projects.len(),
            sessions: self.view.sessions.len(),
            routing: self.view.routing.len(),
        };
        self.view.search_totals = totals;
        if !self.search.is_filtering() {
            return;
        }

        let search = self.search.clone();
        self.view
            .rows
            .retain(|row| search.matches(&format!("{} {}", row.provider, row.model)));
        self.view
            .projects
            .retain(|project| search.matches(&project.project));
        self.view.sessions.retain(|session| {
            search.matches(&session.session_id)
                || session
                    .project
                    .as_deref()
                    .is_some_and(|p| search.matches(p))
                || session.models.iter().any(|model| search.matches(model))
        });
        self.view.routing.retain(|aggregate| {
            search.matches(&aggregate.agent)
                || search.matches(&aggregate.model)
                || search.matches(&aggregate.provider)
        });

        // The cursor can easily be past the end of a list that just got shorter.
        self.selected = self.selected.min(self.visible_rows().saturating_sub(1));
    }

    /// Start typing a `/` filter.
    pub fn begin_search(&mut self) {
        self.search.typing = true;
    }

    /// Feed a keystroke to the filter. Returns false when the key was not consumed.
    pub fn search_key(&mut self, key: char) -> bool {
        if !self.search.typing {
            return false;
        }
        self.search.query.push(key);
        self.recompute();
        true
    }

    /// Remove the last character, or leave the filter when it is already empty.
    pub fn search_backspace(&mut self) {
        if self.search.query.pop().is_none() {
            self.search.typing = false;
        }
        self.recompute();
    }

    /// Keep the filter but stop capturing keys, so the dashboard's bindings work again.
    pub fn accept_search(&mut self) {
        self.search.typing = false;
    }

    /// Abandon the filter entirely.
    pub fn cancel_search(&mut self) {
        self.search = Search::default();
        self.recompute();
    }

    pub fn is_typing_search(&self) -> bool {
        self.search.typing
    }

    /// The filter and how much it is hiding, for the footer. `None` when nothing is filtered.
    pub fn search_status(&self) -> Option<(&str, usize, usize)> {
        if !self.search.typing && !self.search.is_filtering() {
            return None;
        }
        let totals = self.view.search_totals;
        let total = match self.panel {
            Panel::Projects => totals.projects,
            Panel::Sessions => totals.sessions,
            Panel::Routing => totals.routing,
            _ => totals.rows,
        };
        Some((self.search.query.as_str(), self.visible_rows(), total))
    }

    /// Scope the sessions view to the project under the cursor, if the cursor is on one.
    ///
    /// Returns whether it drilled: the caller uses that to decide whether the key did anything,
    /// and pressing Enter on a panel that has nothing to drill into must not change the view.
    pub fn drill_into_selected_project(&mut self) -> bool {
        if self.panel != Panel::Projects {
            return false;
        }
        let Some(project) = self.view.projects.get(self.selected) else {
            return false;
        };
        self.drilldown = Some(Drilldown {
            project: project.project.clone(),
            from_row: self.selected,
        });
        self.panel = Panel::Sessions;
        self.selected = 0;
        self.recompute();
        true
    }

    /// Leave a drilldown, returning to the Projects row it started from.
    ///
    /// Returns whether it was in one — `Esc` quits the dashboard unless it has somewhere to go
    /// back to, so the caller needs to know which happened.
    pub fn leave_drilldown(&mut self) -> bool {
        let Some(drilldown) = self.drilldown.take() else {
            return false;
        };
        self.panel = Panel::Projects;
        self.recompute();
        // By name first. The index is only right if nothing reordered the list while the user
        // was inside the project -- and sorting, filtering and a refresh can all do that.
        self.selected = self
            .view
            .projects
            .iter()
            .position(|project| project.project == drilldown.project)
            .unwrap_or(drilldown.from_row)
            .min(self.visible_rows().saturating_sub(1));
        true
    }

    /// The project the sessions view is scoped to, if any.
    pub fn drilldown_project(&self) -> Option<&str> {
        self.drilldown.as_ref().map(|d| d.project.as_str())
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
            match crate::collector::load_usage(&self.roots) {
                Ok((usages, source)) => {
                    self.usages = usages;
                    self.status = source;
                    self.degraded = false;
                }
                Err(error) => {
                    // Not "OpenCode unavailable": `load_usage` also propagates the journal read,
                    // and labelling every failure with one source sent readers to the wrong file.
                    self.usages.clear();
                    self.status = format!("sources unavailable: {}", error);
                    self.degraded = true;
                }
            }
        }
        self.last_refresh = format_clock();
        self.refreshed_at = Instant::now();
        self.recompute();
        // Routing lives in SQLite; read it here, not inside `draw`.
        self.view.routing = match crate::collector::journal::load_routing(&self.roots.journal) {
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
        // Omarchy's records: three small files, read here beside the routing table so the
        // render path stays free of I/O. Absent on any machine without Omarchy.
        if self.roots.limits_enabled {
            if let Some(dir) = self.roots.omarchy_usage_dir() {
                let report =
                    omarchy::load_limits(&dir, crate::utils::now(), omarchy::STALE_AFTER_SECS);
                if !report.present && !self.limits_absence_logged {
                    crate::logging::info(
                        "omarchy",
                        &format!("no usage records at {}; limits panel idle", dir.display()),
                    );
                    self.limits_absence_logged = true;
                }
                if !report.problems.is_empty() {
                    self.status =
                        format!("{} | limits: {}", self.status, report.problems.join("; "));
                    self.degraded = true;
                }
                self.view.limits = report;
            }
        }
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
    /// Inject routing aggregates and order them the way a refresh would.
    ///
    /// The sort is applied here because it is applied in `recompute`, and this helper exists to
    /// stand in for one. Without it a test would see the aggregates in whatever order it passed
    /// them — which is exactly what `routing_leads_with_cost_per_delivered_result` is written to
    /// catch, and it duly caught it when the ordering moved out of the panel's draw call.
    #[cfg(test)]
    pub(super) fn set_routing_for_test(&mut self, routing: Vec<RoutingAggregates>) {
        self.view.routing = routing;
        self.apply_sorts();
    }

    pub fn escalations(&self) -> &Escalations {
        &self.view.escalations
    }

    pub fn limits(&self) -> &LimitsReport {
        &self.view.limits
    }

    #[cfg(test)]
    pub(super) fn set_limits_for_test(&mut self, limits: LimitsReport) {
        self.view.limits = limits;
    }

    #[cfg(test)]
    pub(super) fn set_escalations_for_test(&mut self, escalations: Escalations) {
        self.view.escalations = escalations;
    }

    pub fn routing(&self) -> &[RoutingAggregates] {
        &self.view.routing
    }
}
