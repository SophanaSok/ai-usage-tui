//! Routing analytics and derived escalations, recorded and inferred.

use super::*;

#[test]
fn routing_leads_with_cost_per_delivered_result() {
    let mut app = test_app(Vec::new());
    // Passed in the opposite order to the expected ranking, so the assertion below fails if the
    // panel merely echoes its input.
    app.set_routing_for_test(vec![
        routing_agg("junior", "opencode/glm-5.2", 20, 60.00, 5, 15),
        routing_agg("reviewer", "anthropic/claude-opus-5", 12, 41.20, 12, 0),
    ]);
    let rendered = render_routing(&app, 84, 6);
    assert!(rendered.contains("$/SUCCESS"), "{rendered}");
    // $41.20/12 = $3.43 beats $60/5 = $12.00, so the pricier model ranks first.
    let reviewer = rendered.find("reviewer").expect("reviewer row");
    let junior = rendered.find("junior").expect("junior row");
    assert!(
        reviewer < junior,
        "rows are not ranked by cost per success:\n{rendered}"
    );
}

#[test]
fn a_free_model_says_free_rather_than_implying_a_precise_comparison() {
    // $0.0000 for a free model is arithmetically true and analytically empty: the metric cannot
    // discriminate between free models however badly they perform.
    let mut app = test_app(Vec::new());
    app.set_routing_for_test(vec![routing_agg(
        "junior",
        "opencode/free-model",
        20,
        0.0,
        8,
        12,
    )]);
    let rendered = render_routing(&app, 84, 5);
    assert!(rendered.contains("free"), "{rendered}");
    assert!(!rendered.contains("$0.0000"), "{rendered}");
}

#[test]
fn an_uninstrumented_agent_shows_a_dash_not_a_zero_pass_rate() {
    // An agent that never reported a test result must not read as one that fails everything.
    // Both its pass rate and its cost-per-success are unknown, so both render as a dash.
    //
    // Scoped to the row rather than the whole buffer, for two reasons: a genuine zero retry
    // rate is also "0%", and the panel title itself contains an em dash.
    let mut app = test_app(Vec::new());
    app.set_routing_for_test(vec![routing_agg(
        "explorer",
        "opencode/glm-5.2",
        9,
        3.10,
        0,
        0,
    )]);
    let rendered = render_routing(&app, 84, 5);

    let row_start = rendered.find("explorer").expect("the agent row");
    let row_end = rendered[row_start..]
        .find("100.0K")
        .expect("the token column")
        + row_start;
    let row = &rendered[row_start..row_end];

    assert_eq!(
        row.matches('\u{2014}').count(),
        2,
        "expected unknown pass rate and unknown cost-per-success in this row:\n{row}"
    );
}

#[test]
fn the_empty_routing_panel_explains_the_feature_and_how_to_use_it() {
    // This is the state nearly every user sees: routing events come from the user's own
    // harness, so a bare "no events recorded" made the most differentiated thing this project
    // does also its least discoverable.
    let app = test_app(Vec::new());
    let rendered = render_routing(&app, 76, 19);
    assert!(
        rendered.contains("earning its cost"),
        "no explanation of what it answers:\n{rendered}"
    );
    assert!(
        rendered.contains("--record-routing"),
        "no way to enable it:\n{rendered}"
    );
    assert!(
        rendered.contains("routing-analytics.md"),
        "no pointer to the docs:\n{rendered}"
    );
}

#[test]
fn derived_escalations_render_above_the_recorded_routing_table() {
    // The panel is useless to anyone who has not instrumented --record-routing by hand. This
    // block needs no instrumentation, so it is what most users will actually see there.
    let mut app = test_app(Vec::new());
    app.set_escalations_for_test(escalations_for_test(
        30,
        12,
        vec![transition(
            "opencode/glm-5.2",
            "anthropic/claude-opus-5",
            7,
            4.10,
            0,
        )],
    ));
    let rendered = render_routing(&app, 84, 12);
    assert!(
        rendered.contains("40%"),
        "escalation rate missing:\n{rendered}"
    );
    assert!(
        rendered.contains("of 30 sessions"),
        "the denominator must be visible — a rate without one is not a fact:\n{rendered}"
    );
    assert!(
        rendered.contains("glm-5.2 → claude-opus-5"),
        "the transition itself is the finding:\n{rendered}"
    );
    assert!(rendered.contains("$4.10 after"), "{rendered}");
}

#[test]
fn derived_and_recorded_routing_are_labelled_as_different_things() {
    // An inferred transition and a measured pass rate must never share a table. On screen they
    // would be indistinguishable, which is the failure CostStatus exists to prevent.
    let mut app = test_app(Vec::new());
    app.set_escalations_for_test(escalations_for_test(
        10,
        5,
        vec![transition("haiku", "opus", 2, 1.00, 0)],
    ));
    app.set_routing_for_test(vec![routing_agg(
        "reviewer",
        "anthropic/claude-opus-5",
        4,
        2.0,
        4,
        0,
    )]);
    let rendered = render_routing(&app, 84, 16);
    assert!(
        rendered.contains("ESCALATIONS") && rendered.contains("derived from sessions"),
        "the derived block must say it is derived:\n{rendered}"
    );
    assert!(
        rendered.contains("ROUTING") && rendered.contains("$/SUCCESS"),
        "the recorded table must still be present:\n{rendered}"
    );
}

#[test]
fn spend_after_an_escalation_reads_as_a_floor_when_partly_unpriced() {
    let mut app = test_app(Vec::new());
    app.set_escalations_for_test(escalations_for_test(
        4,
        2,
        vec![transition("haiku", "opus", 2, 1.50, 3)],
    ));
    let rendered = render_routing(&app, 84, 10);
    assert!(
        rendered.contains("≥ $1.50 after"),
        "unpriced spend after the move makes the figure a floor, not a total:\n{rendered}"
    );
}

#[test]
fn nothing_derived_leaves_the_routing_panel_as_it_was() {
    // A user with no multi-request sessions must not get an empty block taking up a third of
    // the pane.
    let app = test_app(Vec::new());
    let rendered = render_routing(&app, 84, 12);
    assert!(
        !rendered.contains("ESCALATIONS"),
        "an empty derived block should not be rendered at all:\n{rendered}"
    );
}

#[test]
fn escalating_onto_a_quota_billed_model_reports_no_unpriced_spend() {
    // The call site farthest from the fix. Before it, the escalation block reported quota work
    // as spend it had failed to price, and rendered the total as a floor.
    let now = crate::utils::now();
    let mut opener = usage(None, Some("s1"), Some(0.01), 100);
    opener.model = "claude-haiku-4-5".into();
    opener.created = now;
    let mut escalated = quota_usage(100, now + 10);
    escalated.session_id = Some("s1".into());

    let engine = crate::pricing::PricingEngine::bundled();
    let derived = crate::escalation::derive(&[opener, escalated], |m| engine.input_rate(m));
    assert_eq!(
        derived.transitions.len(),
        1,
        "a cloud model with a table entry can still be ranked"
    );
    assert_eq!(
        derived.transitions[0].unpriced_after, 0,
        "quota-billed spend is not spend we failed to price"
    );
}

#[test]
fn an_escalation_onto_a_subscription_model_is_not_zero_dollars_after() {
    // Before: "$0.00 after" for a session that escalated to Opus on a Max plan.
    let mut app = test_app(Vec::new());
    let mut onto_plan = transition("haiku", "opus", 3, 0.0, 0);
    onto_plan.quota_after = 9;
    app.set_escalations_for_test(escalations_for_test(10, 3, vec![onto_plan]));
    let rendered = render_routing(&app, 84, 12);
    assert!(rendered.contains("on quota after"), "{rendered}");
    assert!(!rendered.contains("$0.00 after"), "{rendered}");
}

/// An agent whose spend cannot be priced must not rank as the cheapest work on the machine.
///
/// This is the bug the provenance counters exist for, and it was two failures stacked. `aggregate`
/// summed `cost.unwrap_or(0.0)`, so an all-unpriced agent arrived with `cost: 0.0`;
/// `cost_per_success` then divided that by its passes and returned `Some(0.0)` rather than "no
/// figure". The panel's default sort is `$/SUCCESS` **ascending**, and `cost_order` holds only a
/// `None` at the end — so the row it was written to protect against sailed past it into first
/// place, rendered green as `free`.
#[test]
fn an_unpriced_agent_does_not_rank_as_the_cheapest_per_success() {
    let mut app = test_app(Vec::new());
    // Deliberately ordered with the unpriced agent first, so a panel that merely echoes its
    // input fails this.
    app.set_routing_for_test(vec![
        routing_agg_with_gaps("unpriced", "some/unknown-model", 4, 0.0, 4, 0, 4, 0),
        routing_agg("priced", "anthropic/claude-opus-5", 4, 4.00, 4, 0),
    ]);
    // `set_routing_for_test` applies the sorts, standing in for `recompute`.
    let rendered = render_routing(&app, 84, 6);

    let priced = rendered.find("priced").expect("priced row");
    let unpriced = rendered.find("unpriced").expect("unpriced row");
    assert!(
        priced < unpriced,
        "an agent with no defensible cost outranked one at $1.00 per success:\n{rendered}"
    );
    assert!(
        !rendered.contains("free"),
        "unpriced work rendered as free:\n{rendered}"
    );
    assert!(
        rendered.contains("unpriced  "),
        "expected the cell to say so plainly; a floor of $0.0000 is not a figure:\n{rendered}"
    );
}

/// Subscription work is the common case of the above, and the one a Max account hits on day one.
///
/// Opus on a plan arrives as `cost: null, cost_status: quota`. It is real spend with no
/// per-request figure — not free, and not zero.
#[test]
fn subscription_billed_work_reads_as_on_quota_not_free() {
    let mut app = test_app(Vec::new());
    app.set_routing_for_test(vec![routing_agg_with_gaps(
        "architect",
        "anthropic/claude-opus-5",
        6,
        0.0,
        6,
        0,
        0,
        6,
    )]);
    let rendered = render_routing(&app, 84, 5);
    assert!(rendered.contains("on quota"), "{rendered}");
    assert!(!rendered.contains("free"), "{rendered}");
    assert!(!rendered.contains("$0.0000"), "{rendered}");
}

/// Partly priced spend reads as a floor, in the vocabulary the escalations block already uses.
#[test]
fn partly_unpriced_spend_per_success_reads_as_a_floor() {
    let mut app = test_app(Vec::new());
    app.set_routing_for_test(vec![routing_agg_with_gaps(
        "junior",
        "openrouter/mystery",
        10,
        2.00,
        4,
        0,
        3,
        0,
    )]);
    let rendered = render_routing(&app, 84, 5);
    assert!(
        rendered.contains("≥ $0.5000"),
        "expected a floor, got:\n{rendered}"
    );
}
