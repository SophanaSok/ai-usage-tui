//! The budgets panel: spend against each configured limit, and what that spend is standing on.

use super::*;
use crate::budget::{BudgetEntry, BudgetPeriod, BudgetScopeKind, BudgetsConfig};

fn app_with_global_budget(usages: Vec<Usage>) -> App {
    let mut app = test_app(usages);
    app.budget_engine = BudgetEngine::from_config(&BudgetsConfig {
        entry: vec![BudgetEntry {
            scope: BudgetScopeKind::Global,
            period: BudgetPeriod::Monthly,
            limit: 50.0,
            ..Default::default()
        }],
        ..Default::default()
    });
    app.recompute();
    // What `refresh` does after loading, without the I/O it does first: the alerts the panel
    // reads come from the real engine, not a hand-built `Alert`.
    app.alerts = app.budget_engine.check(&app.usages);
    app
}

fn render_budgets(app: &App, w: u16, h: u16) -> String {
    render_panel(w, h, |frame, area| {
        crate::ui::panels::budgets::draw_budgets(frame, area, app)
    })
}

#[test]
fn a_partly_unpriced_budget_renders_its_spend_as_a_floor() {
    // Restore the bug by summing only the priced rows into `Alert::spend` with no counter
    // beside it: the panel reads `$2.00` and `4%`, exactly, over work that was one-third
    // unpriced — and the burn panel's projection builds on the same number.
    let app = app_with_global_budget(vec![
        usage(None, None, Some(1.0), 100),
        usage(None, None, Some(1.0), 100),
        usage(None, None, None, 100),
    ]);
    let rendered = render_budgets(&app, 80, 6);
    assert!(rendered.contains("≥ $2.00"), "{rendered}");
    assert!(rendered.contains("≥ 4%"), "{rendered}");
}

#[test]
fn a_fully_priced_budget_renders_a_bare_figure() {
    // The anti-test: the fix must not make every budget a floor.
    let app = app_with_global_budget(vec![
        usage(None, None, Some(1.0), 100),
        usage(None, None, Some(1.0), 100),
    ]);
    let rendered = render_budgets(&app, 80, 6);
    assert!(rendered.contains("$2.00"), "{rendered}");
    assert!(rendered.contains("4%"), "{rendered}");
    assert!(
        !rendered.contains('≥'),
        "a fully priced budget was shown as a floor:\n{rendered}"
    );
}

#[test]
fn a_budget_with_no_alert_yet_renders_unknown_rather_than_zero() {
    // Alerts are computed in `refresh`; until the first one runs the panel has no figure, and
    // it used to print `$0.00 / 0% / OK` — a claim nothing had checked.
    let mut app = app_with_global_budget(vec![usage(None, None, Some(1.0), 100)]);
    app.alerts.clear();
    let rendered = render_budgets(&app, 80, 6);
    assert!(!rendered.contains("$0.00"), "{rendered}");
    assert!(!rendered.contains("OK"), "{rendered}");
    assert!(rendered.contains('—'), "{rendered}");
}

fn render_banner(app: &App, w: u16, h: u16) -> String {
    render_panel(w, h, |frame, area| {
        crate::ui::panels::alerts::draw_alert_banner(frame, area, app)
    })
}

#[test]
fn the_alert_banner_marks_a_floor_and_only_a_floor() {
    // The banner is the loudest place the figure appears, so it is the last place a floor may
    // pass as a total — and the first place an exact figure must not be dressed as one.
    use crate::budget::{Alert, AlertLevel, BudgetPeriod, BudgetScope};
    let mut app = test_app(Vec::new());
    let partial = Alert {
        scope: BudgetScope::Global,
        period: BudgetPeriod::Monthly,
        spend: 40.0,
        limit: 50.0,
        pct: 80.0,
        level: AlertLevel::Warn,
        unpriced_requests: 3,
        quota_requests: 0,
    };
    app.alerts = vec![partial.clone()];
    let rendered = render_banner(&app, 100, 2);
    assert!(rendered.contains("≥ $40.00"), "{rendered}");
    assert!(rendered.contains("(≥ 80%)"), "{rendered}");

    app.alerts = vec![Alert {
        unpriced_requests: 0,
        ..partial
    }];
    let rendered = render_banner(&app, 100, 2);
    assert!(rendered.contains("$40.00/$50.00 (80%)"), "{rendered}");
    assert!(
        !rendered.contains('≥'),
        "an exact figure was shown as a floor:\n{rendered}"
    );
}

#[test]
fn a_budget_over_only_quota_work_says_on_quota_rather_than_zero() {
    // `$0.00 / 0% / OK` reads as "this budget is untouched", which on a Max account — where
    // every request is quota-billed — is false for all of them.
    let now = crate::utils::now();
    let app = app_with_global_budget((0..3).map(|i| quota_usage(100, now - i)).collect());
    let rendered = render_budgets(&app, 80, 6);
    assert!(rendered.contains("on quota"), "{rendered}");
    assert!(
        !rendered.contains("$0.00"),
        "quota-billed work was rendered as costing nothing:\n{rendered}"
    );
    assert!(
        rendered.contains("OK"),
        "no threshold was crossed:\n{rendered}"
    );
}
