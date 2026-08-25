use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::model::{accrue, Usage};

const DEFAULT_WARN_PCT: f64 = 75.0;
const DEFAULT_CRITICAL_PCT: f64 = 90.0;
const ALERT_DEDUP_INTERVAL: Duration = Duration::from_secs(3600);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize)]
pub enum BudgetScopeKind {
    #[default]
    #[serde(rename = "global")]
    Global,
    #[serde(rename = "provider")]
    Provider,
    #[serde(rename = "model")]
    Model,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize)]
pub enum BudgetPeriod {
    #[default]
    #[serde(rename = "daily")]
    Daily,
    #[serde(rename = "monthly")]
    Monthly,
}

impl BudgetPeriod {
    pub fn label(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Monthly => "monthly",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BudgetScope {
    Global,
    Provider(String),
    Model(String),
}

impl BudgetScope {
    pub fn label(&self) -> String {
        match self {
            Self::Global => "global".to_string(),
            Self::Provider(name) => format!("provider:{}", name),
            Self::Model(name) => format!("model:{}", name),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Budget {
    pub scope: BudgetScope,
    pub period: BudgetPeriod,
    pub limit: f64,
    pub warn_pct: f64,
    pub critical_pct: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertLevel {
    Ok,
    Warn,
    Critical,
    Exceeded,
}

impl AlertLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warn => "WARN",
            Self::Critical => "CRITICAL",
            Self::Exceeded => "EXCEEDED",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Alert {
    pub scope: BudgetScope,
    pub period: BudgetPeriod,
    /// Priced spend in the period. A floor when `unpriced_requests` is non-zero: the requests
    /// that could not be priced are real spend this figure does not include.
    pub spend: f64,
    pub limit: f64,
    /// `spend` over `limit`, so a floor whenever `spend` is.
    pub pct: f64,
    /// Derived from `pct`, so a floor too: a budget can be over its limit in truth and `Ok` on
    /// its priced spend. `is_actionable` — and with it the exit code and the webhook — acts on
    /// this floor; the counters below are how a reader learns what it left out.
    pub level: AlertLevel,
    /// Requests in the period that should carry a price and do not.
    pub unpriced_requests: u64,
    /// Requests billed against a plan quota rather than per token. Real cost with no per-request
    /// figure, so never in `spend` — and counted, so a budget over a subscription account does
    /// not read as untouched.
    pub quota_requests: u64,
}

impl Alert {
    pub fn is_actionable(&self) -> bool {
        self.level != AlertLevel::Ok
    }

    /// Whether some of the period's usage has no price, making `spend` a floor.
    pub fn is_partial(&self) -> bool {
        self.unpriced_requests > 0
    }

    /// Whether the period contains only quota-billed work, so there is no figure to report.
    pub fn is_quota_only(&self) -> bool {
        self.quota_requests > 0 && self.spend == 0.0 && self.unpriced_requests == 0
    }

    /// The alert as the webhook and `--check-budgets` report it.
    ///
    /// One shape for both: they were two hand-copied literals, and a field added to one would
    /// not have reached the other. `spend` and `pct` are floors when `unpriced_requests` is
    /// non-zero; a consumer that treats them as exact is reading a number the tool never claimed.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "scope": self.scope.label(),
            "period": self.period.label(),
            "level": self.level.label(),
            "spend": self.spend,
            "limit": self.limit,
            "pct": self.pct,
            "unpriced_requests": self.unpriced_requests,
            "quota_requests": self.quota_requests,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetEntry {
    pub scope: BudgetScopeKind,
    pub name: Option<String>,
    pub period: BudgetPeriod,
    pub limit: f64,
    pub warn: Option<f64>,
    pub critical: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetsConfig {
    pub webhook: Option<String>,
    /// Absent means no budgets, not a malformed config. Configuring only a webhook — to be
    /// notified once budgets are added — is a legitimate state, and now that config parse
    /// errors are fatal rather than silently defaulted, requiring this field would reject it.
    #[serde(default)]
    pub entry: Vec<BudgetEntry>,
}

impl BudgetsConfig {
    /// Refuse an entry that parses but cannot do what it says.
    ///
    /// `deny_unknown_fields` catches a misspelled key; nothing caught a *missing* one. A
    /// provider budget with no `name` became `Provider("")`, which matches nothing, and a
    /// `limit` of zero was permanently `OK` — both sat in the panel looking configured. Same
    /// class as a `billing` line under the wrong collector table, which `ConfigFile::validate`
    /// already refuses.
    pub fn validate(&self) -> anyhow::Result<()> {
        for (index, entry) in self.entry.iter().enumerate() {
            let at = format!("[[budgets.entry]] #{}", index + 1);
            let named = entry.name.as_deref().is_some_and(|n| !n.trim().is_empty());
            match entry.scope {
                BudgetScopeKind::Global if entry.name.is_some() => {
                    return Err(anyhow::anyhow!(
                        "{at}: scope = \"global\" does not take a `name`; it applies to everything"
                    ));
                }
                BudgetScopeKind::Provider | BudgetScopeKind::Model if !named => {
                    let what = match entry.scope {
                        BudgetScopeKind::Provider => "provider",
                        _ => "model",
                    };
                    return Err(anyhow::anyhow!(
                        "{at}: scope = \"{what}\" needs a `name` — the {what} it applies to"
                    ));
                }
                _ => {}
            }
            if !(entry.limit.is_finite() && entry.limit > 0.0) {
                return Err(anyhow::anyhow!(
                    "{at}: `limit` must be a finite number greater than 0, got {}",
                    entry.limit
                ));
            }
            for (key, value) in [("warn", entry.warn), ("critical", entry.critical)] {
                if let Some(pct) = value {
                    if !(pct.is_finite() && pct > 0.0 && pct <= 100.0) {
                        return Err(anyhow::anyhow!(
                            "{at}: `{key}` is a percentage of `limit` and must be above 0 and at \
                             most 100, got {pct}"
                        ));
                    }
                }
            }
            let warn = entry.warn.unwrap_or(DEFAULT_WARN_PCT);
            let critical = entry.critical.unwrap_or(DEFAULT_CRITICAL_PCT);
            if warn >= critical {
                // Say which side was implied: a `warn = 95` with no `critical` fails against a
                // number the user never typed.
                let shown = |value: f64, given: bool| {
                    if given {
                        format!("{value}")
                    } else {
                        format!("{value}, the default")
                    }
                };
                return Err(anyhow::anyhow!(
                    "{at}: `warn` ({}) must be below `critical` ({}), or the warning level can \
                     never be reached",
                    shown(warn, entry.warn.is_some()),
                    shown(critical, entry.critical.is_some())
                ));
            }
        }
        Ok(())
    }
}

pub struct BudgetEngine {
    budgets: Vec<Budget>,
}

impl BudgetEngine {
    pub fn from_config(config: &BudgetsConfig) -> Self {
        let budgets = config
            .entry
            .iter()
            .map(|e| Budget {
                // A config file has been through `BudgetsConfig::validate` by the time it gets
                // here, so a provider or model entry has its name. An entry built by hand
                // without one gets an empty scope, which matches nothing rather than everything.
                scope: match e.scope {
                    BudgetScopeKind::Global => BudgetScope::Global,
                    // Trimmed once here, so the scope the engine matches on is the one
                    // validation approved: `" openai"` passes the emptiness check and would
                    // otherwise never equal a row's `openai`.
                    BudgetScopeKind::Provider => BudgetScope::Provider(trimmed_name(e)),
                    BudgetScopeKind::Model => BudgetScope::Model(trimmed_name(e)),
                },
                period: e.period,
                limit: e.limit,
                warn_pct: e.warn.unwrap_or(DEFAULT_WARN_PCT),
                critical_pct: e.critical.unwrap_or(DEFAULT_CRITICAL_PCT),
            })
            .collect();
        Self { budgets }
    }

    pub fn empty() -> Self {
        Self {
            budgets: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.budgets.is_empty()
    }

    pub fn budgets(&self) -> &[Budget] {
        &self.budgets
    }

    pub fn check(&self, usages: &[Usage]) -> Vec<Alert> {
        self.budgets
            .iter()
            .map(|budget| {
                let ScopeSpend {
                    spend,
                    unpriced_requests,
                    quota_requests,
                } = spend_for_scope(usages, &budget.scope, budget.period);
                let pct = if budget.limit > 0.0 {
                    (spend / budget.limit) * 100.0
                } else {
                    0.0
                };
                let level = if pct >= 100.0 {
                    AlertLevel::Exceeded
                } else if pct >= budget.critical_pct {
                    AlertLevel::Critical
                } else if pct >= budget.warn_pct {
                    AlertLevel::Warn
                } else {
                    AlertLevel::Ok
                };
                Alert {
                    scope: budget.scope.clone(),
                    period: budget.period,
                    spend,
                    limit: budget.limit,
                    pct,
                    level,
                    unpriced_requests,
                    quota_requests,
                }
            })
            .collect()
    }
}

/// The entry's name with the padding a user may have typed inside the quotes removed.
fn trimmed_name(entry: &BudgetEntry) -> String {
    entry
        .name
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn period_cutoff(period: BudgetPeriod) -> i64 {
    // Local calendar boundaries, shared with `Range::Today` so the budget panel and the
    // dashboard's TODAY total can never report different numbers for the same data.
    match period {
        BudgetPeriod::Daily => crate::utils::local_day_start(),
        BudgetPeriod::Monthly => crate::utils::local_month_start(),
    }
}

fn matches_scope(usage: &Usage, scope: &BudgetScope) -> bool {
    match scope {
        BudgetScope::Global => true,
        BudgetScope::Provider(name) => usage.provider.eq_ignore_ascii_case(name),
        BudgetScope::Model(name) => usage.model.eq_ignore_ascii_case(name),
    }
}

/// What a period's usage adds up to, and what that sum is standing on.
#[derive(Default)]
struct ScopeSpend {
    spend: f64,
    unpriced_requests: u64,
    quota_requests: u64,
}

fn spend_for_scope(usages: &[Usage], scope: &BudgetScope, period: BudgetPeriod) -> ScopeSpend {
    let cutoff = period_cutoff(period);
    let mut total = ScopeSpend::default();
    // The same fold every other rollup uses. This one summed `filter_map(|u| u.cost)` on its
    // own, so a row with no price contributed nothing and was counted nowhere — a budget over
    // unpriced or quota-billed work read as untouched, in the one figure the tool acts on.
    for usage in usages
        .iter()
        .filter(|u| u.created >= cutoff)
        .filter(|u| matches_scope(u, scope))
    {
        accrue(
            usage,
            &mut total.spend,
            &mut total.unpriced_requests,
            &mut total.quota_requests,
        );
    }
    total
}

pub struct AlertDispatcher {
    pub webhook_url: Option<String>,
    last_dispatched: HashMap<(BudgetScope, BudgetPeriod), (AlertLevel, Instant)>,
}

impl AlertDispatcher {
    pub fn new(webhook_url: Option<String>) -> Self {
        Self {
            webhook_url,
            last_dispatched: HashMap::new(),
        }
    }

    pub fn should_dispatch(&mut self, alert: &Alert) -> bool {
        if alert.level == AlertLevel::Ok {
            return false;
        }
        let key = (alert.scope.clone(), alert.period);
        if let Some((last_level, last_time)) = self.last_dispatched.get(&key) {
            if *last_level == alert.level && last_time.elapsed() < ALERT_DEDUP_INTERVAL {
                return false;
            }
        }
        true
    }

    pub fn dispatch(&mut self, alerts: &[Alert]) -> anyhow::Result<()> {
        let Some(webhook_url) = self.webhook_url.clone() else {
            return Ok(());
        };
        if !webhook_url.starts_with("https://") && !webhook_url.starts_with("http://") {
            return Err(anyhow::anyhow!(
                "webhook URL must start with http:// or https://, got {:?}",
                webhook_url
            ));
        }
        let actionable: Vec<&Alert> = alerts.iter().filter(|a| a.is_actionable()).collect();
        if actionable.is_empty() {
            return Ok(());
        }

        let mut to_send: Vec<&Alert> = Vec::new();
        for alert in &actionable {
            if self.should_dispatch(alert) {
                to_send.push(*alert);
            }
        }
        if to_send.is_empty() {
            return Ok(());
        }

        let payload = serde_json::json!({
            "tool": "ai-usage-tui",
            "timestamp": crate::utils::now(),
            "alerts": to_send.iter().map(|a| a.to_json()).collect::<Vec<_>>(),
        });

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        client
            .post(webhook_url.as_str())
            .json(&payload)
            .send()?
            .error_for_status()?;

        for alert in to_send {
            self.last_dispatched.insert(
                (alert.scope.clone(), alert.period),
                (alert.level, Instant::now()),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Category, CostStatus, Usage};

    fn make_usage(provider: &str, model: &str, cost: f64, created: i64) -> Usage {
        Usage {
            provider: provider.into(),
            model: model.into(),
            category: Category::Paid,
            cost_status: CostStatus::Calculated,
            cost: Some(cost),
            requests: 1,
            created,
            ..Default::default()
        }
    }

    /// A row that should carry a price and does not: a paid provider, a model no table knows.
    fn unpriced_usage(provider: &str, model: &str) -> Usage {
        Usage {
            provider: provider.into(),
            model: model.into(),
            category: Category::Paid,
            cost_status: CostStatus::Unavailable,
            cost: None,
            requests: 1,
            created: crate::utils::now(),
            ..Default::default()
        }
    }

    /// A row billed against a plan: real work, no per-request figure.
    fn quota_usage(provider: &str, model: &str) -> Usage {
        Usage {
            provider: provider.into(),
            model: model.into(),
            category: Category::Paid,
            cost_status: CostStatus::Quota,
            billing: crate::model::Billing::Subscription,
            cost: None,
            api_equivalent_cost: Some(50.0),
            requests: 1,
            created: crate::utils::now(),
            ..Default::default()
        }
    }

    fn global_monthly(limit: f64) -> BudgetsConfig {
        BudgetsConfig {
            entry: vec![BudgetEntry {
                scope: BudgetScopeKind::Global,
                name: None,
                period: BudgetPeriod::Monthly,
                limit,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn global_budget_checks_all_usages() {
        let config = BudgetsConfig {
            entry: vec![BudgetEntry {
                scope: BudgetScopeKind::Global,
                name: None,
                period: BudgetPeriod::Monthly,
                limit: 100.0,
                warn: Some(50.0),
                critical: Some(80.0),
            }],
            ..Default::default()
        };
        let engine = BudgetEngine::from_config(&config);
        let usages = vec![
            make_usage("opencode", "gpt-5.6-luna", 40.0, crate::utils::now()),
            make_usage("opencode", "gpt-5.6-sol", 50.0, crate::utils::now()),
        ];
        let alerts = engine.check(&usages);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].level, AlertLevel::Critical);
        assert!((alerts[0].pct - 90.0).abs() < 0.01);
    }

    #[test]
    fn provider_budget_filters_by_provider() {
        let config = BudgetsConfig {
            entry: vec![BudgetEntry {
                scope: BudgetScopeKind::Provider,
                name: Some("opencode".into()),
                period: BudgetPeriod::Monthly,
                limit: 10.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = BudgetEngine::from_config(&config);
        let usages = vec![
            make_usage("opencode", "gpt-5.6-luna", 8.0, crate::utils::now()),
            make_usage("ollama", "qwen3-coder", 5.0, crate::utils::now()),
        ];
        let alerts = engine.check(&usages);
        assert_eq!(alerts.len(), 1);
        assert!((alerts[0].spend - 8.0).abs() < 0.01);
        assert_eq!(alerts[0].level, AlertLevel::Warn);
    }

    #[test]
    fn model_budget_filters_by_model() {
        let config = BudgetsConfig {
            entry: vec![BudgetEntry {
                scope: BudgetScopeKind::Model,
                name: Some("gpt-5.6-sol".into()),
                period: BudgetPeriod::Monthly,
                limit: 5.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = BudgetEngine::from_config(&config);
        let usages = vec![
            make_usage("opencode", "gpt-5.6-sol", 4.0, crate::utils::now()),
            make_usage("opencode", "gpt-5.6-luna", 100.0, crate::utils::now()),
        ];
        let alerts = engine.check(&usages);
        assert_eq!(alerts.len(), 1);
        assert!((alerts[0].spend - 4.0).abs() < 0.01);
    }

    #[test]
    fn exceeded_level_when_over_limit() {
        let config = BudgetsConfig {
            entry: vec![BudgetEntry {
                scope: BudgetScopeKind::Global,
                name: None,
                period: BudgetPeriod::Monthly,
                limit: 10.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = BudgetEngine::from_config(&config);
        let usages = vec![make_usage(
            "opencode",
            "gpt-5.6-luna",
            15.0,
            crate::utils::now(),
        )];
        let alerts = engine.check(&usages);
        assert_eq!(alerts[0].level, AlertLevel::Exceeded);
    }

    #[test]
    fn free_and_local_costs_excluded() {
        let config = BudgetsConfig {
            entry: vec![BudgetEntry {
                scope: BudgetScopeKind::Global,
                name: None,
                period: BudgetPeriod::Monthly,
                limit: 10.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        let engine = BudgetEngine::from_config(&config);
        let usages = vec![
            Usage {
                provider: "opencode".into(),
                model: "nemotron-3-ultra-free".into(),
                category: Category::Free,
                cost_status: CostStatus::Free,
                cost: Some(5.0),
                requests: 1,
                created: crate::utils::now(),
                ..Default::default()
            },
            Usage {
                provider: "ollama".into(),
                model: "qwen3-coder".into(),
                category: Category::Local,
                cost_status: CostStatus::Local,
                cost: Some(3.0),
                requests: 1,
                created: crate::utils::now(),
                ..Default::default()
            },
            make_usage("opencode", "gpt-5.6-luna", 3.0, crate::utils::now()),
        ];
        let alerts = engine.check(&usages);
        assert!((alerts[0].spend - 3.0).abs() < 0.01);
        // Costless is not unpriced: a free or local row must not mark the budget a floor.
        assert!(!alerts[0].is_partial());
        assert_eq!(alerts[0].unpriced_requests, 0);
        assert_eq!(alerts[0].quota_requests, 0);
    }

    fn entry(scope: BudgetScopeKind, name: Option<&str>, limit: f64) -> BudgetEntry {
        BudgetEntry {
            scope,
            name: name.map(str::to_string),
            period: BudgetPeriod::Monthly,
            limit,
            ..Default::default()
        }
    }

    fn validate(entries: Vec<BudgetEntry>) -> Result<(), String> {
        BudgetsConfig {
            entry: entries,
            ..Default::default()
        }
        .validate()
        .map_err(|e| e.to_string())
    }

    #[test]
    fn a_scoped_budget_without_a_name_is_refused_rather_than_matching_nothing() {
        // Restore the bug by dropping `validate`: the entry becomes `Provider("")`, matches no
        // row, and sits in the panel at `$0.00 / 0% / OK` looking configured.
        let error = validate(vec![entry(BudgetScopeKind::Provider, None, 10.0)]).unwrap_err();
        assert!(
            error.contains("#1") && error.contains("needs a `name`"),
            "{error}"
        );
        let error = validate(vec![entry(BudgetScopeKind::Model, Some("  "), 10.0)]).unwrap_err();
        assert!(
            error.contains("model") && error.contains("needs a `name`"),
            "{error}"
        );
        // And the anti-test: a named one is fine.
        validate(vec![entry(BudgetScopeKind::Provider, Some("openai"), 10.0)]).unwrap();
        // Padding inside the quotes passes validation and is trimmed on the way in, so the
        // scope the engine matches is the one validation approved — not `" openai "`, which
        // would have matched nothing and sat at OK like the nameless entry it replaced.
        let padded = entry(BudgetScopeKind::Provider, Some(" openai "), 10.0);
        validate(vec![padded.clone()]).unwrap();
        let engine = BudgetEngine::from_config(&BudgetsConfig {
            entry: vec![padded],
            ..Default::default()
        });
        assert_eq!(
            engine.budgets()[0].scope,
            BudgetScope::Provider("openai".into())
        );
    }

    #[test]
    fn a_global_budget_with_a_name_is_refused_rather_than_ignoring_it() {
        // A key that parses and does nothing is an error here, not a no-op.
        let error = validate(vec![entry(BudgetScopeKind::Global, Some("all"), 10.0)]).unwrap_err();
        assert!(error.contains("does not take a `name`"), "{error}");
        validate(vec![entry(BudgetScopeKind::Global, None, 10.0)]).unwrap();
    }

    #[test]
    fn a_limit_that_can_never_be_reached_is_refused() {
        // `pct` is `spend / limit` when `limit > 0` and `0.0` otherwise, so a zero or negative
        // limit was a budget that read `OK` forever.
        for limit in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            let error = validate(vec![entry(BudgetScopeKind::Global, None, limit)]).unwrap_err();
            assert!(error.contains("`limit`"), "{limit}: {error}");
        }
    }

    #[test]
    fn thresholds_are_percentages_in_order() {
        let mut inverted = entry(BudgetScopeKind::Global, None, 10.0);
        inverted.warn = Some(90.0);
        inverted.critical = Some(75.0);
        let error = validate(vec![inverted]).unwrap_err();
        assert!(error.contains("must be below"), "{error}");

        let mut over = entry(BudgetScopeKind::Global, None, 10.0);
        over.warn = Some(150.0);
        let error = validate(vec![over]).unwrap_err();
        assert!(error.contains("at most 100"), "{error}");

        // A warn above the default critical is the same inversion with one side implied.
        let mut implied = entry(BudgetScopeKind::Global, None, 10.0);
        implied.warn = Some(95.0);
        let error = validate(vec![implied]).unwrap_err();
        assert!(
            error.contains("must be below") && error.contains("90, the default"),
            "{error}"
        );

        let mut fine = entry(BudgetScopeKind::Global, None, 10.0);
        fine.warn = Some(50.0);
        fine.critical = Some(100.0);
        validate(vec![fine]).unwrap();
    }

    #[test]
    fn the_error_names_the_entry_that_failed() {
        let entries = vec![
            entry(BudgetScopeKind::Global, None, 10.0),
            entry(BudgetScopeKind::Provider, None, 10.0),
        ];
        let error = validate(entries).unwrap_err();
        assert!(error.starts_with("[[budgets.entry]] #2"), "{error}");
    }

    #[test]
    fn alert_dedup_prevents_repeated_dispatch() {
        let mut dispatcher = AlertDispatcher::new(None);
        let alert = Alert {
            scope: BudgetScope::Global,
            period: BudgetPeriod::Monthly,
            spend: 80.0,
            limit: 100.0,
            pct: 80.0,
            level: AlertLevel::Critical,
            unpriced_requests: 0,
            quota_requests: 0,
        };
        assert!(dispatcher.should_dispatch(&alert));
        dispatcher.last_dispatched.insert(
            (alert.scope.clone(), alert.period),
            (alert.level, Instant::now()),
        );
        assert!(!dispatcher.should_dispatch(&alert));
    }

    #[test]
    fn ok_alerts_are_not_actionable() {
        let alert = Alert {
            scope: BudgetScope::Global,
            period: BudgetPeriod::Monthly,
            spend: 10.0,
            limit: 100.0,
            pct: 10.0,
            level: AlertLevel::Ok,
            unpriced_requests: 0,
            quota_requests: 0,
        };
        assert!(!alert.is_actionable());
    }

    #[test]
    fn unpriced_work_is_counted_rather_than_dropped() {
        // Restore the bug by summing `filter_map(|u| u.cost)` straight into `spend` with no
        // counter beside it: the two unpriced rows vanish, and a $10 budget over $1 of priced
        // work plus two requests nobody could price reads as 10% spent, exactly.
        let engine = BudgetEngine::from_config(&global_monthly(10.0));
        let usages = vec![
            make_usage("opencode", "gpt-5.6-luna", 1.0, crate::utils::now()),
            unpriced_usage("opencode", "a-model-no-table-has"),
            unpriced_usage("opencode", "a-model-no-table-has"),
            quota_usage("anthropic", "claude-opus-5"),
        ];
        let alerts = engine.check(&usages);
        let alert = &alerts[0];
        assert!(
            (alert.spend - 1.0).abs() < 1e-9,
            "spend is the priced floor, got {}",
            alert.spend
        );
        assert_eq!(alert.unpriced_requests, 2);
        assert_eq!(alert.quota_requests, 1);
        assert!(alert.is_partial());
        assert!(!alert.is_quota_only());
    }

    #[test]
    fn a_fully_priced_budget_is_not_a_floor() {
        // The anti-test: the fix must not mark every budget as partial.
        let engine = BudgetEngine::from_config(&global_monthly(10.0));
        let usages = vec![
            make_usage("opencode", "gpt-5.6-luna", 1.0, crate::utils::now()),
            make_usage("opencode", "gpt-5.6-sol", 2.0, crate::utils::now()),
        ];
        let alert = &engine.check(&usages)[0];
        assert!((alert.spend - 3.0).abs() < 1e-9);
        assert_eq!(alert.unpriced_requests, 0);
        assert_eq!(alert.quota_requests, 0);
        assert!(!alert.is_partial());
        assert!(!alert.is_quota_only());
    }

    #[test]
    fn a_budget_over_only_quota_work_says_so() {
        // On a Max account every Anthropic row is quota-billed. `$0.00 / 0% / OK` is what this
        // read before: true of the per-token figure, and false about the work.
        let engine = BudgetEngine::from_config(&global_monthly(10.0));
        let usages = vec![
            quota_usage("anthropic", "claude-opus-5"),
            quota_usage("anthropic", "claude-sonnet-5"),
        ];
        let alert = &engine.check(&usages)[0];
        assert_eq!(alert.spend, 0.0);
        assert_eq!(alert.quota_requests, 2);
        assert!(alert.is_quota_only());
        assert_eq!(alert.level, AlertLevel::Ok, "no threshold was crossed");
    }

    #[test]
    fn the_alert_json_carries_what_the_figure_is_standing_on() {
        // One shape for the webhook and `--check-budgets`: they were two hand-copied literals.
        let alert = Alert {
            scope: BudgetScope::Provider("openai".into()),
            period: BudgetPeriod::Daily,
            spend: 1.5,
            limit: 10.0,
            pct: 15.0,
            level: AlertLevel::Warn,
            unpriced_requests: 3,
            quota_requests: 7,
        };
        let json = alert.to_json();
        assert_eq!(json["scope"], "provider:openai");
        assert_eq!(json["period"], "daily");
        assert_eq!(json["level"], "WARN");
        assert_eq!(json["spend"], 1.5);
        assert_eq!(json["limit"], 10.0);
        assert_eq!(json["pct"], 15.0);
        assert_eq!(json["unpriced_requests"], 3);
        assert_eq!(json["quota_requests"], 7);
    }

    #[test]
    fn empty_engine_produces_no_alerts() {
        let engine = BudgetEngine::empty();
        let alerts = engine.check(&[make_usage(
            "opencode",
            "gpt-5.6-luna",
            100.0,
            crate::utils::now(),
        )]);
        assert!(alerts.is_empty());
    }

    #[test]
    fn subscription_work_does_not_count_toward_spend() {
        // Restore the bug by stamping the row Estimated with cost Some(50.0): spend becomes
        // 51.0 and a $10 budget reads as exceeded on money that was never charged.
        for scope in [BudgetScopeKind::Global, BudgetScopeKind::Provider] {
            let config = BudgetsConfig {
                entry: vec![BudgetEntry {
                    scope,
                    name: Some("anthropic".into()),
                    period: BudgetPeriod::Monthly,
                    limit: 10.0,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let engine = BudgetEngine::from_config(&config);
            let usages = vec![
                quota_usage("anthropic", "claude-opus-5"),
                make_usage("anthropic", "claude-opus-5", 1.0, crate::utils::now()),
            ];
            let alerts = engine.check(&usages);
            assert!(
                (alerts[0].spend - 1.0).abs() < 1e-9,
                "{scope:?}: {}",
                alerts[0].spend
            );
            assert_eq!(alerts[0].level, AlertLevel::Ok);
            // Excluded from the sum is not the same as gone: the work happened.
            assert_eq!(alerts[0].quota_requests, 1, "{scope:?}");
            assert_eq!(alerts[0].unpriced_requests, 0, "{scope:?}");
        }
    }
}
