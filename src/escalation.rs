//! Model escalation, derived from usage that was already collected.
//!
//! The routing panel answers "is the expensive model earning its cost?", but it can only answer
//! it for work someone instrumented by hand with `--record-routing`. Nothing infers a test
//! result, and nothing should: a pass rate this tool invented would be indistinguishable on
//! screen from one an agent harness actually measured.
//!
//! One part of the same question *is* directly observable in data already collected. When a
//! session moves from a cheaper model to a more expensive one, that transition happened —
//! either a human or a harness decided the cheap model was not getting there. Counting those
//! transitions, and the spend that followed each one, needs no instrumentation at all.
//!
//! What this deliberately does not claim:
//!
//! - **Not a routing event.** Derived transitions are never merged into recorded routing
//!   aggregates. Measured and inferred data stay in separate structures so the UI cannot blur
//!   them, which is the same reason `CostStatus` exists.
//! - **Not a verdict.** An escalation is not a failure of the cheaper model. A session may
//!   escalate because the task genuinely got harder. What the numbers support is "this happens
//!   N times out of M sessions and costs this much", not "the cheap model wasted your money".
//! - **Not a guess when the rates are unknown.** Ordering two models requires a price for both.
//!   Absent either, the transition is counted as unclassified rather than assumed flat.
//!
//! Counting is per *session*, not per switch, and this is the whole difficulty. Sessions
//! interleave models rather than stepping up once — a real session in testing switched models
//! 20 times, 10 of them upward. Counting each switch and attributing the spend that followed it
//! reported **$233 of escalated spend for a $29 session**, because the same tail was summed ten
//! times over. A session is characterised once: the model it opened with, the priciest model it
//! used afterwards, and the spend on everything pricier than the opener. Nothing is counted
//! twice, and the reported total can never exceed the session.

use std::collections::BTreeMap;

use crate::model::Usage;

/// Sessions that opened on `from` and went on to use `to`, the priciest model they reached.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transition {
    /// The model the session opened with.
    pub from: String,
    /// The priciest model the session used afterwards.
    pub to: String,
    /// How many sessions have this shape. Deliberately sessions and not switches: a session
    /// that bounces between two models twenty times made one routing decision worth reporting,
    /// not twenty.
    pub sessions: u64,
    /// Spend in those sessions on any model pricier than the one they opened with. Bounded by
    /// the sessions' own cost, because each request is counted at most once.
    pub cost_after: f64,
    /// Requests on the pricier models that should carry a price but do not, making `cost_after`
    /// a floor rather than a total.
    pub unpriced_after: u64,
}

/// What could be derived about escalation across the sessions in range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Escalations {
    /// Sessions with enough information to look at: a session id and more than one request.
    pub sessions_examined: u64,
    /// Sessions that used a model pricier than the one they opened with.
    pub sessions_escalated: u64,
    /// Model changes that could not be ordered because a rate was missing on one side. Reported
    /// rather than dropped, so a low escalation count is distinguishable from a blind one.
    pub unclassified_changes: u64,
    /// Most sessions first, ties broken by spend.
    pub transitions: Vec<Transition>,
}

impl Escalations {
    /// Share of examined sessions that escalated. `None` when nothing was examined — a rate
    /// over zero sessions is not a fact about anything.
    pub fn rate(&self) -> Option<f64> {
        if self.sessions_examined == 0 {
            return None;
        }
        Some(self.sessions_escalated as f64 / self.sessions_examined as f64 * 100.0)
    }

    pub fn is_empty(&self) -> bool {
        self.sessions_examined == 0
    }
}

/// Derive escalations from usage, ranking models with `rate_of`.
///
/// `rate_of` returns a model's list input rate, or `None` when the model cannot be priced. It
/// is passed in rather than reached for so this stays pure — the caller owns the pricing table,
/// and tests can order two models without one.
pub fn derive(usages: &[Usage], rate_of: impl Fn(&str) -> Option<f64>) -> Escalations {
    let mut by_session: BTreeMap<&str, Vec<&Usage>> = BTreeMap::new();
    for usage in usages {
        // A row with no session id cannot be placed in a sequence, so it cannot show a
        // transition. Silently lumping those together would invent adjacency that never
        // existed.
        if let Some(id) = usage.session_id.as_deref() {
            by_session.entry(id).or_default().push(usage);
        }
    }

    let mut result = Escalations::default();
    let mut pairs: BTreeMap<(String, String), Transition> = BTreeMap::new();

    for (_, mut rows) in by_session {
        // Order within a session is the whole signal. Ties on the timestamp keep their
        // collection order, which is the source's own order.
        rows.sort_by_key(|usage| usage.created);
        if rows.len() < 2 {
            continue;
        }
        result.sessions_examined += 1;

        let opening = &rows[0].model;
        let Some(opening_rate) = rate_of(opening) else {
            // The session cannot be characterised at all without a price for its opener.
            // Counting the changes inside it as unrankable is more honest than dropping it.
            result.unclassified_changes +=
                rows[1..].iter().filter(|row| &row.model != opening).count() as u64;
            continue;
        };

        // One pass over the rest of the session: the priciest model reached, and the spend on
        // everything above the opening rate. Each request contributes once, so the total is
        // bounded by the session's own cost.
        let mut priciest: Option<(&str, f64)> = None;
        let mut cost_after = 0.0;
        let mut unpriced_after = 0;
        for row in &rows[1..] {
            if row.model == *opening {
                continue;
            }
            let Some(rate) = rate_of(&row.model) else {
                result.unclassified_changes += 1;
                continue;
            };
            if rate <= opening_rate {
                continue;
            }
            if priciest.is_none_or(|(_, best)| rate > best) {
                priciest = Some((&row.model, rate));
            }
            if row.cost_status.is_billable() {
                cost_after += row.cost.unwrap_or(0.0);
            } else if row.cost_status.needs_price() {
                unpriced_after += row.requests;
            }
        }

        let Some((to, _)) = priciest else { continue };
        result.sessions_escalated += 1;

        let entry = pairs
            .entry((opening.clone(), to.to_string()))
            .or_insert_with(|| Transition {
                from: opening.clone(),
                to: to.to_string(),
                ..Default::default()
            });
        entry.sessions += 1;
        entry.cost_after += cost_after;
        entry.unpriced_after += unpriced_after;
    }

    result.transitions = pairs.into_values().collect();
    result.transitions.sort_by(|a, b| {
        b.sessions
            .cmp(&a.sessions)
            .then(b.cost_after.total_cmp(&a.cost_after))
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CostStatus;

    fn rate_of(model: &str) -> Option<f64> {
        match model {
            "haiku" => Some(1.0),
            "sonnet" => Some(3.0),
            "opus" => Some(5.0),
            _ => None,
        }
    }

    fn usage(session: &str, model: &str, created: i64, cost: Option<f64>) -> Usage {
        Usage {
            session_id: Some(session.to_string()),
            model: model.to_string(),
            requests: 1,
            created,
            cost,
            cost_status: match cost {
                Some(_) => CostStatus::Calculated,
                None => CostStatus::Unavailable,
            },
            ..Default::default()
        }
    }

    #[test]
    fn a_move_to_a_pricier_model_is_an_escalation() {
        let rows = vec![
            usage("s1", "haiku", 10, Some(0.01)),
            usage("s1", "opus", 20, Some(0.50)),
        ];
        let derived = derive(&rows, rate_of);
        assert_eq!(derived.sessions_examined, 1);
        assert_eq!(derived.sessions_escalated, 1);
        assert_eq!(derived.transitions.len(), 1);
        assert_eq!(derived.transitions[0].from, "haiku");
        assert_eq!(derived.transitions[0].to, "opus");
        assert!((derived.transitions[0].cost_after - 0.50).abs() < 1e-9);
    }

    #[test]
    fn dropping_to_a_cheaper_model_is_not_an_escalation() {
        // The opposite move is a de-escalation and says nothing about the expensive model
        // failing. Counting any model change would have made this look identical.
        let rows = vec![
            usage("s1", "opus", 10, Some(0.50)),
            usage("s1", "haiku", 20, Some(0.01)),
        ];
        let derived = derive(&rows, rate_of);
        assert_eq!(derived.sessions_escalated, 0);
        assert!(derived.transitions.is_empty());
    }

    #[test]
    fn out_of_order_rows_are_sequenced_by_timestamp() {
        // Sources do not promise ordering, and reading the collection order as the session
        // order would turn this escalation into a de-escalation.
        let rows = vec![
            usage("s1", "opus", 99, Some(0.50)),
            usage("s1", "haiku", 10, Some(0.01)),
        ];
        assert_eq!(derive(&rows, rate_of).sessions_escalated, 1);
    }

    #[test]
    fn an_unpriceable_model_yields_an_unclassified_change_not_a_silent_drop() {
        let rows = vec![
            usage("s1", "haiku", 10, Some(0.01)),
            usage("s1", "mystery", 20, Some(0.50)),
        ];
        let derived = derive(&rows, rate_of);
        assert_eq!(derived.sessions_escalated, 0);
        assert_eq!(
            derived.unclassified_changes, 1,
            "a change that could not be ordered must be reported, or a low escalation count is \
             indistinguishable from a blind one"
        );
    }

    #[test]
    fn spend_after_the_move_covers_every_pricier_request_in_the_session() {
        // Attributing only the next request would understate a move that redirected everything
        // that followed.
        let rows = vec![
            usage("s1", "haiku", 10, Some(0.01)),
            usage("s1", "opus", 20, Some(0.50)),
            usage("s1", "opus", 30, Some(0.25)),
        ];
        let derived = derive(&rows, rate_of);
        assert!((derived.transitions[0].cost_after - 0.75).abs() < 1e-9);
    }

    #[test]
    fn unpriced_requests_after_the_move_make_the_cost_a_floor() {
        let rows = vec![
            usage("s1", "haiku", 10, Some(0.01)),
            usage("s1", "opus", 20, Some(0.50)),
            usage("s1", "opus", 30, None),
        ];
        let derived = derive(&rows, rate_of);
        assert_eq!(derived.transitions[0].unpriced_after, 1);
    }

    #[test]
    fn a_single_request_session_is_not_examined() {
        // It could never show a transition, so counting it would dilute the escalation rate
        // with sessions that were never candidates.
        let rows = vec![usage("s1", "haiku", 10, Some(0.01))];
        let derived = derive(&rows, rate_of);
        assert_eq!(derived.sessions_examined, 0);
        assert_eq!(derived.rate(), None);
    }

    #[test]
    fn rows_without_a_session_id_are_ignored_rather_than_pooled() {
        // Pooling them would invent adjacency between unrelated requests.
        let rows = vec![
            Usage {
                model: "haiku".into(),
                created: 10,
                ..Default::default()
            },
            Usage {
                model: "opus".into(),
                created: 20,
                ..Default::default()
            },
        ];
        assert_eq!(derive(&rows, rate_of), Escalations::default());
    }

    #[test]
    fn an_interleaved_session_is_counted_once_and_its_spend_is_not_multiplied() {
        // Found against real data, where a session switched models 20 times, 10 of them
        // upward. Counting each switch and summing the spend that followed it reported $233 of
        // escalated spend for a $29 session. A session is one routing decision, and the
        // reported cost can never exceed what the session actually cost.
        let mut rows = Vec::new();
        let mut clock = 0;
        for _ in 0..10 {
            clock += 1;
            rows.push(usage("s1", "sonnet", clock, Some(1.00)));
            clock += 1;
            rows.push(usage("s1", "opus", clock, Some(1.00)));
        }
        let session_cost: f64 = 20.0;

        let derived = derive(&rows, rate_of);
        assert_eq!(derived.sessions_escalated, 1, "one session, one escalation");
        assert_eq!(derived.transitions.len(), 1);
        assert_eq!(derived.transitions[0].sessions, 1);
        assert!(
            derived.transitions[0].cost_after <= session_cost,
            "escalated spend of ${} exceeds the ${} the session cost — the tail is being \
             summed once per switch",
            derived.transitions[0].cost_after,
            session_cost
        );
        // Exactly the ten opus requests, each counted once.
        assert!((derived.transitions[0].cost_after - 10.0).abs() < 1e-9);
    }

    #[test]
    fn the_priciest_model_reached_is_the_one_reported() {
        // A session that touches sonnet and then opus escalated to opus. Reporting the first
        // step up would understate where the money went.
        let rows = vec![
            usage("s1", "haiku", 10, Some(0.01)),
            usage("s1", "sonnet", 20, Some(0.10)),
            usage("s1", "opus", 30, Some(0.50)),
        ];
        let derived = derive(&rows, rate_of);
        assert_eq!(derived.transitions[0].to, "opus");
        // Everything above the opening rate, counted once each.
        assert!((derived.transitions[0].cost_after - 0.60).abs() < 1e-9);
    }

    #[test]
    fn returning_to_the_opening_model_does_not_re_escalate() {
        let rows = vec![
            usage("s1", "haiku", 10, Some(0.01)),
            usage("s1", "opus", 20, Some(0.50)),
            usage("s1", "haiku", 30, Some(0.01)),
            usage("s1", "opus", 40, Some(0.50)),
        ];
        let derived = derive(&rows, rate_of);
        assert_eq!(derived.sessions_escalated, 1);
        assert_eq!(derived.transitions[0].sessions, 1);
        assert!((derived.transitions[0].cost_after - 1.00).abs() < 1e-9);
    }

    #[test]
    fn an_unpriceable_opening_model_leaves_the_session_unclassified() {
        // Without a price for the opener nothing can be ranked against it, so the session is
        // reported as unranked rather than quietly treated as never escalating.
        let rows = vec![
            usage("s1", "mystery", 10, Some(0.01)),
            usage("s1", "opus", 20, Some(0.50)),
        ];
        let derived = derive(&rows, rate_of);
        assert_eq!(derived.sessions_escalated, 0);
        assert_eq!(derived.unclassified_changes, 1);
    }

    #[test]
    fn the_escalation_rate_is_over_sessions_that_could_have_escalated() {
        let rows = vec![
            usage("s1", "haiku", 10, Some(0.01)),
            usage("s1", "opus", 20, Some(0.50)),
            usage("s2", "haiku", 10, Some(0.01)),
            usage("s2", "haiku", 20, Some(0.01)),
        ];
        let derived = derive(&rows, rate_of);
        assert_eq!(derived.sessions_examined, 2);
        assert_eq!(derived.rate(), Some(50.0));
    }
}
