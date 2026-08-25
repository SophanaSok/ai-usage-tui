use crate::model::{CostStatus, RoutingAggregates, RoutingEvent};
use std::collections::BTreeMap;

/// What `cost_per_success` is standing on.
///
/// The cell and the sort both read this, so they cannot disagree about the same row — the reason
/// `theme::cost_sort_key` lives beside `theme::cost_display` one level up. A COST column that
/// sorts by a number it never showed is how `ON QUOTA` ends up ranked as the cheapest work on the
/// machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostBasis {
    /// Nothing passed, so there is no denominator. Not zero — unknown.
    NoSuccesses,
    /// Every contributing task was free or local. Genuinely zero.
    Free,
    /// Every contributing task carried a price.
    Exact,
    /// Some tasks were billed against a plan; the rest are priced.
    PlusQuota,
    /// Every contributing task was billed against a plan. Real spend, no per-request figure.
    Quota,
    /// Nothing was priced and something should have been. There is no figure at all — a floor of
    /// `$0.0000` is arithmetically true and says nothing, which is the failure one level up.
    Unpriced,
    /// Some spend was priced and some was not, so the figure is a floor.
    Floor,
}

pub fn aggregate(events: &[RoutingEvent]) -> Vec<RoutingAggregates> {
    let mut map: BTreeMap<(String, String, String), RoutingAggregates> = BTreeMap::new();

    for event in events {
        let key = (
            event.agent.clone(),
            event.model.clone(),
            event.provider.clone(),
        );
        let entry = map.entry(key).or_insert_with(|| RoutingAggregates {
            agent: event.agent.clone(),
            model: event.model.clone(),
            provider: event.provider.clone(),
            // Every counter starts at Default, so a counter added later cannot be left out of
            // the accumulator and silently read as zero for every row.
            ..Default::default()
        });

        entry.tasks += 1;
        entry.tokens += event.tokens;
        // Classified, not unwrapped. `unwrap_or(0.0)` charged $0 for work whose rate is unknown
        // and for work billed against a plan, and then the panel divided by passes and called the
        // result free. This is the same split `escalation::derive` makes, and it is matched
        // exhaustively on purpose: a new `CostStatus` must not be able to acquire a cost of zero
        // by falling through a wildcard.
        match event.cost_status {
            CostStatus::ProviderReported | CostStatus::Calculated | CostStatus::Estimated => {
                match event.cost {
                    Some(cost) => {
                        entry.cost += cost;
                        entry.priced_tasks += 1;
                    }
                    // A status that promises a figure, with no figure. Trusting the status over
                    // the missing value is what produced the bug in the first place.
                    None => entry.unpriced_tasks += 1,
                }
            }
            CostStatus::Quota => entry.quota_tasks += 1,
            CostStatus::Unavailable => entry.unpriced_tasks += 1,
            CostStatus::Free | CostStatus::Local => entry.free_tasks += 1,
        }
        entry.retries += event.retries;
        entry.escalations += event.escalations;
        if let Some(result) = event.test_result {
            if result {
                entry.test_passes += 1;
            } else {
                entry.test_failures += 1;
            }
        }
        entry.review_defects += event.review_defects;
    }

    let mut result: Vec<RoutingAggregates> = map.into_values().collect();
    result.sort_by_key(|a| std::cmp::Reverse(a.tokens));
    result
}

pub fn retry_rate(agg: &RoutingAggregates) -> f64 {
    if agg.tasks == 0 {
        0.0
    } else {
        agg.retries as f64 / agg.tasks as f64 * 100.0
    }
}

pub fn escalation_rate(agg: &RoutingAggregates) -> f64 {
    if agg.tasks == 0 {
        0.0
    } else {
        agg.escalations as f64 / agg.tasks as f64 * 100.0
    }
}

/// Share of tasks whose recorded test result passed.
///
/// `None` when no task recorded a test result at all. A rate computed over zero observations
/// is not 0% — it is unknown, and rendering it as 0% would make an uninstrumented agent look
/// like a failing one.
pub fn success_rate(agg: &RoutingAggregates) -> Option<f64> {
    let observed = agg.test_passes + agg.test_failures;
    if observed == 0 {
        return None;
    }
    Some(agg.test_passes as f64 / observed as f64 * 100.0)
}

/// Dollars spent per task that passed its tests.
///
/// This is the number the routing panel exists for: it is what makes "is the expensive model
/// worth it?" answerable rather than a matter of taste. A model at twice the token price that
/// passes first time can be cheaper per delivered result than a cheap one that needs three
/// attempts.
///
/// `None` when nothing passed — dividing by zero successes is either infinite or, rendered
/// carelessly, $0.00.
pub fn cost_per_success(agg: &RoutingAggregates) -> Option<f64> {
    if agg.test_passes == 0 {
        return None;
    }
    Some(agg.cost / agg.test_passes as f64)
}

/// What that figure is standing on, from the counters `aggregate` kept.
///
/// The order of the arms is the order of severity: an unpriced task makes the figure a floor no
/// matter what else is true, because the missing rate could be any size.
pub fn cost_per_success_basis(agg: &RoutingAggregates) -> CostBasis {
    if agg.test_passes == 0 {
        return CostBasis::NoSuccesses;
    }
    match (agg.unpriced_tasks, agg.quota_tasks, agg.priced_tasks) {
        (0, 0, 0) => CostBasis::Free,
        (0, 0, _) => CostBasis::Exact,
        (0, _, 0) => CostBasis::Quota,
        (0, _, _) => CostBasis::PlusQuota,
        // Nothing priced: the floor is zero, and "at least nothing" is not a figure worth
        // printing beside one that is real.
        (_, _, 0) => CostBasis::Unpriced,
        _ => CostBasis::Floor,
    }
}

/// How a `$/SUCCESS` column sorts, where `None` means "not a point on this scale".
///
/// Only an exact figure and a genuine zero are comparable. A floor is not: an agent at
/// `≥ $0.01` with nine unpriced tasks may be the most expensive on the machine, and ordering it
/// as though the floor were the total is precisely the claim this whole change removes. `None`
/// sorts to one end in both directions via `App::cost_order`, exactly as an unknown row cost
/// does in the model table.
pub fn cost_per_success_sort_key(agg: &RoutingAggregates) -> Option<f64> {
    match cost_per_success_basis(agg) {
        CostBasis::Free => Some(0.0),
        CostBasis::Exact => cost_per_success(agg),
        CostBasis::NoSuccesses
        | CostBasis::Quota
        | CostBasis::PlusQuota
        | CostBasis::Unpriced
        | CostBasis::Floor => None,
    }
}

/// The basis as a stable string, for `--routing-json`.
///
/// Its own function rather than `Debug`, because a derive is not a contract: renaming a variant
/// would silently change an exported field that scripts key on.
pub fn cost_basis_label(agg: &RoutingAggregates) -> &'static str {
    match cost_per_success_basis(agg) {
        CostBasis::NoSuccesses => "no_successes",
        CostBasis::Free => "free",
        CostBasis::Exact => "exact",
        CostBasis::PlusQuota => "plus_quota",
        CostBasis::Quota => "quota",
        CostBasis::Unpriced => "unpriced",
        CostBasis::Floor => "floor",
    }
}

pub fn defect_rate(agg: &RoutingAggregates) -> f64 {
    if agg.tasks == 0 {
        0.0
    } else {
        agg.review_defects as f64 / agg.tasks as f64 * 100.0
    }
}

pub fn load_routing_events(path: &std::path::Path) -> anyhow::Result<Vec<RoutingEvent>> {
    crate::collector::journal::load_routing(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agg(tasks: u64, cost: f64, passes: u32, failures: u32) -> RoutingAggregates {
        RoutingAggregates {
            agent: "a".into(),
            model: "m".into(),
            provider: "p".into(),
            tasks,
            cost,
            test_passes: passes,
            test_failures: failures,
            ..Default::default()
        }
    }

    #[test]
    fn an_uninstrumented_agent_has_an_unknown_success_rate_not_a_zero_one() {
        // Rendering "0%" for an agent that simply never reported a test result would make it
        // look like it fails everything.
        assert_eq!(success_rate(&agg(5, 1.0, 0, 0)), None);
        assert_eq!(success_rate(&agg(5, 1.0, 3, 1)), Some(75.0));
    }

    #[test]
    fn cost_per_success_answers_whether_the_expensive_model_earned_it() {
        // The whole point: a model at 4x the cost that passes every time beats a cheap one
        // that needs several attempts.
        let pricey = agg(4, 40.0, 4, 0);
        let cheap = agg(4, 12.0, 1, 3);
        let pricey_per_win = cost_per_success(&pricey).expect("has passes");
        let cheap_per_win = cost_per_success(&cheap).expect("has passes");
        assert!((pricey_per_win - 10.0).abs() < 1e-9);
        assert!((cheap_per_win - 12.0).abs() < 1e-9);
        assert!(
            pricey_per_win < cheap_per_win,
            "the pricier model is cheaper per delivered result"
        );
    }

    #[test]
    fn nothing_passing_yields_no_cost_per_success_rather_than_zero() {
        assert_eq!(cost_per_success(&agg(3, 9.0, 0, 3)), None);
    }
    use crate::model::{Category, CostStatus, RoutingEvent};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[allow(clippy::too_many_arguments)]
    fn make_event(
        agent: &str,
        model: &str,
        provider: &str,
        tokens: u64,
        cost: Option<f64>,
        retries: u32,
        escalations: u32,
        test_result: Option<bool>,
        review_defects: u32,
    ) -> RoutingEvent {
        RoutingEvent {
            task: "test".to_string(),
            phase: "test".to_string(),
            agent: agent.to_string(),
            model: model.to_string(),
            provider: provider.to_string(),
            category: Category::Unknown,
            requests: 1,
            tokens,
            cost,
            // Coherent with `cost`, which it was not: it said `Unavailable` while carrying a
            // figure. The old `unwrap_or(0.0)` ignored the status and summed the number anyway,
            // so the contradiction never showed. Trusting the number over the status is exactly
            // the laundering that let unpriced work reach the panel as $0.00.
            cost_status: match cost {
                Some(_) => CostStatus::ProviderReported,
                None => CostStatus::Unavailable,
            },
            retries,
            escalations,
            test_result,
            review_defects,
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        }
    }

    #[test]
    fn aggregate_groups_by_agent_model() {
        let events = vec![
            make_event(
                "agent1",
                "model1",
                "provider1",
                100,
                Some(0.01),
                0,
                0,
                None,
                0,
            ),
            make_event(
                "agent1",
                "model1",
                "provider1",
                200,
                Some(0.02),
                1,
                0,
                Some(true),
                0,
            ),
            make_event(
                "agent2",
                "model2",
                "provider2",
                300,
                Some(0.03),
                0,
                1,
                Some(false),
                1,
            ),
        ];
        let result = aggregate(&events);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn retry_rate_handles_zero_tasks() {
        let agg = RoutingAggregates {
            retries: 5,
            ..Default::default()
        };
        assert_eq!(retry_rate(&agg), 0.0);
    }

    #[test]
    fn aggregate_sums_tokens_and_cost() {
        let events = vec![
            make_event(
                "agent1",
                "model1",
                "provider1",
                100,
                Some(0.01),
                0,
                0,
                None,
                0,
            ),
            make_event(
                "agent1",
                "model1",
                "provider1",
                200,
                Some(0.02),
                0,
                0,
                None,
                0,
            ),
        ];
        let result = aggregate(&events);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].tokens, 300);
        assert!((result[0].cost - 0.03).abs() < f64::EPSILON);
    }
}
