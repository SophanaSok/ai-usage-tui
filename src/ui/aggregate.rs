//! Pure aggregation over `Usage` rows.
//!
//! Deliberately free of any ratatui types: these are the numbers the dashboard shows, and they
//! are unit-testable without constructing a terminal or an `App`.

use crate::model::{accrue, BurnRate, DayTotals, ProjectTotals, SessionTotals, Usage};

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
/// Where usage from a source that records no working directory is filed.
///
/// It has to go somewhere with a name, or the per-project totals silently disagree with the
/// headline total. Named because the drilldown has to recognise it: a session with no project
/// is what this row is made of.
pub const UNATTRIBUTED: &str = "(unattributed)";

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
            .unwrap_or_else(|| UNATTRIBUTED.to_string());
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
        accrue(
            usage,
            &mut acc.totals.cost,
            &mut acc.totals.unpriced_requests,
            &mut acc.totals.quota_requests,
        );
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
        // Counted but kept out of both sides of the ratio: quota-billed work has no per-request
        // price to be missing, so scoring it as a coverage gap reports a deliberate refusal to
        // invent a number as a failure to produce one. Reported separately so it cannot vanish.
        if usage.cost_status.is_quota_billed() {
            coverage.quota_requests += usage.requests;
            continue;
        }
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
        accrue(
            usage,
            &mut entry.cost,
            &mut entry.unpriced_requests,
            &mut entry.quota_requests,
        );
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

/// Usage within a trailing window ending at `now`.
///
/// `now` is passed in rather than read here: this runs on the render path, which must not read
/// the clock, and a caller-supplied instant is also what makes the result testable.
pub fn burn_rate(usages: &[Usage], window_secs: i64, now: i64) -> BurnRate {
    let mut burn = BurnRate {
        window_secs,
        ..Default::default()
    };
    if window_secs <= 0 {
        return burn;
    }
    let cutoff = now.saturating_sub(window_secs);

    for usage in usages {
        // Future-dated rows are excluded rather than clamped in. A clock skew between the
        // machine that wrote the log and this one would otherwise inflate the rate.
        if usage.created <= cutoff || usage.created > now {
            continue;
        }
        burn.requests += usage.requests;
        burn.tokens += usage.total_tokens();
        accrue(
            usage,
            &mut burn.cost,
            &mut burn.unpriced_requests,
            &mut burn.quota_requests,
        );
    }
    burn
}

/// Seconds until `remaining` dollars are spent at this burn rate.
///
/// `None` when the window is too thin to extrapolate from, or nothing is left to spend.
pub fn seconds_to_exhaust(burn: &BurnRate, remaining: f64) -> Option<i64> {
    if !burn.is_projectable() || remaining <= 0.0 {
        return None;
    }
    let per_hour = burn.cost_per_hour();
    if per_hour <= 0.0 {
        return None;
    }
    Some((remaining / per_hour * 3600.0).round() as i64)
}

/// `2h 14m`, `45m`, `<1m`. Coarse on purpose — a projection accurate to the second would imply
/// a precision the underlying rate does not have.
pub fn format_duration(seconds: i64) -> String {
    if seconds < 60 {
        return "<1m".to_string();
    }
    let minutes = seconds / 60;
    let hours = minutes / 60;
    if hours == 0 {
        format!("{minutes}m")
    } else if hours < 24 {
        format!("{}h {:02}m", hours, minutes % 60)
    } else {
        format!("{}d {}h", hours / 24, hours % 24)
    }
}

/// Roll usage up by session, most recently active first.
///
/// Recency order because the question is almost always "what did I just do", and because the
/// list grows without bound — sessions accumulate forever where projects top out in dozens.
pub fn session_totals(usages: &[Usage]) -> Vec<SessionTotals> {
    use std::collections::BTreeMap;

    let mut grouped: BTreeMap<String, SessionTotals> = BTreeMap::new();
    for usage in usages {
        let Some(id) = usage.session_id.as_ref() else {
            continue;
        };
        let entry = grouped.entry(id.clone()).or_insert_with(|| SessionTotals {
            session_id: id.clone(),
            project: usage.project.clone(),
            first_seen: usage.created,
            last_seen: usage.created,
            ..Default::default()
        });
        if usage.created > 0 {
            if entry.first_seen <= 0 || usage.created < entry.first_seen {
                entry.first_seen = usage.created;
            }
            entry.last_seen = entry.last_seen.max(usage.created);
        }
        entry.requests += usage.requests;
        entry.tokens += usage.total_tokens();
        accrue(
            usage,
            &mut entry.cost,
            &mut entry.unpriced_requests,
            &mut entry.quota_requests,
        );
        let model = format!("{}/{}", usage.provider, usage.model);
        if !entry.models.contains(&model) {
            entry.models.push(model);
        }
        // A session can move between projects only if the source is inconsistent; take the
        // first non-empty one rather than letting a later null blank it out.
        if entry.project.is_none() {
            entry.project.clone_from(&usage.project);
        }
    }

    let mut sessions: Vec<SessionTotals> = grouped.into_values().collect();
    sessions.sort_by_key(|s| std::cmp::Reverse(s.last_seen));
    sessions
}

/// Split `width` cells between `weights` in proportion, for the share strip under the tiles.
///
/// Three rules, each of which a plain `width * w / total` breaks. A weight above zero gets at
/// least one cell: rounding a small category down to nothing draws "there was none", which is a
/// number the data does not support. A weight of zero gets none. And the cells sum to `width`
/// exactly, by largest remainder, so the strip neither stops short of the edge nor overruns it.
///
/// Returns an empty vector when there is nothing to draw: no weight at all, or fewer cells than
/// categories that need one. No strip is the honest rendering of both.
pub fn share_cells(weights: &[u64], width: u16) -> Vec<u16> {
    let total: u128 = weights.iter().map(|w| u128::from(*w)).sum();
    let present = weights.iter().filter(|w| **w > 0).count();
    if total == 0 || present > usize::from(width) {
        return Vec::new();
    }
    // One cell each is set aside first, and only the rest is shared out in proportion.
    let spare = u128::from(width) - present as u128;
    let mut cells: Vec<u16> = Vec::with_capacity(weights.len());
    let mut remainders: Vec<(u128, usize)> = Vec::new();
    for (index, weight) in weights.iter().enumerate() {
        if *weight == 0 {
            cells.push(0);
            continue;
        }
        let scaled = spare * u128::from(*weight);
        cells.push(1 + (scaled / total) as u16);
        remainders.push((scaled % total, index));
    }
    let assigned: u32 = cells.iter().map(|c| u32::from(*c)).sum();
    let mut left = u32::from(width) - assigned;
    // Largest remainder first; ties go to the earlier category so the result is deterministic.
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for (_, index) in remainders {
        if left == 0 {
            break;
        }
        cells[index] += 1;
        left -= 1;
    }
    cells
}

/// A share of a whole as a reader sees it: `21%`, `<1%` for a share that is real but rounds to
/// nothing, and `None` for no share at all -- which the caller renders as a dash, not `0%`.
pub fn share_label(part: u64, whole: u64) -> Option<String> {
    if part == 0 || whole == 0 {
        return None;
    }
    let pct = part as f64 / whole as f64 * 100.0;
    Some(if pct < 1.0 {
        "<1%".to_string()
    } else {
        format!("{pct:.0}%")
    })
}
