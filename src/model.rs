use ratatui::style::Color;

use crate::utils::now;

pub const CYAN: Color = Color::Rgb(69, 211, 255);
pub const GREEN: Color = Color::Rgb(116, 235, 152);
pub const YELLOW: Color = Color::Rgb(255, 205, 92);
pub const RED: Color = Color::Rgb(255, 105, 105);
/// The CLOUD category's purple, promoted from an inline literal so quota-billed cost cells can
/// match the category tile a reader sees next to them.
pub const CLOUD: Color = Color::Rgb(194, 137, 255);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Local,
    Free,
    Paid,
    Cloud,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum CostStatus {
    ProviderReported,
    Calculated,
    Estimated,
    Free,
    Local,
    /// Billed, but not per token at any rate this tool can know — an account quota, a
    /// subscription tier, GPU time. Ollama Cloud is the current instance.
    ///
    /// Distinct from `Unavailable`, which means "this should carry a price and does not".
    /// Collapsing the two made a deliberate, correct refusal to invent a number read as a
    /// failure to produce one, in every panel that reports pricing coverage.
    Quota,
    #[default]
    Unavailable,
}

impl CostStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::ProviderReported => "reported",
            Self::Calculated => "calculated",
            Self::Estimated => "estimated",
            Self::Free => "free",
            Self::Local => "local",
            Self::Quota => "quota",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn is_billable(self) -> bool {
        matches!(
            self,
            Self::ProviderReported | Self::Calculated | Self::Estimated
        )
    }

    /// Whether this usage ought to carry a price and would be a gap without one.
    ///
    /// Distinct from `is_billable`, which means "has a known cost worth summing" and so
    /// excludes `Unavailable` — the very case a coverage figure exists to count.
    ///
    /// `Free` and `Local` are genuinely costless. `Quota` is not costless, but no per-request
    /// price exists to be missing, so counting it as a gap misreports a deliberate refusal as a
    /// failure. Callers that exclude it here must count it somewhere: a row whose work is
    /// entirely quota-billed has no unpriced requests and no dollars, and would otherwise render
    /// as `$0.00`.
    pub fn needs_price(self) -> bool {
        !matches!(self, Self::Free | Self::Local | Self::Quota)
    }

    /// Whether this usage is billed against a plan rather than per token.
    ///
    /// Exists so aggregates and panels can count quota volume without naming the variant.
    pub fn is_quota_billed(self) -> bool {
        self == Self::Quota
    }

    pub fn is_known(self) -> bool {
        self != Self::Unavailable
    }
}

/// Fold one usage row into a rollup's cost fields.
///
/// Four rollups needed exactly this and carried four copies of it. Adding the quota case to a
/// copy-pasted block is the shape of change that reliably gets applied to three places out of
/// four, so there is now one. The budget engine was the fifth copy, written before the counters
/// existed: it summed the priced rows and dropped the rest, so a budget over unpriced or
/// quota-billed work read as untouched. It lives here rather than in the UI's aggregation
/// module because the budget engine is not UI, and the types it folds are these.
pub fn accrue(usage: &Usage, cost: &mut f64, unpriced: &mut u64, quota: &mut u64) {
    if usage.cost_status.is_quota_billed() {
        *quota += usage.requests;
        return;
    }
    if usage.cost_status.needs_price() {
        match usage.cost.filter(|_| usage.cost_status.is_billable()) {
            Some(value) => *cost += value,
            None => *unpriced += usage.requests,
        }
    }
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "LOCAL",
            Self::Free => "FREE",
            Self::Paid => "PAID",
            Self::Cloud => "CLOUD",
            Self::Unknown => "UNKNOWN",
        }
    }
    pub fn color(self) -> Color {
        match self {
            Self::Local => GREEN,
            Self::Free => CYAN,
            Self::Paid => YELLOW,
            Self::Cloud => CLOUD,
            Self::Unknown => RED,
        }
    }
}

/// How a request is paid for: per token, or against a plan the user already pays for.
///
/// Claude Code on a Pro or Max plan and Codex on a ChatGPT plan write the same token counts
/// as an API-key session, and nothing on the row says which. Priced at list rates, a
/// subscription's usage read as hundreds of dollars of spend that were never charged, and
/// tripped budgets on money that did not exist. The collector decides once per source (see
/// `collector::billing`) and stamps every row; pricing then keeps the list-rate figure as a
/// labelled counterfactual rather than as cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Billing {
    #[default]
    PerToken,
    Subscription,
}

impl Billing {
    pub fn label(self) -> &'static str {
        match self {
            Self::PerToken => "per_token",
            Self::Subscription => "subscription",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Usage {
    /// Stable per-event identity from the source (OpenCode message id, journal `event_id`).
    /// Used for deduplication; `None` falls back to shape-plus-timestamp matching.
    pub event_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub category: Category,
    pub requests: u64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: Option<f64>,
    pub cost_status: CostStatus,
    /// Whether this request was billed per token or against a subscription. Set by the
    /// collector; consulted by pricing, which turns subscription rows into `Quota`.
    pub billing: Billing,
    /// What the request would have cost at API list rates. Only set on subscription rows,
    /// where `cost` is deliberately `None`: it is a counterfactual, never money that changed
    /// hands, and it is never summed into a dollar total.
    pub api_equivalent_cost: Option<f64>,
    pub created: i64,
    /// Conversation/session this usage belongs to, when the source records one.
    pub session_id: Option<String>,
    /// Project the work happened in — for Claude Code, the repository working directory.
    /// Enables per-project cost, which no view could express before.
    pub project: Option<String>,
}

impl Usage {
    pub fn total_tokens(&self) -> u64 {
        self.input + self.output + self.reasoning + self.cache_read + self.cache_write
    }
}

/// Usage rolled up by local calendar day, for the time-series view.
///
/// The day boundary is *local*, matching `Range::Today` and the budget engine. A UTC bucket
/// would put a user's evening work on the wrong bar and disagree with the TODAY total shown
/// two panels away.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DayTotals {
    /// Local calendar day, as `YYYY-MM-DD`.
    pub day: String,
    pub requests: u64,
    pub tokens: u64,
    pub cost: f64,
    /// Requests that should carry a price but do not. A day that is only partly priced must
    /// not render its cost as if it were complete.
    pub unpriced_requests: u64,
    /// Requests billed against a plan quota rather than per token. They carry real cost that no
    /// API exposes per request, so they are counted separately: without this, a row whose work is
    /// entirely quota-billed has no unpriced requests and no dollars, and renders as `$0.00`.
    pub quota_requests: u64,
}
/// Usage rolled up by session.
///
/// A session id is a bare UUID and tells a reader nothing, so everything here exists to make a
/// row identifiable without it: when it ran, where, and on what.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SessionTotals {
    pub session_id: String,
    /// Working directory the session ran in, when the source records one.
    pub project: Option<String>,
    /// Unix timestamps of the first and last request seen in this session.
    pub first_seen: i64,
    pub last_seen: i64,
    pub requests: u64,
    pub tokens: u64,
    pub cost: f64,
    pub unpriced_requests: u64,
    /// Requests billed against a plan quota rather than per token. They carry real cost that no
    /// API exposes per request, so they are counted separately: without this, a row whose work is
    /// entirely quota-billed has no unpriced requests and no dollars, and renders as `$0.00`.
    pub quota_requests: u64,
    /// Distinct `provider/model` pairs used.
    pub models: Vec<String>,
}

impl SessionTotals {
    /// How long the session ran, in seconds. Zero for a single-request session.
    pub fn duration_secs(&self) -> i64 {
        (self.last_seen - self.first_seen).max(0)
    }
}

/// How fast usage is being consumed over a trailing window.
///
/// Deliberately carries the evidence alongside the rate. A burn rate computed from three
/// requests is noise, and one computed over partly-unpriced usage is a floor rather than a
/// figure — a consumer that cannot see either would present both as confident numbers.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct BurnRate {
    /// Length of the trailing window, in seconds.
    pub window_secs: i64,
    pub requests: u64,
    pub tokens: u64,
    pub cost: f64,
    /// Requests in the window that should carry a price but do not.
    pub unpriced_requests: u64,
    /// Requests billed against a plan quota rather than per token. They carry real cost that no
    /// API exposes per request, so they are counted separately: without this, a row whose work is
    /// entirely quota-billed has no unpriced requests and no dollars, and renders as `$0.00`.
    pub quota_requests: u64,
}

impl BurnRate {
    /// Fewer than this many requests in the window is too thin to extrapolate from.
    pub const MIN_SAMPLE: u64 = 5;

    pub fn tokens_per_minute(&self) -> f64 {
        if self.window_secs <= 0 {
            return 0.0;
        }
        self.tokens as f64 / (self.window_secs as f64 / 60.0)
    }

    pub fn cost_per_hour(&self) -> f64 {
        if self.window_secs <= 0 {
            return 0.0;
        }
        self.cost / (self.window_secs as f64 / 3600.0)
    }

    /// Whether there is enough in the window to project from.
    ///
    /// Three requests in an hour does not support "you will hit your budget at 4pm". Printing a
    /// confident figure from too little evidence is the same failure as rendering unknown cost
    /// as `$0.00`, wearing a different hat.
    pub fn is_projectable(&self) -> bool {
        self.requests >= Self::MIN_SAMPLE && self.cost > 0.0
    }

    /// Whether some of the window's usage has no price, making `cost` a floor.
    pub fn is_partial(&self) -> bool {
        self.unpriced_requests > 0
    }

    /// Whether the window contains only quota-billed work, so there is no rate to report.
    pub fn is_quota_only(&self) -> bool {
        self.quota_requests > 0 && self.cost == 0.0 && self.unpriced_requests == 0
    }
}
/// Usage rolled up by project, for the per-project cost view.
#[derive(Debug, Default, Clone)]
pub struct ProjectTotals {
    pub project: String,
    pub requests: u64,
    pub tokens: u64,
    pub cost: f64,
    /// Requests whose cost is billable but unknown. Rendering `$12.00` next to work that is
    /// only two-thirds priced would misstate the number without saying so.
    pub unpriced_requests: u64,
    /// Requests billed against a plan quota rather than per token. They carry real cost that no
    /// API exposes per request, so they are counted separately: without this, a row whose work is
    /// entirely quota-billed has no unpriced requests and no dollars, and renders as `$0.00`.
    pub quota_requests: u64,
    pub sessions: usize,
    pub models: usize,
}

#[derive(Default)]
pub struct Totals {
    pub requests: u64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
    pub unknown_requests: u64,
    /// Requests billed against a plan quota rather than per token. They carry real cost that no
    /// API exposes per request, so they are counted separately: without this, a row whose work is
    /// entirely quota-billed has no unpriced requests and no dollars, and renders as `$0.00`.
    pub quota_requests: u64,
    /// What the subscription-billed requests would have cost at API list rates. Kept apart
    /// from `cost` because it was never charged; shown only as a labelled counterfactual.
    pub api_equivalent: f64,
}

impl Totals {
    pub fn add(&mut self, usage: &Usage) {
        self.requests += usage.requests;
        self.input += usage.input;
        self.output += usage.output;
        self.reasoning += usage.reasoning;
        self.cache_read += usage.cache_read;
        self.cache_write += usage.cache_write;
        if !usage.cost_status.is_known() {
            self.unknown_requests += usage.requests;
        }
        if usage.cost_status.is_quota_billed() {
            self.quota_requests += usage.requests;
        }
        if usage.cost_status.is_billable() {
            if let Some(cost) = usage.cost {
                self.cost += cost;
            }
        }
        if let Some(equivalent) = usage.api_equivalent_cost {
            self.api_equivalent += equivalent;
        }
    }
    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.reasoning + self.cache_read + self.cache_write
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Range {
    Today,
    Week,
    Month,
    Days(u64),
    All,
}

impl Range {
    pub fn label(self) -> String {
        match self {
            Self::Today => "TODAY".to_string(),
            Self::Week => "7 DAYS".to_string(),
            Self::Month => "30 DAYS".to_string(),
            Self::Days(days) => format!("{} DAYS", days),
            Self::All => "ALL TIME".to_string(),
        }
    }
    pub fn cutoff(self) -> i64 {
        let seconds: i64 = match self {
            Self::All => return 0,
            // "TODAY" means the local calendar day, matching how budgets bill and how users
            // read the label. A rolling 24h window silently disagrees with both.
            Self::Today => return crate::utils::local_day_start(),
            Self::Week => 604_800,
            Self::Month => 2_592_000,
            Self::Days(days) => days.saturating_mul(86_400).min(i64::MAX as u64) as i64,
        };
        now().saturating_sub(seconds)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RoutingEvent {
    pub task: String,
    pub phase: String,
    pub agent: String,
    pub model: String,
    pub provider: String,
    pub category: Category,
    pub requests: u64,
    pub tokens: u64,
    pub cost: Option<f64>,
    pub cost_status: CostStatus,
    pub retries: u32,
    pub escalations: u32,
    pub test_result: Option<bool>,
    pub review_defects: u32,
    pub created: i64,
}

#[derive(Clone, Debug, Default)]
pub struct RoutingAggregates {
    pub agent: String,
    pub model: String,
    pub provider: String,
    pub tasks: u64,
    pub tokens: u64,
    /// Spend on the tasks that carried a price. **A floor, not a total** — read it with the three
    /// counters below, exactly as `escalation::Transition::cost_after` is read with
    /// `unpriced_after` and `quota_after`.
    ///
    /// This used to be `event.cost.unwrap_or(0.0)`, which made an unpriced or subscription-billed
    /// model divide to `$0.0000` per success. The routing panel sorts by that figure ascending by
    /// default, so such a model ranked as the cheapest work on the machine and rendered green as
    /// `free`. Convention 1, broken in the panel that carries the project's pitch.
    pub cost: f64,
    /// Tasks whose cost is in `cost`.
    pub priced_tasks: u64,
    /// Tasks that should carry a price and do not, making `cost` a floor.
    pub unpriced_tasks: u64,
    /// Tasks billed against a plan rather than per token. Real spend with no per-request figure;
    /// without this counter an all-subscription agent reads as free.
    pub quota_tasks: u64,
    /// Tasks that genuinely cost nothing — a local model, or one explicitly free.
    pub free_tasks: u64,
    pub retries: u32,
    pub escalations: u32,
    pub test_passes: u32,
    pub test_failures: u32,
    pub review_defects: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_include_all_token_buckets() {
        let u = Usage {
            input: 2,
            output: 3,
            reasoning: 4,
            cache_read: 5,
            cache_write: 6,
            ..Default::default()
        };
        assert_eq!(u.total_tokens(), 20);
    }

    #[test]
    fn quota_billed_usage_never_enters_a_dollar_total() {
        // It has real cost, but no per-request figure to sum. Adding it to `cost` would invent
        // one; counting it as unknown would report it as a gap. It gets its own counter.
        let mut totals = Totals::default();
        totals.add(&Usage {
            requests: 3,
            input: 500,
            cost: None,
            cost_status: CostStatus::Quota,
            ..Default::default()
        });
        assert_eq!(totals.cost, 0.0);
        assert_eq!(totals.unknown_requests, 0, "quota-billed is a known state");
        assert_eq!(totals.quota_requests, 3);
        assert_eq!(totals.tokens(), 500, "its tokens still count");
    }

    #[test]
    fn extreme_day_ranges_are_safe() {
        assert!(Range::Days(u64::MAX).cutoff() <= now());
    }

    #[test]
    fn a_subscription_row_adds_to_quota_and_the_counterfactual_but_never_to_cost() {
        let mut totals = Totals::default();
        totals.add(&Usage {
            requests: 2,
            input: 100,
            cost: None,
            cost_status: CostStatus::Quota,
            billing: Billing::Subscription,
            api_equivalent_cost: Some(1.25),
            ..Default::default()
        });
        assert_eq!(totals.cost, 0.0);
        assert_eq!(totals.quota_requests, 2);
        assert_eq!(totals.unknown_requests, 0);
        assert!((totals.api_equivalent - 1.25).abs() < 1e-9);
    }
}
