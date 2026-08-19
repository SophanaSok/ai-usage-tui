//! Pure aggregation over `Usage` rows.
//!
//! Deliberately free of any ratatui types: these are the numbers the dashboard shows, and they
//! are unit-testable without constructing a terminal or an `App`.

use crate::model::{DayTotals, ProjectTotals, Usage};

use super::app::Coverage;

pub fn project_labels(paths: &[String]) -> Vec<String> {
    fn segments(path: &str) -> Vec<&str> {
        path.split(['/', '\\']).filter(|s| !s.is_empty()).collect()
    }

    let split: Vec<Vec<&str>> = paths.iter().map(|p| segments(p)).collect();
    let deepest = split.iter().map(Vec::len).max().unwrap_or(0);

    let mut labels: Vec<String> = Vec::with_capacity(paths.len());
    for (index, parts) in split.iter().enumerate() {
        if parts.is_empty() {
            labels.push(paths[index].clone());
            continue;
        }
        let mut take = 1;
        while take < parts.len().min(deepest) {
            let candidate = &parts[parts.len() - take..];
            let collides = split.iter().enumerate().any(|(other, other_parts)| {
                other != index
                    && other_parts.len() >= take
                    && &other_parts[other_parts.len() - take..] == candidate
            });
            if !collides {
                break;
            }
            take += 1;
        }
        labels.push(parts[parts.len() - take..].join("/"));
    }
    labels
}

/// Roll usage up by project.
///
/// `project` and `session_id` have been populated by the Claude Code collector since it
/// landed, and nothing rendered them. Sorted by cost, then tokens: the question this view
/// answers is "where is the money going", and a project can burn tokens cheaply.
pub fn project_totals(usages: &[Usage]) -> Vec<ProjectTotals> {
    use std::collections::{BTreeMap, HashSet};

    struct Acc {
        totals: ProjectTotals,
        sessions: HashSet<String>,
        models: HashSet<String>,
    }

    let mut grouped: BTreeMap<String, Acc> = BTreeMap::new();
    for usage in usages {
        // Usage from a source that records no project still has to be accounted for
        // somewhere, or the per-project totals silently disagree with the headline total.
        let name = usage
            .project
            .clone()
            .unwrap_or_else(|| "(unattributed)".to_string());
        let acc = grouped.entry(name.clone()).or_insert_with(|| Acc {
            totals: ProjectTotals {
                project: name,
                ..Default::default()
            },
            sessions: HashSet::new(),
            models: HashSet::new(),
        });
        acc.totals.requests += usage.requests;
        acc.totals.tokens += usage.total_tokens();
        if usage.cost_status.needs_price() {
            match usage.cost.filter(|_| usage.cost_status.is_billable()) {
                Some(cost) => acc.totals.cost += cost,
                None => acc.totals.unpriced_requests += usage.requests,
            }
        }
        if let Some(session) = &usage.session_id {
            acc.sessions.insert(session.clone());
        }
        acc.models
            .insert(format!("{}/{}", usage.provider, usage.model));
    }

    let mut rows: Vec<ProjectTotals> = grouped
        .into_values()
        .map(|acc| ProjectTotals {
            sessions: acc.sessions.len(),
            models: acc.models.len(),
            ..acc.totals
        })
        .collect();
    rows.sort_by(|a, b| {
        b.cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.tokens.cmp(&a.tokens))
    });
    rows
}

pub fn coverage(usages: &[Usage]) -> Coverage {
    let mut coverage = Coverage::default();
    for usage in usages {
        if !usage.cost_status.needs_price() {
            continue;
        }
        coverage.billable_requests += usage.requests;
        if usage.cost.is_some() && usage.cost_status.is_billable() {
            coverage.priced_requests += usage.requests;
        }
    }
    coverage
}

/// Roll usage up by local calendar day, oldest first, with empty days filled in.
///
/// Gaps matter: a chart that silently omits days with no usage compresses a quiet week into
/// the same width as a busy one and misreads as steady activity. A day with no requests is a
/// real observation and gets a zero bar.
pub fn daily_totals(usages: &[Usage]) -> Vec<DayTotals> {
    use std::collections::BTreeMap;

    let mut by_day: BTreeMap<chrono::NaiveDate, DayTotals> = BTreeMap::new();
    for usage in usages {
        let Some(day) = local_day(usage.created) else {
            continue;
        };
        let entry = by_day.entry(day).or_insert_with(|| DayTotals {
            day: day.format("%Y-%m-%d").to_string(),
            ..Default::default()
        });
        entry.requests += usage.requests;
        entry.tokens += usage.total_tokens();
        if usage.cost_status.needs_price() {
            match usage.cost.filter(|_| usage.cost_status.is_billable()) {
                Some(cost) => entry.cost += cost,
                None => entry.unpriced_requests += usage.requests,
            }
        }
    }

    let (Some(first), Some(last)) = (by_day.keys().next().copied(), by_day.keys().last().copied())
    else {
        return Vec::new();
    };

    let mut days = Vec::new();
    let mut cursor = first;
    while cursor <= last {
        days.push(by_day.remove(&cursor).unwrap_or_else(|| DayTotals {
            day: cursor.format("%Y-%m-%d").to_string(),
            ..Default::default()
        }));
        cursor = match cursor.succ_opt() {
            Some(next) => next,
            None => break,
        };
    }
    days
}

/// The local calendar day an event happened on, or `None` for an undated event.
fn local_day(created: i64) -> Option<chrono::NaiveDate> {
    use chrono::TimeZone;
    if created <= 0 {
        return None;
    }
    match chrono::Local.timestamp_opt(created, 0) {
        chrono::offset::LocalResult::Single(dt) => Some(dt.date_naive()),
        _ => None,
    }
}
