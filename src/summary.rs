//! `--summary-json`: the whole picture in a few kilobytes.
//!
//! `--json` prints one object per request. That is the right export for a script that wants
//! rows, and the wrong one for a reader with a context window: about 130 tokens a request, so a
//! month of agent work is over a million tokens before the first question is asked. Everything a
//! reader needs to reason about *where the tokens go and whether the routing earns its cost* was
//! already computed for the dashboard -- by model, by project, by session, by day -- and was
//! rendered only there. This is that, as one compact document.
//!
//! Two rules shape it.
//!
//! **Facts, not advice.** Every figure here is a count or a ratio of counts, with the evidence it
//! rests on beside it. There are no thresholds, no verdicts and no hypothetical prices: what a
//! cache-hit percentage *means* for a project is the reader's judgement, and the guide printed by
//! `--agent-guide` is where that judgement is coached.
//!
//! **Unknown stays unknown** (convention 1), for derived figures as much as for cost. A
//! percentage over a zero denominator is `null`. So is one whose numerator nothing recorded:
//! several sources never report cache or reasoning tokens, and "0% cache hits" for a source that
//! cannot say is an invented finding. `cost` is `null` when nothing in the bucket could be priced,
//! and `cost_is_floor` says when the figure leaves unpriced requests out.
//!
//! Everything below `build` is pure: the clock, the pricing table and every file read are the
//! caller's, so the document is testable without any of them.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use crate::budget::Alert;
use crate::collector::SourceReport;
use crate::model::{accrue, Category, CostStatus, RoutingEvent, Usage};
use crate::omarchy::LimitsReport;

/// How many of the most recent days `by_day` lists. A day is small, but `--all` on a year of
/// history is not, and the recent past is what a reader optimising their usage acts on.
pub const MAX_DAYS: usize = 31;

/// How many of a session's models `by_session` names before `models_total` takes over.
pub const MAX_SESSION_MODELS: usize = 6;

/// The label usage with no recorded working directory is grouped under, as in the dashboard.
pub const UNATTRIBUTED: &str = "(unattributed)";

/// One rollup: the five token buckets, what it cost as far as that is known, and how much of it
/// could be priced at all. Every list in the document is made of these.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bucket {
    pub requests: u64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Dollars over the priced requests only; see `cost_value`.
    cost: f64,
    /// Requests that carry a billable price.
    pub priced_requests: u64,
    /// Requests that should carry a price and do not.
    pub unpriced_requests: u64,
    /// Requests billed against a plan quota: real cost, no per-request figure.
    pub quota_requests: u64,
    /// What the subscription-billed requests would have cost at list rates. Never charged.
    pub api_equivalent_cost: Option<f64>,
}

impl Bucket {
    pub fn add(&mut self, usage: &Usage) {
        self.requests += usage.requests;
        self.input += usage.input;
        self.output += usage.output;
        self.reasoning += usage.reasoning;
        self.cache_read += usage.cache_read;
        self.cache_write += usage.cache_write;
        if usage.cost_status.is_billable() && usage.cost.is_some() {
            self.priced_requests += usage.requests;
        }
        accrue(
            usage,
            &mut self.cost,
            &mut self.unpriced_requests,
            &mut self.quota_requests,
        );
        if let Some(equivalent) = usage.api_equivalent_cost {
            *self.api_equivalent_cost.get_or_insert(0.0) += equivalent;
        }
    }

    pub fn merge(&mut self, other: &Bucket) {
        self.requests += other.requests;
        self.input += other.input;
        self.output += other.output;
        self.reasoning += other.reasoning;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cost += other.cost;
        self.priced_requests += other.priced_requests;
        self.unpriced_requests += other.unpriced_requests;
        self.quota_requests += other.quota_requests;
        if let Some(equivalent) = other.api_equivalent_cost {
            *self.api_equivalent_cost.get_or_insert(0.0) += equivalent;
        }
    }

    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.reasoning + self.cache_read + self.cache_write
    }

    /// Dollars, where dollars are a fact.
    ///
    /// `None` when the bucket holds work that costs money and none of it could be priced --
    /// all quota, all unpriced, or a mix of the two. `Some(0.0)` only when everything in it is
    /// free or local, or it is empty: a zero nobody had to invent.
    pub fn cost_value(&self) -> Option<f64> {
        if self.priced_requests > 0 || (self.unpriced_requests == 0 && self.quota_requests == 0) {
            Some(self.cost)
        } else {
            None
        }
    }

    /// Whether `cost` leaves out requests that should have carried a price.
    pub fn cost_is_floor(&self) -> bool {
        self.cost_value().is_some() && self.unpriced_requests > 0
    }

    /// Prompt tokens served from cache, as a percentage of all prompt tokens.
    ///
    /// `None` when no cache activity was recorded at all. Several sources never report cache
    /// tokens, and nothing on a row distinguishes "did not cache" from "did not say", so a bucket
    /// with no cache tokens has no ratio rather than a ratio of zero.
    pub fn cache_hit_pct(&self) -> Option<f64> {
        if self.cache_read + self.cache_write == 0 {
            return None;
        }
        percent(
            self.cache_read,
            self.input + self.cache_read + self.cache_write,
        )
    }

    pub fn output_pct(&self) -> Option<f64> {
        percent(self.output, self.tokens())
    }

    /// `None` when no reasoning tokens were recorded, for the reason `cache_hit_pct` gives:
    /// Claude Code, for one, counts thinking inside `output_tokens` and reports no split.
    pub fn reasoning_pct(&self) -> Option<f64> {
        if self.reasoning == 0 {
            return None;
        }
        percent(self.reasoning, self.tokens())
    }

    pub fn tokens_per_request(&self) -> Option<f64> {
        (self.requests > 0).then(|| round(self.tokens() as f64 / self.requests as f64, 1))
    }

    /// Dollars per request, over the requests that carry a price and no others.
    pub fn cost_per_request(&self) -> Option<f64> {
        (self.priced_requests > 0).then(|| round(self.cost / self.priced_requests as f64, 6))
    }

    /// The bucket as JSON. `of_tokens` is the document's total, for `share_of_tokens_pct`.
    pub fn to_json(&self, of_tokens: u64) -> Value {
        json!({
            "requests": self.requests,
            "tokens": self.tokens(),
            "input_tokens": self.input,
            "output_tokens": self.output,
            "reasoning_tokens": self.reasoning,
            "cache_read_tokens": self.cache_read,
            "cache_write_tokens": self.cache_write,
            "cost": self.cost_value().map(|cost| round(cost, 6)),
            "cost_is_floor": self.cost_is_floor(),
            "priced_requests": self.priced_requests,
            "unpriced_requests": self.unpriced_requests,
            "quota_requests": self.quota_requests,
            "api_equivalent_cost": self.api_equivalent_cost.map(|cost| round(cost, 6)),
            "metrics": {
                "share_of_tokens_pct": percent(self.tokens(), of_tokens),
                "cache_hit_pct": self.cache_hit_pct(),
                "output_pct": self.output_pct(),
                "reasoning_pct": self.reasoning_pct(),
                "tokens_per_request": self.tokens_per_request(),
                "cost_per_request": self.cost_per_request(),
            },
        })
    }
}

impl Bucket {
    /// The bucket without its five-way token split, for the long lists -- days and sessions.
    ///
    /// Same keys at the same paths as `to_json`, fewer of them: a reader that knows one shape
    /// knows both. Thirty-one days of full buckets were a third of the whole document, and the
    /// split is one `--session` or `--days` drill-down away when it is wanted.
    pub fn to_json_brief(&self, of_tokens: u64) -> Value {
        json!({
            "requests": self.requests,
            "tokens": self.tokens(),
            "cost": self.cost_value().map(|cost| round(cost, 6)),
            "cost_is_floor": self.cost_is_floor(),
            "unpriced_requests": self.unpriced_requests,
            "quota_requests": self.quota_requests,
            "api_equivalent_cost": self.api_equivalent_cost.map(|cost| round(cost, 6)),
            "metrics": {
                "share_of_tokens_pct": percent(self.tokens(), of_tokens),
                "cache_hit_pct": self.cache_hit_pct(),
                "tokens_per_request": self.tokens_per_request(),
            },
        })
    }
}

/// `part` of `whole` on the 0..100 scale every other export uses, or `None` over nothing.
fn percent(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| round(part as f64 / whole as f64 * 100.0, 2))
}

/// Rounded for the reader, not for arithmetic: a document whose purpose is token economy should
/// not spend them on the fifteenth decimal place of a percentage.
fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// The dashboard's model table: usage grouped by provider, model, category and cost status,
/// largest first.
///
/// This lived inline in `App::recompute`. It is here so the table a user sees and the `by_model`
/// list an export prints are grouped by one key in one place; a test holds the two to each other.
pub fn model_rows(usages: &[Usage]) -> Vec<Usage> {
    let mut grouped = BTreeMap::<(String, String, Category, CostStatus), Usage>::new();
    for u in usages {
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
            entry.api_equivalent_cost = Some(entry.api_equivalent_cost.unwrap_or(0.0) + equivalent);
        }
    }
    let mut rows: Vec<Usage> = grouped.into_values().collect();
    rows.sort_by_key(|u| std::cmp::Reverse(u.total_tokens()));
    rows
}

/// A group of rows under one key, with what identifies it beside the bucket.
#[derive(Clone, Debug, Default)]
struct Group {
    bucket: Bucket,
    sessions: BTreeSet<String>,
    models: BTreeSet<String>,
    project: Option<String>,
    first_seen: i64,
    last_seen: i64,
}

impl Group {
    fn add(&mut self, usage: &Usage) {
        self.bucket.add(usage);
        if let Some(session) = &usage.session_id {
            self.sessions.insert(session.clone());
        }
        self.models
            .insert(format!("{}/{}", usage.provider, usage.model));
        if self.project.is_none() {
            self.project.clone_from(&usage.project);
        }
        // An undated row (`created == 0`) says nothing about when the group ran.
        if usage.created > 0 {
            if self.first_seen <= 0 || usage.created < self.first_seen {
                self.first_seen = usage.created;
            }
            self.last_seen = self.last_seen.max(usage.created);
        }
    }
}

/// Group by `key`; rows it returns `None` for are folded into the second value.
fn group_by<K: Ord>(
    usages: &[Usage],
    key: impl Fn(&Usage) -> Option<K>,
) -> (BTreeMap<K, Group>, Bucket) {
    let mut groups: BTreeMap<K, Group> = BTreeMap::new();
    let mut unkeyed = Bucket::default();
    for usage in usages {
        match key(usage) {
            Some(key) => groups.entry(key).or_default().add(usage),
            None => unkeyed.add(usage),
        }
    }
    (groups, unkeyed)
}

/// The `top` largest groups by tokens as JSON rows, and everything else folded into `other`.
///
/// `other` is what keeps a truncated list honest: the rows shown plus `other` always add up to
/// the document's totals, so a reader never mistakes the ten largest projects for all of them.
/// `top == 0` lists everything.
fn top_list<K: Ord + Clone>(
    groups: BTreeMap<K, Group>,
    unkeyed: Bucket,
    top: usize,
    of_tokens: u64,
    shape: Shape,
    row: impl Fn(&K, &Group) -> Value,
) -> Value {
    let total = groups.len();
    let mut ordered: Vec<(K, Group)> = groups.into_iter().collect();
    // Tokens first; the key breaks ties so the order does not depend on the map's iteration.
    ordered.sort_by(|(ka, a), (kb, b)| {
        b.bucket
            .tokens()
            .cmp(&a.bucket.tokens())
            .then_with(|| ka.cmp(kb))
    });
    let keep = if top == 0 { total } else { top.min(total) };
    let mut other = unkeyed;
    for (_, group) in &ordered[keep..] {
        other.merge(&group.bucket);
    }
    let rows: Vec<Value> = ordered[..keep]
        .iter()
        .map(|(key, group)| {
            let mut value = row(key, group);
            merge_object(&mut value, shape.render(&group.bucket, of_tokens));
            value
        })
        .collect();
    json!({
        "total": total,
        "shown": keep,
        "rows": rows,
        // Null rather than a bucket of zeros when nothing was folded away.
        "other": (other.requests > 0).then(|| other.to_json(of_tokens)),
    })
}

/// Which of the two bucket shapes a list's rows carry. `other` is always full.
#[derive(Clone, Copy)]
enum Shape {
    Full,
    Brief,
}

impl Shape {
    fn render(self, bucket: &Bucket, of_tokens: u64) -> Value {
        match self {
            Shape::Full => bucket.to_json(of_tokens),
            Shape::Brief => bucket.to_json_brief(of_tokens),
        }
    }
}

fn merge_object(into: &mut Value, from: Value) {
    if let (Some(into), Value::Object(from)) = (into.as_object_mut(), from) {
        into.extend(from);
    }
}

/// What the caller has already narrowed the rows to, echoed so the document says what it covers.
#[derive(Clone, Debug, Default)]
pub struct Scope {
    pub range: String,
    /// Unix seconds the range starts at, or `None` for all history.
    pub since: Option<i64>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub project: Option<String>,
    pub session: Option<String>,
}

/// Everything `build` needs, gathered by the caller. No field is read from disk or the clock here.
/// What this binary is and whether a newer one is known: the two things `--doctor` knew and no
/// JSON document said.
pub struct Build {
    pub version: &'static str,
    /// `update::Channel::label()`: how the running binary got where it is.
    pub install_channel: &'static str,
    /// The command that upgrades an install of that kind; `None` for a location not recognised,
    /// where naming one would upgrade a copy the user is not running.
    pub upgrade_command: Option<&'static str>,
    /// The last answer `--check-update` (or an opted-in `--doctor`) cached. `None` when nothing
    /// has ever asked: the check is opt-in, and building this document never makes it.
    pub update: Option<crate::update::CachedCheck>,
}

pub struct Inputs<'a> {
    pub schema_version: u32,
    pub build: Build,
    pub now: i64,
    pub scope: Scope,
    pub top: usize,
    /// Rows already narrowed to `scope`.
    pub usages: &'a [Usage],
    pub sources: &'a [SourceReport],
    pub pricing_models: usize,
    pub pricing_warnings: &'a [String],
    /// The trailing-hour burn rate, from *all* usage: a range of "last month" has no bearing on
    /// how fast tokens are being spent now.
    pub burn: &'a crate::model::BurnRate,
    /// Every configured budget, `OK` ones included: headroom is a fact a reader needs, and
    /// `--check-budgets` prints only the ones already past a threshold.
    pub budgets: &'a [Alert],
    pub limits: &'a LimitsReport,
    /// Routing events already narrowed to the range.
    pub routing_events: &'a [RoutingEvent],
    /// Test runs seen and not recorded, already narrowed to the range.
    pub withheld: &'a [crate::collector::journal::WithheldRuns],
    /// A model's input rate, for ordering models when deriving escalations.
    pub input_rate: &'a dyn Fn(&str) -> Option<f64>,
}

pub fn build(inputs: &Inputs<'_>) -> Value {
    let usages = inputs.usages;
    let mut totals = Bucket::default();
    for usage in usages {
        totals.add(usage);
    }
    let of_tokens = totals.tokens();

    let (by_category, _) = group_by(usages, |u| Some(u.category));
    let mut categories: Vec<(Category, Group)> = by_category.into_iter().collect();
    categories.sort_by_key(|(_, group)| std::cmp::Reverse(group.bucket.tokens()));

    let (by_model, unkeyed_models) = group_by(usages, |u| {
        Some((
            u.provider.clone(),
            u.model.clone(),
            u.category,
            u.cost_status,
        ))
    });
    let (by_project, unkeyed_projects) = group_by(usages, |u| {
        Some(
            u.project
                .clone()
                .unwrap_or_else(|| UNATTRIBUTED.to_string()),
        )
    });
    let (by_session, sessionless) = group_by(usages, |u| u.session_id.clone());

    let aggregates = crate::routing::aggregate(inputs.routing_events);
    let table_dates = crate::pricing::bundled_table_dates();

    json!({
        "schema_version": inputs.schema_version,
        "generated_at": inputs.now,
        "range": { "label": inputs.scope.range, "since": inputs.scope.since },
        "filters": {
            "provider": inputs.scope.provider,
            "model": inputs.scope.model,
            "project": inputs.scope.project,
            "session": inputs.scope.session,
        },
        "build": {
            "version": inputs.build.version,
            "install_channel": inputs.build.install_channel,
            "upgrade_command": inputs.build.upgrade_command,
            "update": inputs.build.update.as_ref().map(|cached| json!({
                "latest": cached.latest,
                "checked_at": cached.checked,
                "newer": crate::update::is_newer(inputs.build.version, &cached.latest),
            })),
        },
        "sources": inputs.sources.iter().map(|source| json!({
            "id": source.id,
            "present": source.present,
            "rows": source.rows,
            "path": source.path.as_ref().map(|path| path.display().to_string()),
            "status": source.status,
            "detail": source.detail,
        })).collect::<Vec<_>>(),
        "pricing": {
            // Every dollar figure in this document, and every rate, is this. Nothing said so.
            "currency": "USD",
            "models_priced": inputs.pricing_models,
            // When the tables compiled into this build were cut. A rate is a fact as of a date,
            // and a reader weighing a cost estimate should be able to see which.
            "community_table_date": table_dates.0.map(|date| date.to_string()),
            "curated_table_date": table_dates.1.map(|date| date.to_string()),
            "warnings": inputs.pricing_warnings,
        },
        "totals": totals.to_json(of_tokens),
        "by_category": categories.iter().map(|(category, group)| {
            let mut value = json!({ "category": category.label() });
            merge_object(&mut value, group.bucket.to_json(of_tokens));
            value
        }).collect::<Vec<_>>(),
        "by_model": top_list(by_model, unkeyed_models, inputs.top, of_tokens, Shape::Full, |key, group| json!({
            "provider": key.0,
            "model": key.1,
            "category": key.2.label(),
            "cost_status": key.3.label(),
            // The pricing table's list rate, dollars per million input tokens: how expensive the
            // model is relative to the others, whatever this range happened to be billed as.
            // `null` for a free model and for one the table cannot price. It ranks; it is not
            // what anything cost.
            "list_input_rate": (inputs.input_rate)(&key.1),
            "sessions": group.sessions.len(),
        })),
        "by_project": top_list(by_project, unkeyed_projects, inputs.top, of_tokens, Shape::Full, |project, group| json!({
            "project": project,
            "sessions": group.sessions.len(),
            // A count, as in the dashboard: one real project here had used 55 models, and the
            // list was most of the row. `--project` narrows `by_model` to it.
            "models": group.models.len(),
            "first_seen": seen(group.first_seen),
            "last_seen": seen(group.last_seen),
        })),
        // `other` here also holds requests that carry no session id at all.
        "by_session": top_list(by_session, sessionless, inputs.top, of_tokens, Shape::Brief, |session, group| json!({
            "session_id": session,
            "project": group.project,
            // Named, because which models a session moved between is the routing question --
            // but only the first few: `models_total` says when there were more.
            "models": group.models.iter().take(MAX_SESSION_MODELS).collect::<Vec<_>>(),
            "models_total": group.models.len(),
            "first_seen": seen(group.first_seen),
            "last_seen": seen(group.last_seen),
            "duration_secs": (group.first_seen > 0).then(|| group.last_seen - group.first_seen),
        })),
        "by_day": days_json(usages, of_tokens),
        "burn": burn_json(inputs.burn),
        "budgets": inputs.budgets.iter().map(Alert::to_json).collect::<Vec<_>>(),
        "limits": crate::export::limits_report_json(inputs.limits),
        "limit_problems": inputs.limits.problems,
        "escalations": crate::export::escalations_json(usages, inputs.input_rate),
        "provenance": crate::export::provenance_json(usages),
        "routing": {
            "events": inputs.routing_events.len(),
            "aggregates": aggregates.iter().map(crate::routing::aggregate_json).collect::<Vec<_>>(),
            "withheld": crate::routing::withheld_json(inputs.withheld),
        },
    })
}

/// A timestamp, or `null` for a group whose rows were all undated.
fn seen(at: i64) -> Option<i64> {
    (at > 0).then_some(at)
}

/// One bucket per local calendar day that has usage, most recent `MAX_DAYS`.
///
/// Days are local for the reason the dashboard's are: "today" means the reader's today. Undated
/// rows belong to no day and are counted in `undated_requests` rather than dropped in silence.
fn days_json(usages: &[Usage], of_tokens: u64) -> Value {
    use chrono::TimeZone;
    let (by_day, undated) = group_by(usages, |u| {
        if u.created <= 0 {
            return None;
        }
        match chrono::Local.timestamp_opt(u.created, 0) {
            chrono::offset::LocalResult::Single(at) => Some(at.date_naive()),
            _ => None,
        }
    });
    let total = by_day.len();
    let rows: Vec<Value> = by_day
        .iter()
        .rev()
        .take(MAX_DAYS)
        .map(|(day, group)| {
            let mut value = json!({ "day": day.format("%Y-%m-%d").to_string() });
            merge_object(&mut value, group.bucket.to_json_brief(of_tokens));
            value
        })
        .collect();
    json!({
        "total": total,
        "shown": rows.len(),
        "rows": rows,
        "undated_requests": undated.requests,
    })
}

fn burn_json(burn: &crate::model::BurnRate) -> Value {
    json!({
        "window_secs": burn.window_secs,
        "requests": burn.requests,
        "tokens": burn.tokens,
        "tokens_per_minute": round(burn.tokens_per_minute(), 1),
        // A rate from too few requests is noise, and one over quota-only work does not exist.
        "cost_per_hour": burn.is_projectable().then(|| round(burn.cost_per_hour(), 4)),
        "cost_is_floor": burn.is_partial(),
        "quota_only": burn.is_quota_only(),
        "unpriced_requests": burn.unpriced_requests,
        "quota_requests": burn.quota_requests,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Billing, BurnRate};

    fn row(model: &str, project: Option<&str>, session: Option<&str>, tokens: u64) -> Usage {
        Usage {
            provider: "anthropic".into(),
            model: model.into(),
            category: Category::Paid,
            cost_status: CostStatus::Calculated,
            cost: Some(tokens as f64 / 1000.0),
            requests: 1,
            input: tokens,
            created: 1_787_000_000,
            project: project.map(str::to_string),
            session_id: session.map(str::to_string),
            ..Default::default()
        }
    }

    fn quota(model: &str, tokens: u64) -> Usage {
        Usage {
            cost: None,
            cost_status: CostStatus::Quota,
            billing: Billing::Subscription,
            api_equivalent_cost: Some(1.5),
            ..row(model, Some("/w/api"), Some("s-quota"), tokens)
        }
    }

    fn build_info(update: Option<crate::update::CachedCheck>) -> Build {
        Build {
            version: "0.20.0",
            install_channel: "cargo",
            upgrade_command: Some("cargo install ai-usage-tui --locked"),
            update,
        }
    }

    /// "Never checked" and "checked, nothing newer" are different answers, and only one of them
    /// says this build is current.
    #[test]
    fn a_build_that_was_never_checked_says_null_not_up_to_date() {
        let doc = document(&[], 0);
        assert_eq!(doc["build"]["version"], "0.20.0");
        assert_eq!(doc["build"]["install_channel"], "cargo");
        assert!(doc["build"]["update"].is_null());

        let with = |latest: &str| {
            let mut inputs_update = None;
            inputs_update.replace(crate::update::CachedCheck {
                latest: latest.into(),
                checked: 1_787_000_000,
            });
            build_document(&[], 0, build_info(inputs_update))["build"]["update"].clone()
        };
        assert_eq!(with("v0.21.0")["newer"], true);
        assert_eq!(with("v0.21.0")["checked_at"], 1_787_000_000);
        assert_eq!(with("v0.20.0")["newer"], false);
        assert_eq!(with("v0.19.0")["newer"], false);
    }

    fn document(usages: &[Usage], top: usize) -> Value {
        build_document(usages, top, build_info(None))
    }

    fn build_document(usages: &[Usage], top: usize, build_info: Build) -> Value {
        build(&Inputs {
            schema_version: 1,
            build: build_info,
            now: 1_787_000_100,
            scope: Scope {
                range: "ALL TIME".into(),
                ..Default::default()
            },
            top,
            usages,
            sources: &[],
            pricing_models: 0,
            pricing_warnings: &[],
            burn: &BurnRate::default(),
            budgets: &[],
            limits: &LimitsReport::default(),
            routing_events: &[],
            withheld: &[],
            input_rate: &|_| None,
        })
    }

    /// Convention 1, for the figures this module derives. Each assertion below is a place an
    /// `unwrap_or(0.0)` would read as a finding: "0% cache hits", "$0.00 per request", "free".
    #[test]
    fn a_figure_nothing_recorded_is_null_not_zero() {
        let mut bucket = Bucket::default();
        assert_eq!(bucket.cache_hit_pct(), None, "no requests at all");
        assert_eq!(bucket.tokens_per_request(), None);
        assert_eq!(bucket.cost_per_request(), None);
        assert_eq!(
            bucket.cost_value(),
            Some(0.0),
            "an empty bucket cost nothing"
        );

        bucket.add(&quota("claude-opus-5", 1000));
        assert_eq!(
            bucket.cost_value(),
            None,
            "quota work costs money nobody can state per request; it is not $0.00"
        );
        assert_eq!(bucket.cost_per_request(), None);
        assert_eq!(bucket.api_equivalent_cost, Some(1.5));
        assert_eq!(
            bucket.cache_hit_pct(),
            None,
            "no cache tokens recorded is not a 0% hit rate: several sources never report them"
        );
        assert_eq!(bucket.reasoning_pct(), None);

        let json = bucket.to_json(bucket.tokens());
        assert!(json["cost"].is_null(), "{json}");
        assert!(json["metrics"]["cache_hit_pct"].is_null(), "{json}");
        assert!(json["metrics"]["cost_per_request"].is_null(), "{json}");
    }

    #[test]
    fn cost_is_a_floor_only_when_priced_and_unpriced_work_are_mixed() {
        let mut bucket = Bucket::default();
        bucket.add(&row("claude-sonnet-5", None, None, 2000));
        assert_eq!(bucket.cost_value(), Some(2.0));
        assert!(!bucket.cost_is_floor());
        assert_eq!(bucket.cost_per_request(), Some(2.0));

        bucket.add(&Usage {
            cost: None,
            cost_status: CostStatus::Unavailable,
            ..row("mystery-model", None, None, 500)
        });
        assert_eq!(
            bucket.cost_value(),
            Some(2.0),
            "the priced part is still a fact"
        );
        assert!(bucket.cost_is_floor(), "and it now leaves a request out");
        assert_eq!(
            bucket.cost_per_request(),
            Some(2.0),
            "per priced request: the unpriced one is not averaged in as free"
        );
    }

    #[test]
    fn cache_hit_is_cache_reads_over_all_prompt_tokens() {
        let mut bucket = Bucket::default();
        bucket.add(&Usage {
            input: 100,
            cache_read: 800,
            cache_write: 100,
            output: 50,
            ..row("claude-sonnet-5", None, None, 0)
        });
        assert_eq!(bucket.cache_hit_pct(), Some(80.0));
    }

    /// A truncated list that did not say so would present the ten largest projects as all of
    /// them. `other` makes the rows add up to the totals whatever `top` is.
    #[test]
    fn the_rows_shown_and_other_always_add_up_to_the_totals() {
        let usages = vec![
            row("a", Some("/w/one"), Some("s1"), 500),
            row("a", Some("/w/two"), Some("s2"), 300),
            row("b", Some("/w/three"), Some("s3"), 200),
            row("b", None, None, 100),
        ];
        for top in [0usize, 1, 2, 10] {
            let doc = document(&usages, top);
            let total = doc["totals"]["tokens"].as_u64().unwrap();
            assert_eq!(total, 1100);
            for list in ["by_model", "by_project", "by_session"] {
                let shown: u64 = doc[list]["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["tokens"].as_u64().unwrap())
                    .sum();
                let other = doc[list]["other"]["tokens"].as_u64().unwrap_or(0);
                assert_eq!(shown + other, total, "{list} at top={top}: {}", doc[list]);
            }
        }
        let doc = document(&usages, 1);
        assert_eq!(
            doc["by_project"]["total"], 4,
            "three paths and the unattributed group"
        );
        assert_eq!(doc["by_project"]["shown"], 1);
        assert_eq!(doc["by_project"]["rows"][0]["project"], "/w/one");
        // The sessionless row is in `other`, not dropped.
        assert_eq!(doc["by_session"]["total"], 3);
        assert_eq!(document(&usages, 0)["by_session"]["other"]["tokens"], 100);
    }

    /// The dashboard's model table and the export's `by_model` are grouped by one key.
    #[test]
    fn by_model_agrees_with_the_dashboards_model_table() {
        let usages = vec![
            row("claude-sonnet-5", Some("/w/api"), Some("s1"), 400),
            row("claude-sonnet-5", Some("/w/web"), Some("s2"), 100),
            quota("claude-opus-5", 900),
            row("claude-haiku-4-5", None, None, 50),
        ];
        let table = model_rows(&usages);
        let doc = document(&usages, 0);
        let rows = doc["by_model"]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), table.len());
        for (exported, shown) in rows.iter().zip(&table) {
            assert_eq!(exported["model"], shown.model);
            assert_eq!(exported["cost_status"], shown.cost_status.label());
            assert_eq!(exported["requests"], shown.requests);
            assert_eq!(exported["tokens"], shown.total_tokens());
            assert_eq!(exported["cost"].as_f64(), shown.cost, "{}", shown.model);
        }
    }

    #[test]
    fn undated_rows_are_counted_not_dropped_from_the_days() {
        let usages = vec![
            row("a", None, None, 100),
            Usage {
                created: 0,
                ..row("a", None, None, 40)
            },
        ];
        let doc = document(&usages, 0);
        assert_eq!(doc["by_day"]["shown"], 1);
        assert_eq!(doc["by_day"]["undated_requests"], 1);
        assert_eq!(doc["totals"]["tokens"], 140);
    }
}
