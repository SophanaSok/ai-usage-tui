//! Writing a record Omarchy's agents panel will show.
//!
//! The panel tabs any `<id>.json` that lands in its usage directory, whoever wrote it — the
//! updater's own collectors live in a root-owned directory that a user cannot add to, and the
//! panel does not care. So this tool can put its own sources in the bar: an `opencode` tab for
//! everything OpenCode routed (every provider), and optionally an `ollama` tab, with the tool's
//! budgets drawn as the panel's rate-limit meters. Claude Code and Codex rows are deliberately
//! left out: Omarchy's own `claude` and `codex` tabs already cover those logs, and a record
//! named after either would overwrite Omarchy's file.
//!
//! Nothing here runs unless `--omarchy-record` is given. The record carries token counts, model
//! ids, request and session counts, and budget figures; never content, never a path.
//!
//! Field names and semantics follow Omarchy's `omarchy-agent-usage-claude` collector: a prompt
//! is one assistant message, a session is one distinct session id, `outputTokens` includes
//! reasoning, `recentDays` is exactly the last seven local dates ending today (the tokens live
//! in a field still called `messageCount`), and a limit's `percent` is a 0..1 fraction the
//! panel alarms on at 0.9.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Datelike, TimeZone};
use serde::Serialize;

use crate::budget::{Alert, BudgetPeriod};
use crate::model::Usage;

/// Ids this tool may write. `claude`, `codex` and `fireworks` are Omarchy's own files.
pub const ALLOWED_IDS: &[&str] = &["opencode", "ollama"];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OmarchyRecord {
    pub schema_version: u8,
    pub id: String,
    pub name: String,
    pub updated_at: String,
    pub ready: bool,
    pub has_local_stats: bool,
    pub has_prompt_stats: bool,
    /// "Budget $50/month", or "Pay as you go": an empty label renders as "Subscription".
    pub tier_label: String,
    pub usage_status_text: String,
    pub auth_help_text: String,
    pub limits: Vec<OmarchyLimit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<OmarchyBalance>,
    pub today_prompts: u64,
    pub today_sessions: u64,
    pub today_total_tokens: u64,
    pub today_tokens_by_model: BTreeMap<String, u64>,
    pub recent_days: Vec<OmarchyDay>,
    pub total_prompts: u64,
    pub total_sessions: u64,
    pub active_days: u64,
    pub active_dates: Vec<String>,
    pub model_usage: BTreeMap<String, OmarchyBucket>,
}

/// One budget, drawn as a rate-limit meter with a reset countdown.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OmarchyLimit {
    pub label: String,
    /// The panel parses a window out of the label's free text; an explicit title stops a
    /// scope name like "model:gpt-5.1" from reading as a one-minute window.
    pub title: String,
    /// 0..1, clamped: an exceeded budget shows as 100 %, not 150 %.
    pub percent: f64,
    pub resets_at: String,
}

/// A budget drawn as the panel's prepaid ledger. Opt-in: the panel labels this "Prepaid
/// credits … funded", which describes a soft budget loosely.
#[derive(Clone, Debug, Serialize)]
pub struct OmarchyBalance {
    pub remaining: f64,
    pub funded: f64,
    pub spent: f64,
    pub currency: String,
    /// Always true here: this tool never sees a provider ledger.
    pub estimated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct OmarchyDay {
    pub date: String,
    /// Tokens, despite the name — a legacy field shared with Omarchy's synced snapshots.
    #[serde(rename = "messageCount")]
    pub message_count: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OmarchyBucket {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

/// What a record is built from. Pure over `now`, so tests never touch the clock.
pub struct RecordSpec<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// The rows this tab counts. Already priced and deduplicated.
    pub rows: &'a [Usage],
    /// Every budget's state, computed over the tool's whole usage — the same figures
    /// `--check-budgets` reports, not a re-derivation from this tab's rows alone.
    pub alerts: &'a [Alert],
    /// Which alert becomes the prepaid ledger, if any.
    pub balance: Option<&'a Alert>,
    pub now: i64,
}

pub fn build_record(spec: &RecordSpec<'_>) -> OmarchyRecord {
    let today = local_date(spec.now).unwrap_or_else(|| chrono::Local::now().date_naive());
    let recent_dates: Vec<chrono::NaiveDate> = (0..7)
        .rev()
        .filter_map(|back| today.checked_sub_days(chrono::Days::new(back)))
        .collect();

    let mut today_prompts = 0;
    let mut today_sessions = BTreeSet::new();
    let mut today_total_tokens = 0;
    let mut today_tokens_by_model = BTreeMap::new();
    let mut tokens_by_date: BTreeMap<chrono::NaiveDate, u64> = BTreeMap::new();
    let mut total_prompts = 0;
    let mut sessions = BTreeSet::new();
    let mut active_dates = BTreeSet::new();
    let mut model_usage: BTreeMap<String, OmarchyBucket> = BTreeMap::new();
    let mut unpriced = 0;
    let mut billable = 0;

    for row in spec.rows {
        total_prompts += row.requests;
        if let Some(session) = row.session_id.as_deref() {
            sessions.insert(session.to_string());
        }
        let bucket = model_usage.entry(row.model.clone()).or_default();
        bucket.input_tokens += row.input;
        // Omarchy keeps reasoning inside output; both are generated tokens.
        bucket.output_tokens += row.output + row.reasoning;
        bucket.cache_read_input_tokens += row.cache_read;
        bucket.cache_creation_input_tokens += row.cache_write;

        if row.cost_status.needs_price() {
            billable += row.requests;
            if row.cost.is_none() || !row.cost_status.is_billable() {
                unpriced += row.requests;
            }
        }

        let Some(day) = local_date(row.created) else {
            continue;
        };
        active_dates.insert(day);
        *tokens_by_date.entry(day).or_default() += row.total_tokens();
        if day == today {
            today_prompts += row.requests;
            if let Some(session) = row.session_id.as_deref() {
                today_sessions.insert(session.to_string());
            }
            today_total_tokens += row.total_tokens();
            *today_tokens_by_model.entry(row.model.clone()).or_default() += row.total_tokens();
        }
    }

    let limits: Vec<OmarchyLimit> = spec
        .alerts
        .iter()
        .map(|alert| limit_from_alert(alert, spec.now))
        .collect();
    let balance = spec.balance.map(|alert| OmarchyBalance {
        remaining: (alert.limit - alert.spend).max(0.0),
        funded: alert.limit,
        spent: alert.spend,
        currency: "USD".to_string(),
        estimated: true,
    });
    let tier_label = match spec.balance.or(spec.alerts.first()) {
        Some(alert) => format!("Budget ${:.0}/{}", alert.limit, period_noun(alert.period)),
        None => "Pay as you go".to_string(),
    };
    // The tab's own rows can be fully priced while a budget — computed over every source — is
    // not, and a budget whose period is all plan quota draws a 0 % meter over work the panel
    // cannot see. The status line is the one place the record can say what a meter stands on,
    // so it names each budget concerned rather than speaking of "the meters".
    let mut notes: Vec<String> = Vec::new();
    if unpriced > 0 {
        notes.push(format!(
            "{unpriced} of {billable} billable requests have no price; spend is a floor."
        ));
    }
    notes.extend(spec.alerts.iter().filter(|a| a.is_partial()).map(|a| {
        format!(
            "{} {}: {} budgeted requests have no price; its meter is a floor.",
            a.scope.label(),
            a.period.label(),
            a.unpriced_requests
        )
    }));
    let on_quota: Vec<String> = spec
        .alerts
        .iter()
        .filter(|a| a.is_quota_only())
        .map(|a| {
            format!(
                "{} {}: all {} requests are on quota; its meter has nothing per-token to draw.",
                a.scope.label(),
                a.period.label(),
                a.quota_requests
            )
        })
        .collect();
    let usage_status_text = if !notes.is_empty() {
        "Spend partly unpriced"
    } else if !on_quota.is_empty() {
        "Budget on quota"
    } else {
        ""
    }
    .to_string();
    notes.extend(on_quota);
    let auth_help_text = notes.join(" ");

    OmarchyRecord {
        schema_version: 1,
        id: spec.id.to_string(),
        name: spec.name.to_string(),
        updated_at: chrono::Utc
            .timestamp_opt(spec.now, 0)
            .single()
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_default(),
        ready: !spec.rows.is_empty() || !limits.is_empty(),
        has_local_stats: true,
        has_prompt_stats: true,
        tier_label,
        usage_status_text,
        auth_help_text,
        limits,
        balance,
        today_prompts,
        today_sessions: today_sessions.len() as u64,
        today_total_tokens,
        today_tokens_by_model,
        recent_days: recent_dates
            .iter()
            .map(|date| OmarchyDay {
                date: date.format("%Y-%m-%d").to_string(),
                message_count: tokens_by_date.get(date).copied().unwrap_or(0),
            })
            .collect(),
        total_prompts,
        total_sessions: sessions.len() as u64,
        active_days: active_dates.len() as u64,
        active_dates: active_dates
            .iter()
            .map(|date| date.format("%Y-%m-%d").to_string())
            .collect(),
        model_usage,
    }
}

fn limit_from_alert(alert: &Alert, now: i64) -> OmarchyLimit {
    let title = format!("{} budget", period_adjective(alert.period));
    OmarchyLimit {
        label: format!("{title} ({})", alert.scope.label()),
        title,
        percent: if alert.limit > 0.0 {
            (alert.spend / alert.limit).clamp(0.0, 1.0)
        } else {
            0.0
        },
        resets_at: next_boundary(alert.period, now)
            .and_then(|at| chrono::Utc.timestamp_opt(at, 0).single())
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_default(),
    }
}

fn period_noun(period: BudgetPeriod) -> &'static str {
    match period {
        BudgetPeriod::Daily => "day",
        BudgetPeriod::Monthly => "month",
    }
}

fn period_adjective(period: BudgetPeriod) -> &'static str {
    match period {
        BudgetPeriod::Daily => "Daily",
        BudgetPeriod::Monthly => "Monthly",
    }
}

/// The local calendar date of a timestamp, in the machine's timezone.
fn local_date(at: i64) -> Option<chrono::NaiveDate> {
    if at <= 0 {
        return None;
    }
    chrono::Local
        .timestamp_opt(at, 0)
        .single()
        .map(|dt| dt.date_naive())
}

/// When the budget period containing `now` ends: the next local midnight, or the first of
/// next month — the same boundaries `budget.rs` opens its periods on.
fn next_boundary(period: BudgetPeriod, now: i64) -> Option<i64> {
    let today = local_date(now)?;
    let next = match period {
        BudgetPeriod::Daily => today.succ_opt()?,
        BudgetPeriod::Monthly => {
            let (year, month) = if today.month() == 12 {
                (today.year() + 1, 1)
            } else {
                (today.year(), today.month() + 1)
            };
            chrono::NaiveDate::from_ymd_opt(year, month, 1)?
        }
    };
    chrono::Local
        .from_local_datetime(&next.and_hms_opt(0, 0, 0)?)
        .single()
        .map(|dt| dt.timestamp())
}

/// Write `record` as `<dir>/<id>.json`, atomically and readable by the owner only.
///
/// The temporary name must not end in `.json`: the panel lists `*.json` at depth one and would
/// instantiate a tab for a half-written file. It is also unique per process — the updater and
/// this tool may run at once, and a shared temp path means the second rename finds the first's
/// file already moved away.
pub fn write_record(dir: &Path, record: &OmarchyRecord) -> Result<PathBuf> {
    anyhow::ensure!(
        ALLOWED_IDS.contains(&record.id.as_str()),
        "record id {:?} is not one this tool may write (allowed: {})",
        record.id,
        ALLOWED_IDS.join(", ")
    );
    std::fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    let target = dir.join(format!("{}.json", record.id));
    let temporary = dir.join(format!(".{}.{}.tmp", record.id, std::process::id()));

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = (|| -> Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(serde_json::to_string(record)?.as_bytes())?;
        file.write_all(b"\n")?;
        file.flush()?;
        std::fs::rename(&temporary, &target)?;
        Ok(())
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("could not write {}", target.display()));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::{AlertLevel, BudgetScope};
    use crate::model::CostStatus;

    /// 2026-08-18T12:00:00 local, whatever the zone: noon avoids every midnight edge.
    fn noon() -> i64 {
        chrono::Local
            .with_ymd_and_hms(2026, 8, 18, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp()
    }

    fn row(model: &str, session: &str, created: i64, cost: Option<f64>) -> Usage {
        Usage {
            provider: "opencode".into(),
            model: model.into(),
            requests: 1,
            input: 100,
            output: 10,
            reasoning: 5,
            cache_read: 20,
            cache_write: 3,
            cost,
            cost_status: match cost {
                Some(_) => CostStatus::Calculated,
                None => CostStatus::Unavailable,
            },
            category: crate::model::Category::Paid,
            created,
            session_id: Some(session.into()),
            ..Default::default()
        }
    }

    fn alert(scope: BudgetScope, period: BudgetPeriod, spend: f64, limit: f64) -> Alert {
        Alert {
            scope,
            period,
            spend,
            limit,
            pct: spend / limit * 100.0,
            level: AlertLevel::Ok,
            unpriced_requests: 0,
            quota_requests: 0,
        }
    }

    fn build(rows: &[Usage], alerts: &[Alert], balance: Option<&Alert>) -> OmarchyRecord {
        build_record(&RecordSpec {
            id: "opencode",
            name: "OpenCode",
            rows,
            alerts,
            balance,
            now: noon(),
        })
    }

    #[test]
    fn reasoning_folds_into_output_and_the_buckets_carry_every_class() {
        let record = build(&[row("m", "s", noon(), Some(0.1))], &[], None);
        let bucket = &record.model_usage["m"];
        assert_eq!(bucket.input_tokens, 100);
        assert_eq!(
            bucket.output_tokens, 15,
            "output plus reasoning, as Omarchy counts them"
        );
        assert_eq!(bucket.cache_read_input_tokens, 20);
        assert_eq!(bucket.cache_creation_input_tokens, 3);
    }

    #[test]
    fn recent_days_is_exactly_seven_ending_today_and_zero_filled() {
        // `daily_totals` trims to first..last seen day; the panel wants a fixed window.
        let three_days_ago = noon() - 3 * 86_400;
        let record = build(&[row("m", "s", three_days_ago, Some(0.1))], &[], None);
        assert_eq!(record.recent_days.len(), 7);
        assert_eq!(record.recent_days[6].date, "2026-08-18");
        assert_eq!(record.recent_days[0].date, "2026-08-12");
        assert_eq!(record.recent_days[3].message_count, 138);
        assert!(
            record
                .recent_days
                .iter()
                .filter(|d| d.message_count == 0)
                .count()
                == 6
        );
    }

    #[test]
    fn today_counts_use_the_local_calendar_day_and_distinct_sessions() {
        let yesterday_late = chrono::Local
            .with_ymd_and_hms(2026, 8, 17, 23, 59, 0)
            .single()
            .unwrap()
            .timestamp();
        let rows = [
            row("m", "s1", noon(), Some(0.1)),
            row("m", "s1", noon() + 60, Some(0.1)),
            row("m", "s2", noon() + 120, Some(0.1)),
            row("m", "s3", yesterday_late, Some(0.1)),
        ];
        let record = build(&rows, &[], None);
        assert_eq!(record.today_prompts, 3);
        assert_eq!(record.today_sessions, 2);
        assert_eq!(record.total_prompts, 4);
        assert_eq!(record.total_sessions, 3);
        assert_eq!(record.active_days, 2);
        assert_eq!(record.active_dates, ["2026-08-17", "2026-08-18"]);
        assert_eq!(record.today_tokens_by_model["m"], 3 * 138);
    }

    #[test]
    fn budgets_become_limits_with_a_reset_and_the_balance_is_opt_in() {
        let monthly = alert(BudgetScope::Global, BudgetPeriod::Monthly, 15.0, 10.0);
        let daily = alert(
            BudgetScope::Provider("anthropic".into()),
            BudgetPeriod::Daily,
            1.0,
            4.0,
        );
        let record = build(&[], &[monthly.clone(), daily], None);
        assert_eq!(record.limits.len(), 2);
        assert_eq!(record.limits[0].title, "Monthly budget");
        assert_eq!(record.limits[0].label, "Monthly budget (global)");
        assert_eq!(record.limits[0].percent, 1.0, "exceeded clamps to 100 %");
        assert_eq!(
            record.limits[0].resets_at,
            "2026-09-01T00:00:00Z".replace(
                "T00:00:00Z",
                &chrono::Local
                    .with_ymd_and_hms(2026, 9, 1, 0, 0, 0)
                    .single()
                    .unwrap()
                    .with_timezone(&chrono::Utc)
                    .format("T%H:%M:%SZ")
                    .to_string()
            )
        );
        assert_eq!(record.limits[1].label, "Daily budget (provider:anthropic)");
        assert!((record.limits[1].percent - 0.25).abs() < 1e-9);
        assert!(record.balance.is_none(), "no balance unless asked for");
        assert_eq!(record.tier_label, "Budget $10/month");
        assert!(record.ready, "limits alone make a record worth a tab");

        let record = build(&[], std::slice::from_ref(&monthly), Some(&monthly));
        let balance = record.balance.unwrap();
        assert_eq!(
            balance.remaining, 0.0,
            "never negative: the panel nulls that"
        );
        assert_eq!(balance.funded, 10.0);
        assert_eq!(balance.spent, 15.0);
        assert!(balance.estimated);
    }

    #[test]
    fn without_budgets_the_record_says_pay_as_you_go_and_carries_no_meters() {
        let record = build(&[row("m", "s", noon(), Some(0.1))], &[], None);
        assert!(record.limits.is_empty());
        assert_eq!(record.tier_label, "Pay as you go");
        assert_eq!(record.usage_status_text, "");
    }

    #[test]
    fn a_budget_meter_over_unpriced_work_is_declared_a_floor() {
        // The rows this tab counts can be fully priced while the budget — computed over every
        // source — is not. The meter beside a clean status would then be the one figure on the
        // panel with nothing said about what it stands on.
        let rows = [row("m", "s", noon(), Some(0.1))];
        let mut partial = alert(BudgetScope::Global, BudgetPeriod::Monthly, 5.0, 50.0);
        partial.unpriced_requests = 4;
        let record = build(&rows, &[partial], None);
        assert_eq!(record.usage_status_text, "Spend partly unpriced");
        assert!(
            record.auth_help_text.contains("global monthly")
                && record.auth_help_text.contains("4 budgeted requests")
                && record.auth_help_text.contains("floor"),
            "the help text should name the budget:\n{}",
            record.auth_help_text
        );

        // And the anti-test: a fully priced budget over fully priced rows says nothing.
        let exact = alert(BudgetScope::Global, BudgetPeriod::Monthly, 5.0, 50.0);
        let record = build(&rows, &[exact], None);
        assert_eq!(record.usage_status_text, "");
        assert_eq!(record.auth_help_text, "");
    }

    #[test]
    fn a_budget_on_quota_says_so_rather_than_drawing_an_empty_meter() {
        // On a Max account every budgeted request is quota-billed: the meter reads 0 % over real
        // work, and `percent` has no way to say otherwise. The status line does.
        let rows = [row("m", "s", noon(), Some(0.1))];
        let mut on_quota = alert(BudgetScope::Global, BudgetPeriod::Monthly, 0.0, 50.0);
        on_quota.quota_requests = 300;
        let record = build(&rows, &[on_quota], None);
        assert_eq!(record.usage_status_text, "Budget on quota");
        assert!(
            record.auth_help_text.contains("global monthly")
                && record.auth_help_text.contains("300 requests are on quota"),
            "{}",
            record.auth_help_text
        );
        assert_eq!(
            record.limits[0].percent, 0.0,
            "the meter itself cannot say more"
        );
    }

    #[test]
    fn unpriced_billable_requests_are_reported_not_hidden() {
        let rows = [
            row("m", "s", noon(), Some(0.1)),
            row("m", "s", noon(), None),
        ];
        let record = build(&rows, &[], None);
        assert_eq!(record.usage_status_text, "Spend partly unpriced");
        assert!(
            record.auth_help_text.contains("1 of 2"),
            "{}",
            record.auth_help_text
        );
    }

    #[test]
    fn the_wire_format_matches_omarchys_contract() {
        let monthly = alert(BudgetScope::Global, BudgetPeriod::Monthly, 1.0, 10.0);
        let record = build(
            &[row("m", "s", noon(), Some(0.1))],
            std::slice::from_ref(&monthly),
            Some(&monthly),
        );
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for expected in [
            "schemaVersion",
            "id",
            "name",
            "updatedAt",
            "ready",
            "hasLocalStats",
            "hasPromptStats",
            "tierLabel",
            "usageStatusText",
            "authHelpText",
            "limits",
            "balance",
            "todayPrompts",
            "todaySessions",
            "todayTotalTokens",
            "todayTokensByModel",
            "recentDays",
            "totalPrompts",
            "totalSessions",
            "activeDays",
            "activeDates",
            "modelUsage",
        ] {
            assert!(keys.contains(&expected), "missing {expected}: {keys:?}");
        }
        let bucket = json["modelUsage"]["m"].as_object().unwrap();
        assert_eq!(
            bucket.keys().collect::<Vec<_>>(),
            [
                "cacheCreationInputTokens",
                "cacheReadInputTokens",
                "inputTokens",
                "outputTokens"
            ]
        );
        assert!(json["recentDays"][0].get("messageCount").is_some());
        for key in ["label", "title", "percent", "resetsAt"] {
            assert!(json["limits"][0].get(key).is_some(), "limit lacks {key}");
        }
        for key in ["remaining", "funded", "spent", "currency", "estimated"] {
            assert!(json["balance"].get(key).is_some(), "balance lacks {key}");
        }
        assert!(json["updatedAt"].as_str().unwrap().ends_with('Z'));
    }

    #[test]
    fn the_write_is_atomic_private_and_leaves_no_temporary_behind() {
        let dir = tempfile::TempDir::new().unwrap();
        let record = build(&[row("m", "s", noon(), Some(0.1))], &[], None);
        let target = write_record(dir.path(), &record).unwrap();
        assert_eq!(target, dir.path().join("opencode.json"));
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["opencode.json"], "no temporary may survive");
        let content = std::fs::read_to_string(&target).unwrap();
        assert!(content.ends_with('\n'));
        assert!(serde_json::from_str::<serde_json::Value>(&content).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the record names models and spend; owner only");
        }
        // The reader on the other side of this contract accepts what was written.
        let header = crate::omarchy::read_record(&target).unwrap();
        assert_eq!(header.id.as_deref(), Some("opencode"));
    }

    #[test]
    fn a_record_named_after_an_omarchy_agent_is_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut record = build(&[], &[], None);
        record.id = "claude".into();
        let error = write_record(dir.path(), &record).unwrap_err().to_string();
        assert!(error.contains("claude"), "{error}");
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_write_returns_the_error_and_no_temporary() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let record = build(&[], &[], None);
        let result = write_record(&locked, &record);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = result.unwrap_err().to_string();
        assert!(error.contains("could not write"), "{error}");
        assert!(std::fs::read_dir(&locked).unwrap().next().is_none());
    }
}
