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
    // An agent that never reported a test result must not read as one that fails everything —
    // and one that never reported a retry, escalation or defect count must not read as one that
    // never needed a second attempt. All five figures are unknown, so all five render as a dash.
    // RETRY, ESC and DEFECT used to render `0%` here, indistinguishable from a clean run.
    //
    // Scoped to the row rather than the whole buffer: the panel title itself contains an em dash.
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
    let row = routing_row(&rendered, "explorer");

    assert_eq!(
        row.matches('\u{2014}').count(),
        5,
        "expected every unmeasured figure in this row to be a dash:\n{row}"
    );
    assert!(
        !row.contains("0%"),
        "an unreported count rendered as 0%:\n{row}"
    );
}

#[test]
fn a_reported_zero_retry_rate_reads_as_zero_not_unknown() {
    // The anti-test: an agent whose harness counted retries on every task and found none is a
    // genuine 0%, and must not be hidden behind a dash along with the agents that never said.
    //
    // Three of four passed, so PASS reads `75%` and cannot satisfy the `0%` assertion for it.
    let mut app = test_app(Vec::new());
    let mut clean = routing_agg("careful", "anthropic/claude-opus-5", 4, 4.00, 3, 1);
    clean.retries = crate::model::ObservedCount {
        observed: 4,
        ..Default::default()
    };
    app.set_routing_for_test(vec![clean]);
    let rendered = render_routing(&app, 84, 5);
    let row = routing_row(&rendered, "careful");
    assert!(row.contains("0%"), "a measured zero was not shown:\n{row}");
    assert_eq!(
        row.matches('\u{2014}').count(),
        2,
        "only ESC and DEFECT are unmeasured here:\n{row}"
    );
}

#[test]
fn sorting_by_a_rate_column_holds_unmeasured_agents_at_the_end_both_ways() {
    // RETRY sorted by the raw sum, so an agent that never reported retries sorted as the best on
    // the machine ascending and the worst descending. Unmeasured is neither; `cost_order`
    // already knew how to place it and the rate columns now use it.
    //
    // Once per rate column, each driven by its own field, so a column wired to the wrong
    // counter fails here rather than passing on RETRY's behalf.
    use crate::model::{ObservedCount, RoutingAggregates};
    use crate::ui::app::{Panel, Sort};
    let counted = |observed, affected| ObservedCount {
        observed,
        affected,
        total: affected as u32,
    };
    type Field = fn(&mut RoutingAggregates) -> &mut ObservedCount;
    let columns: [(usize, Field); 3] = [
        (4, |a| &mut a.retries),
        (5, |a| &mut a.escalations),
        (6, |a| &mut a.review_defects),
    ];
    for (column, field) in columns {
        let mut half = routing_agg("half", "m", 4, 4.0, 4, 0);
        *field(&mut half) = counted(4, 2);
        let mut clean = routing_agg("clean", "m", 4, 4.0, 4, 0);
        *field(&mut clean) = counted(4, 0);
        let silent = routing_agg("silent", "m", 4, 4.0, 4, 0);

        for (descending, expected) in [
            (false, ["clean", "half", "silent"]),
            (true, ["half", "clean", "silent"]),
        ] {
            let mut app = test_app(Vec::new());
            app.sorts
                .insert(Panel::Routing, Sort { column, descending });
            app.set_routing_for_test(vec![silent.clone(), half.clone(), clean.clone()]);
            let rendered = render_routing(&app, 84, 7);
            let positions: Vec<usize> = expected
                .iter()
                .map(|agent| rendered.find(agent).expect(agent))
                .collect();
            assert!(
                positions.windows(2).all(|w| w[0] < w[1]),
                "column {column} descending={descending}: expected {expected:?} in order:\n{rendered}"
            );
            // And a measured rate renders as its figure, not only as "not a dash".
            assert!(
                routing_row(&rendered, "half").contains("50%"),
                "column {column}: the measured rate was not shown:\n{rendered}"
            );
        }
    }
}

#[test]
fn sorting_by_pass_orders_by_the_rate_the_column_shows() {
    // PASS shows `success_rate` but sorted by the raw `test_passes` count, which ranked one-of-one
    // at 100% below five-of-ten at 50%. Same shape as the sessions STARTED/`last_seen` bug.
    use crate::ui::app::{Panel, Sort};
    let mut app = test_app(Vec::new());
    app.sorts.insert(
        Panel::Routing,
        Sort {
            column: 3,
            descending: true,
        },
    );
    app.set_routing_for_test(vec![
        routing_agg("mostly", "m", 10, 10.0, 5, 5),
        routing_agg("always", "m", 1, 1.0, 1, 0),
    ]);
    let rendered = render_routing(&app, 84, 6);
    let always = rendered.find("always").expect("always row");
    let mostly = rendered.find("mostly").expect("mostly row");
    assert!(
        always < mostly,
        "100% should rank above 50% when sorting PASS descending:\n{rendered}"
    );
}

#[test]
fn the_selected_routing_row_is_highlighted_scrolled_into_view_and_reachable() {
    // Same defect as the projects table, plus one of its own: `visible_rows` clamped the routing
    // cursor to the *model* table, so with no models loaded it could not leave row zero.
    use crate::ui::app::Panel;
    let mut app = test_app(Vec::new());
    app.toggle_panel(Panel::Routing);
    app.set_routing_for_test(
        (0..12)
            .map(|i| routing_agg(&format!("agent{i:02}"), "m", 4, (i + 1) as f64, 4, 0))
            .collect(),
    );
    assert_eq!(
        app.visible_rows(),
        12,
        "the cursor is bounded by the routing table"
    );
    app.selected = 11;

    let buffer = render_panel_buffer(84, 6, |frame, area| {
        crate::ui::panels::routing::draw_routing(frame, area, &app)
    });
    let (row, highlighted) = find_row(&buffer, "agent11");
    assert!(
        highlighted,
        "the selected aggregate is not highlighted:\n{row}"
    );
}

/// The default sort survives a refresh.
///
/// `refresh` read the routing journal *after* `recompute`, which is where the panel is sorted,
/// so every refresh left the table in `aggregate`'s token order until the next key press. The
/// other routing tests inject aggregates through `set_routing_for_test`, which sorts, and so
/// never saw it; this one goes through `refresh` with a real journal.
#[test]
fn a_refresh_leaves_the_routing_panel_in_its_default_order() {
    use crate::collector::journal::record_routing_event;
    use crate::collector::SourceRoots;
    use serde_json::json;

    let dir = std::env::temp_dir().join(format!(
        "ai-usage-tui-routing-refresh-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let journal = dir.join("usage.db");
    // The pricier-per-success agent has the most tokens, so token order and cost order differ.
    for (agent, tokens, cost) in [("pricey", 500_000, 8.0), ("cheap", 1_000, 0.5)] {
        record_routing_event(
            &journal,
            &json!({"agent": agent, "model": "m", "task": agent, "tokens": tokens, "cost": cost, "test_result": true}),
        )
        .expect("record");
    }

    let nowhere = |name: &str| Some(PathBuf::from(format!("/nonexistent/{name}")));
    let mut app = test_app(Vec::new());
    app.roots = SourceRoots {
        db_path: nowhere("opencode.db"),
        claude_dir: nowhere("claude"),
        codex_dir: nowhere("codex"),
        gemini_dir: nowhere("gemini"),
        omarchy_dir: nowhere("omarchy"),
        ..SourceRoots::new(journal)
    };
    app.refresh();

    let order: Vec<&str> = app.routing().iter().map(|a| a.agent.as_str()).collect();
    assert_eq!(
        order,
        ["cheap", "pricey"],
        "cheapest per delivered result should lead after a refresh"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The text of one agent's row, from its name to its token count.
fn routing_row<'a>(rendered: &'a str, agent: &str) -> &'a str {
    let start = rendered.find(agent).expect("the agent row");
    let end = rendered[start..].find("100.0K").expect("the token column") + start;
    &rendered[start..end]
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
