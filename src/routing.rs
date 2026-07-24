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
