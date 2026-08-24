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
        pulse: 0,
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
        show_help: false,
        budget_engine: BudgetEngine::empty(),
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
