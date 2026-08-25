//! Dashboard state: which panel is showing, what the model table renders, and where the
//! selection is allowed to go.

use super::*;

#[test]
fn only_one_panel_can_be_active() {
    // Two independent booleans let "budgets" and "routing" both be on, with one silently
    // winning the draw dispatch.
    let mut app = test_app(Vec::new());
    app.toggle_panel(Panel::Budgets);
    assert_eq!(app.panel, Panel::Budgets);
    app.toggle_panel(Panel::Routing);
    assert_eq!(app.panel, Panel::Routing);
    app.toggle_panel(Panel::Routing);
    assert_eq!(
        app.panel,
        Panel::Models,
        "toggling off returns to the models"
    );
}

#[test]
fn missing_cost_never_displays_as_paid_zero() {
    let usage = Usage {
        cost_status: CostStatus::Calculated,
        cost: None,
        ..Default::default()
    };
    assert_eq!(cost_display(&usage), "CALCULATED / NO COST");
}

#[test]
fn rows_do_not_mix_cost_provenance() {
    let mut app = App {
        range: Range::All,
        usages: vec![
            Usage {
                provider: "zen".into(),
                model: "model".into(),
                category: Category::Paid,
                cost_status: CostStatus::Calculated,
                cost: Some(1.0),
                created: now(),
                ..Default::default()
            },
            Usage {
                provider: "zen".into(),
                model: "model".into(),
                category: Category::Paid,
                cost_status: CostStatus::Estimated,
                cost: Some(2.0),
                created: now(),
                ..Default::default()
            },
        ],
        selected: 0,
        status: String::new(),
        degraded: false,
        last_refresh: String::new(),
        refresh_interval: Duration::from_secs(30),
        refreshed_at: Instant::now(),
        roots: crate::collector::SourceRoots {
            // Never the machine's real records: the tests below plant their own.
            omarchy_dir: Some(PathBuf::from("/nonexistent/omarchy")),
            ..crate::collector::SourceRoots::new(PathBuf::from("/tmp/unused-journal.db"))
        },
        limits_absence_logged: false,
        provider_filter: None,
        model_filter: None,
        collector: None,
        panel: Panel::Models,
        drilldown: None,
        search: Default::default(),
        sorts: Default::default(),
        show_help: false,
        budget_engine: BudgetEngine::empty(),
        update_notice: None,
        // Bundled, not loaded: a refreshed cache on the developer's machine must not change
        // how a test ranks two models.
        pricing: crate::pricing::PricingEngine::bundled(),
        alerts: Vec::new(),
        alert_sink: None,
        view: DerivedView::default(),
    };
    app.recompute();
    assert_eq!(app.rows().len(), 2);
}

#[test]
fn the_selection_clamps_to_the_visible_panel_not_the_model_table() {
    // Selection used to clamp to the model table unconditionally, so on any other table panel
    // it either stopped short of the end or ran past it.
    let base = crate::utils::now() - 5_000;
    let mut app = test_app(vec![
        session_usage("s1", Some("/w"), "claude-opus-5", base, Some(1.0)),
        session_usage("s2", Some("/w"), "claude-opus-5", base + 10, Some(1.0)),
        session_usage("s3", Some("/w"), "claude-opus-5", base + 20, Some(1.0)),
    ]);
    app.recompute();

    // All three rows collapse to one model row, but there are three sessions.
    assert_eq!(app.rows().len(), 1);
    app.toggle_panel(Panel::Sessions);
    assert_eq!(
        app.visible_rows(),
        3,
        "selection would have been capped at the model count"
    );
    app.panel = Panel::Models;
    assert_eq!(app.visible_rows(), 1);
}

/// `/` filters what a panel lists, and says so.
#[test]
fn a_search_narrows_the_visible_rows() {
    let mut app = test_app(vec![
        usage(Some("/w/api"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/docs"), Some("s2"), Some(2.0), 100),
        usage(Some("/w/api-tests"), Some("s3"), Some(3.0), 100),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    assert_eq!(app.projects().len(), 3);

    app.begin_search();
    for c in "api".chars() {
        app.search_key(c);
    }
    assert_eq!(app.projects().len(), 2, "/w/api and /w/api-tests");
    let (query, shown, total) = app
        .search_status()
        .expect("the footer must say it is filtering");
    assert_eq!((query, shown, total), ("api", 2, 3));
}

/// The filter changes what is listed, never what was spent.
///
/// Two views of one range disagreeing about the money is the failure this project cares most
/// about; a row filter that quietly narrowed the totals would be exactly that.
#[test]
fn a_search_does_not_change_the_totals() {
    let mut app = test_app(vec![
        usage(Some("/w/api"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/docs"), Some("s2"), Some(2.0), 100),
    ]);
    app.recompute();
    let before = app.totals().cost;
    let coverage_before = (
        app.coverage().billable_requests,
        app.coverage().priced_requests,
    );

    app.toggle_panel(Panel::Projects);
    app.begin_search();
    for c in "docs".chars() {
        app.search_key(c);
    }
    assert_eq!(app.projects().len(), 1, "the list narrowed");
    assert!(
        (app.totals().cost - before).abs() < 1e-9,
        "the headline total must not move: {} vs {before}",
        app.totals().cost
    );
    assert_eq!(
        (
            app.coverage().billable_requests,
            app.coverage().priced_requests
        ),
        coverage_before,
        "nor the coverage figure"
    );
}

/// Matching is case-insensitive and looks at the whole identity of a row.
#[test]
fn a_search_matches_case_insensitively_across_a_rows_identity() {
    let mut app = test_app(vec![usage(Some("/w/API"), Some("s1"), Some(1.0), 100)]);
    app.recompute();
    app.toggle_panel(Panel::Projects);

    app.begin_search();
    for c in "api".chars() {
        app.search_key(c);
    }
    assert_eq!(app.projects().len(), 1, "lowercase query, uppercase path");

    // Sessions match on their project and their models too, not only the opaque id.
    app.cancel_search();
    app.toggle_panel(Panel::Sessions);
    app.begin_search();
    // The fixture rows are `anthropic/claude-sonnet-5`, and `SessionTotals::models` holds
    // `provider/model` pairs -- so a model substring has to match through that.
    for c in "sonnet".chars() {
        app.search_key(c);
    }
    assert_eq!(
        app.sessions().len(),
        1,
        "the session's model should match, not just its uuid"
    );
}

/// Backspace shortens the query and, when it is already empty, leaves the filter.
#[test]
fn backspace_shortens_then_leaves_the_filter() {
    let mut app = test_app(vec![usage(Some("/w/api"), Some("s1"), Some(1.0), 100)]);
    app.recompute();
    app.begin_search();
    app.search_key('a');
    app.search_key('b');
    assert!(app.is_typing_search());

    app.search_backspace();
    assert_eq!(app.search_status().map(|(q, _, _)| q), Some("a"));
    app.search_backspace();
    assert_eq!(app.search_status().map(|(q, _, _)| q), Some(""));
    app.search_backspace();
    assert!(!app.is_typing_search(), "an empty query backspaces out");
}

/// Enter keeps the filter but hands the keyboard back; Esc abandons it.
#[test]
fn enter_keeps_the_filter_and_esc_clears_it() {
    let mut app = test_app(vec![
        usage(Some("/w/api"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/docs"), Some("s2"), Some(2.0), 100),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    app.begin_search();
    for c in "api".chars() {
        app.search_key(c);
    }

    app.accept_search();
    assert!(!app.is_typing_search(), "keys go to the dashboard again");
    assert_eq!(app.projects().len(), 1, "but the filter is still on");
    assert!(app.search_status().is_some(), "and still announced");

    app.cancel_search();
    assert_eq!(app.projects().len(), 2);
    assert!(app.search_status().is_none());
}

/// A filter that empties the list must not leave the cursor pointing past the end.
#[test]
fn the_cursor_is_pulled_back_when_the_filter_shortens_the_list() {
    let mut app = test_app(vec![
        usage(Some("/w/a"), Some("s1"), Some(3.0), 100),
        usage(Some("/w/b"), Some("s2"), Some(2.0), 100),
        usage(Some("/w/c"), Some("s3"), Some(1.0), 100),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    app.selected = 2;

    app.begin_search();
    for c in "/w/a".chars() {
        app.search_key(c);
    }
    assert_eq!(app.projects().len(), 1);
    assert_eq!(app.selected, 0, "the cursor cannot sit past the last row");
}

/// The default sorts must reproduce the order each list has always had.
///
/// This is the invariant that lets sorting ship without changing what anyone sees until they
/// press a key. Every existing test that asserts on position — "ranked by cost", "most recently
/// active first" — is really a test of this.
#[test]
fn the_default_sorts_are_the_orders_the_lists_already_had() {
    let now = crate::utils::now();
    let mut app = test_app(vec![
        usage(Some("/w/api"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/docs"), Some("s2"), Some(9.0), 50),
    ]);
    app.recompute();

    // Projects: most expensive first.
    assert_eq!(app.projects()[0].project, "/w/docs");
    // Models: most tokens first.
    let tokens: Vec<u64> = app.rows().iter().map(|r| r.total_tokens()).collect();
    assert!(tokens.windows(2).all(|w| w[0] >= w[1]), "{tokens:?}");
    // Routing: cheapest per delivered result, which is what the panel used to produce by
    // re-sorting inside its draw call.
    let mut app = test_app(Vec::new());
    app.set_routing_for_test(vec![
        routing_agg("junior", "opencode/glm-5.2", 20, 60.00, 5, 15),
        routing_agg("reviewer", "anthropic/claude-opus-5", 12, 41.20, 12, 0),
    ]);
    assert_eq!(app.routing()[0].agent, "reviewer", "$3.43 beats $12.00");
    let _ = now;
}

/// Sessions order by STARTED, which is the column they display.
///
/// A deliberate change: the list used to order by `last_seen` while the time column showed
/// `first_seen`, so the column a reader saw was not the column the rows were in. Marking a
/// column as sorted while sorting by a different one would have made that permanent. The
/// fixture below discriminates — a session that started earlier but ran longer is more recently
/// *active* and less recently *started*, so the two orders disagree.
#[test]
fn sessions_order_by_the_time_column_they_display() {
    let now = crate::utils::now();
    let long_early = Usage {
        session_id: Some("early-but-long".into()),
        created: now - 1_000,
        ..usage(Some("/w/a"), Some("early-but-long"), Some(1.0), 10)
    };
    let long_early_end = Usage {
        created: now - 10,
        ..long_early.clone()
    };
    let short_late = Usage {
        session_id: Some("late-but-short".into()),
        created: now - 500,
        ..usage(Some("/w/b"), Some("late-but-short"), Some(1.0), 10)
    };

    let mut app = test_app(vec![long_early, long_early_end, short_late]);
    app.recompute();

    let order: Vec<&str> = app
        .sessions()
        .iter()
        .map(|s| s.session_id.as_str())
        .collect();
    assert_eq!(
        order,
        vec!["late-but-short", "early-but-long"],
        "most recently STARTED first, matching the column shown"
    );

    // The two orders genuinely differ here, or this proves nothing.
    let by_last: Vec<&str> = {
        let mut sessions: Vec<_> = app.sessions().to_vec();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_seen));
        sessions
            .iter()
            .map(|s| s.session_id.clone())
            .collect::<Vec<_>>()
            .leak()
            .iter()
            .map(|s| s.as_str())
            .collect()
    };
    assert_ne!(
        order, by_last,
        "the fixture must distinguish the two orders"
    );
}

/// `>` moves to the next column, `<` back, and both wrap.
#[test]
fn the_sort_column_cycles_within_the_panel() {
    let mut app = test_app(vec![usage(Some("/w/api"), Some("s1"), Some(1.0), 100)]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    let columns = Panel::Projects.sort_columns().len();
    let start = app.sort_for(Panel::Projects).column;

    app.cycle_sort_column(true);
    assert_eq!(app.sort_for(Panel::Projects).column, (start + 1) % columns);
    app.cycle_sort_column(false);
    assert_eq!(
        app.sort_for(Panel::Projects).column,
        start,
        "and back again"
    );

    // Wrapping, rather than stopping at the end.
    for _ in 0..columns {
        app.cycle_sort_column(true);
    }
    assert_eq!(
        app.sort_for(Panel::Projects).column,
        start,
        "a full lap returns"
    );
}

/// Sorting is per panel: a column index means different things on different panels.
#[test]
fn each_panel_keeps_its_own_sort() {
    let mut app = test_app(vec![usage(Some("/w/api"), Some("s1"), Some(1.0), 100)]);
    app.recompute();

    app.toggle_panel(Panel::Projects);
    let projects_before = app.sort_for(Panel::Projects);
    app.cycle_sort_column(true);
    let projects_after = app.sort_for(Panel::Projects);
    assert_ne!(projects_before.column, projects_after.column);

    app.toggle_panel(Panel::Sessions);
    assert_eq!(
        app.sort_for(Panel::Sessions),
        Panel::Sessions.default_sort_for_test(),
        "the sessions panel keeps its own default, not the projects column"
    );
    app.toggle_panel(Panel::Projects);
    assert_eq!(
        app.sort_for(Panel::Projects),
        projects_after,
        "and projects remembers"
    );
}

/// Reversing flips the order and the marker.
#[test]
fn reversing_flips_the_order_and_says_so() {
    let mut app = test_app(vec![
        usage(Some("/w/a"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/b"), Some("s2"), Some(9.0), 100),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    assert_eq!(
        app.projects()[0].project,
        "/w/b",
        "dearest first by default"
    );
    let column = app.sort_for(Panel::Projects).column;
    assert_eq!(app.sort_marker(Panel::Projects, column), " v");

    app.reverse_sort();
    assert_eq!(app.projects()[0].project, "/w/a", "cheapest first now");
    assert_eq!(app.sort_marker(Panel::Projects, column), " ^");
    // And only the sorted column is marked.
    let other = (column + 1) % Panel::Projects.sort_columns().len();
    assert_eq!(app.sort_marker(Panel::Projects, other), "");
}

/// Unknown cost is not zero, and must not be sorted as if it were -- in either direction.
///
/// Reversing the sorted list instead of the comparator satisfies "clusters at one end" while
/// still being wrong: ascending it led with the unknowns, presenting them as the cheapest rows.
/// Both directions are asserted because only one of them catches that.
#[test]
fn unknown_cost_sorts_to_one_end_rather_than_as_zero() {
    let mut app = test_app(vec![
        usage(Some("/w/a"), Some("s1"), Some(5.0), 100),
        usage(Some("/w/b"), Some("s2"), None, 100),
        usage(Some("/w/c"), Some("s3"), Some(1.0), 100),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Models);
    let cost_column = Panel::Models
        .sort_columns()
        .iter()
        .position(|c| *c == "COST")
        .unwrap();
    while app.sort_for(Panel::Models).column != cost_column {
        app.cycle_sort_column(true);
    }

    for _ in 0..2 {
        let descending = app.sort_for(Panel::Models).descending;
        let costs: Vec<Option<f64>> = app.rows().iter().map(|r| r.cost).collect();
        let first_unknown = costs.iter().position(|c| c.is_none());
        let last_known = costs.iter().rposition(|c| c.is_some());
        if let (Some(first_unknown), Some(last_known)) = (first_unknown, last_known) {
            assert!(
                last_known < first_unknown,
                "unknown cost must sit after every known one (descending={descending}), \
                 not lead as the cheapest row: {costs:?}"
            );
        }
        app.reverse_sort();
    }
}

/// A quota-billed rollup has a real cost that no API prices. It must not sort as $0.00.
///
/// The COST cell for these reads `quota`, never `$0.00` -- so ordering them below a project that
/// genuinely spent a cent has the cell and the ordering saying different things about one row.
#[test]
fn quota_billed_projects_do_not_sort_as_the_cheapest() {
    let mut app = test_app(vec![
        usage(Some("/w/priced"), Some("s1"), Some(0.01), 100),
        quota_project_usage(Some("/w/plan"), Some("s2"), 100),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    let cost_column = Panel::Projects
        .sort_columns()
        .iter()
        .position(|c| *c == "COST")
        .unwrap();
    while app.sort_for(Panel::Projects).column != cost_column {
        app.cycle_sort_column(true);
    }

    for _ in 0..2 {
        let descending = app.sort_for(Panel::Projects).descending;
        let order: Vec<&str> = app.projects().iter().map(|p| p.project.as_str()).collect();
        assert_eq!(
            order.last().copied(),
            Some("/w/plan"),
            "the quota-billed project has no figure and belongs after the priced one \
             (descending={descending}), not ranked below it as $0.00"
        );
        app.reverse_sort();
    }
}

/// The default order must be the one `project_totals` already produced, ties included.
///
/// `project_totals` sorts by cost then tokens, both descending. Sorting ascending and reversing
/// the vector reproduces the cost order and inverts the tokens tiebreak, because Rust's sorts are
/// stable. Equal costs are the only thing that shows it, so the two projects here share one.
#[test]
fn the_default_project_order_keeps_the_aggregator_tiebreak() {
    let mut app = test_app(vec![
        usage(Some("/w/small"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/large"), Some("s2"), Some(1.0), 900),
    ]);
    app.recompute();

    let order: Vec<&str> = app.projects().iter().map(|p| p.project.as_str()).collect();
    assert_eq!(
        order,
        vec!["/w/large", "/w/small"],
        "equal-cost projects are ordered by tokens descending, and the default sort must not \
         invert that"
    );
}

/// On a subscription the tiebreak is not an edge case: it orders the entire panel.
///
/// `accrue` returns before touching `cost` for quota-billed usage, so every project is `0.0` and
/// the whole list is one tie group. Reversing the vector inverted all of it at once.
#[test]
fn an_all_quota_projects_panel_is_still_ordered_by_tokens() {
    let mut app = test_app(vec![
        quota_project_usage(Some("/w/small"), Some("s1"), 100),
        quota_project_usage(Some("/w/large"), Some("s2"), 900),
        quota_project_usage(Some("/w/medium"), Some("s3"), 500),
    ]);
    app.recompute();

    let order: Vec<&str> = app.projects().iter().map(|p| p.project.as_str()).collect();
    assert_eq!(
        order,
        vec!["/w/large", "/w/medium", "/w/small"],
        "every project is quota-billed and costless, so tokens is the only thing ordering them"
    );
}

/// The sort key for a COST cell must stand for what that cell actually shows.
///
/// `cost_display` ignores `usage.cost` entirely for `Quota` and `Unavailable` -- it prints
/// `ON QUOTA` and `UNKNOWN COST` whatever number is sitting in the field. A sort key read
/// straight off `cost` therefore orders those rows by a figure no reader ever sees.
#[test]
fn the_cost_sort_key_stands_for_what_the_cell_shows() {
    use crate::ui::theme::cost_sort_key;
    let row = |cost_status, cost| Usage {
        cost_status,
        cost,
        ..Default::default()
    };

    // Genuinely costless. The cell says so in words and zero is the honest position.
    assert_eq!(cost_sort_key(&row(CostStatus::Free, None)), Some(0.0));
    assert_eq!(cost_sort_key(&row(CostStatus::Local, None)), Some(0.0));

    // Real cost, no per-token rate anywhere. `ON QUOTA` is not `$0.00` in the cell, so it must
    // not become `$0.00` in the ordering -- including when a stray figure rides along.
    assert_eq!(cost_sort_key(&row(CostStatus::Quota, Some(0.0))), None);
    assert_eq!(cost_sort_key(&row(CostStatus::Quota, Some(4.0))), None);
    assert_eq!(
        cost_sort_key(&row(CostStatus::Unavailable, Some(4.0))),
        None
    );

    // Priced statuses carry whatever the engine worked out, including nothing.
    assert_eq!(
        cost_sort_key(&row(CostStatus::Calculated, Some(4.0))),
        Some(4.0)
    );
    assert_eq!(cost_sort_key(&row(CostStatus::Calculated, None)), None);
}

/// The same rule, through the panel: a quota row must not lead a COST sort as the cheapest.
#[test]
fn a_quota_model_row_does_not_sort_as_the_cheapest() {
    let mut app = test_app(vec![
        usage(Some("/w/a"), Some("s1"), Some(0.01), 100),
        quota_project_usage(Some("/w/b"), Some("s2"), 100),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Models);
    let cost_column = Panel::Models
        .sort_columns()
        .iter()
        .position(|c| *c == "COST")
        .unwrap();
    while app.sort_for(Panel::Models).column != cost_column {
        app.cycle_sort_column(true);
    }

    for _ in 0..2 {
        let descending = app.sort_for(Panel::Models).descending;
        let last = app.rows().last().expect("two rows");
        assert_eq!(
            last.cost_status,
            CostStatus::Quota,
            "the quota row has no figure and belongs at the end (descending={descending})"
        );
        app.reverse_sort();
    }
}

/// Ties must survive a descending sort on an ordinary column too, not only on COST.
///
/// Equal-token projects arrive from `project_totals` in cost-descending order. Sorting the panel
/// by TOKENS descending has nothing to distinguish them, so that incoming order is the whole
/// answer -- and reversing the vector rather than the comparator threw it away.
#[test]
fn a_descending_sort_on_a_plain_column_keeps_the_incoming_tie_order() {
    let mut app = test_app(vec![
        usage(Some("/w/rich"), Some("s1"), Some(5.0), 100),
        usage(Some("/w/poor"), Some("s2"), Some(1.0), 100),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    let tokens_column = Panel::Projects
        .sort_columns()
        .iter()
        .position(|c| *c == "TOKENS")
        .unwrap();
    while app.sort_for(Panel::Projects).column != tokens_column {
        app.cycle_sort_column(true);
    }
    if !app.sort_for(Panel::Projects).descending {
        app.reverse_sort();
    }

    let order: Vec<&str> = app.projects().iter().map(|p| p.project.as_str()).collect();
    assert_eq!(
        order,
        vec!["/w/rich", "/w/poor"],
        "equal token counts leave the aggregator's cost ordering as the only tiebreak"
    );
}

/// A panel with nothing to sort ignores the keys rather than storing an unusable column.
#[test]
fn panels_without_columns_ignore_the_sort_keys() {
    let mut app = test_app(vec![usage(Some("/w/api"), Some("s1"), Some(1.0), 100)]);
    app.recompute();
    app.toggle_panel(Panel::Budgets);
    assert!(Panel::Budgets.sort_columns().is_empty());

    app.cycle_sort_column(true);
    app.reverse_sort();
    assert_eq!(app.sort_marker(Panel::Budgets, 0), "", "nothing to mark");
}
