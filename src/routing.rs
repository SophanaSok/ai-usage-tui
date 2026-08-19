use crate::model::{RoutingAggregates, RoutingEvent};
use std::collections::BTreeMap;

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
            tasks: 0,
            tokens: 0,
            cost: 0.0,
            retries: 0,
            escalations: 0,
            test_passes: 0,
            test_failures: 0,
            review_defects: 0,
        });

        entry.tasks += 1;
        entry.tokens += event.tokens;
        entry.cost += event.cost.unwrap_or(0.0);
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
            cost_status: CostStatus::Unavailable,
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
            agent: "a".to_string(),
            model: "m".to_string(),
            provider: "p".to_string(),
            tasks: 0,
            tokens: 0,
            cost: 0.0,
            retries: 5,
            escalations: 0,
            test_passes: 0,
            test_failures: 0,
            review_defects: 0,
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
