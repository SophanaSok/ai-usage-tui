use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{Datelike, TimeZone, Utc};
use serde::Deserialize;

use crate::model::Usage;

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
    pub spend: f64,
    pub limit: f64,
    pub pct: f64,
    pub level: AlertLevel,
}

impl Alert {
    pub fn is_actionable(&self) -> bool {
        self.level != AlertLevel::Ok
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct BudgetEntry {
    pub scope: BudgetScopeKind,
    pub name: Option<String>,
    pub period: BudgetPeriod,
    pub limit: f64,
    pub warn: Option<f64>,
    pub critical: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct BudgetsConfig {
    pub webhook: Option<String>,
    pub entry: Vec<BudgetEntry>,
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
                scope: match e.scope {
                    BudgetScopeKind::Global => BudgetScope::Global,
                    BudgetScopeKind::Provider => {
                        BudgetScope::Provider(e.name.clone().unwrap_or_default())
                    }
                    BudgetScopeKind::Model => {
                        BudgetScope::Model(e.name.clone().unwrap_or_default())
                    }
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
                let spend = spend_for_scope(usages, &budget.scope, budget.period);
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
                }
            })
            .collect()
    }
}

fn period_cutoff(period: BudgetPeriod) -> i64 {
    let now = Utc::now();
    match period {
        BudgetPeriod::Daily => {
            Utc.with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
                .unwrap()
                .timestamp()
        }
        BudgetPeriod::Monthly => {
            Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
                .unwrap()
                .timestamp()
        }
    }
}

fn matches_scope(usage: &Usage, scope: &BudgetScope) -> bool {
    match scope {
        BudgetScope::Global => true,
        BudgetScope::Provider(name) => usage.provider.eq_ignore_ascii_case(name),
        BudgetScope::Model(name) => usage.model.eq_ignore_ascii_case(name),
    }
}

fn spend_for_scope(usages: &[Usage], scope: &BudgetScope, period: BudgetPeriod) -> f64 {
    let cutoff = period_cutoff(period);
    usages
        .iter()
        .filter(|u| u.created >= cutoff)
        .filter(|u| u.cost_status.is_billable())
        .filter(|u| matches_scope(u, scope))
        .filter_map(|u| u.cost)
        .sum()
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
            "alerts": to_send.iter().map(|a| serde_json::json!({
                "scope": a.scope.label(),
                "period": match a.period {
                    BudgetPeriod::Daily => "daily",
                    BudgetPeriod::Monthly => "monthly",
                },
                "level": a.level.label(),
                "spend": a.spend,
                "limit": a.limit,
                "pct": a.pct,
            })).collect::<Vec<_>>(),
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
            self.last_dispatched
                .insert((alert.scope.clone(), alert.period), (alert.level, Instant::now()));
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
            created,
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
        let usages = vec![make_usage("opencode", "gpt-5.6-luna", 15.0, crate::utils::now())];
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
                created: crate::utils::now(),
                ..Default::default()
            },
            Usage {
                provider: "ollama".into(),
                model: "qwen3-coder".into(),
                category: Category::Local,
                cost_status: CostStatus::Local,
                cost: Some(3.0),
                created: crate::utils::now(),
                ..Default::default()
            },
            make_usage("opencode", "gpt-5.6-luna", 3.0, crate::utils::now()),
        ];
        let alerts = engine.check(&usages);
        assert!((alerts[0].spend - 3.0).abs() < 0.01);
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
        };
        assert!(dispatcher.should_dispatch(&alert));
        dispatcher
            .last_dispatched
            .insert((alert.scope.clone(), alert.period), (alert.level, Instant::now()));
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
        };
        assert!(!alert.is_actionable());
    }

    #[test]
    fn empty_engine_produces_no_alerts() {
        let engine = BudgetEngine::empty();
        let alerts = engine.check(&[make_usage("opencode", "gpt-5.6-luna", 100.0, crate::utils::now())]);
        assert!(alerts.is_empty());
    }
}