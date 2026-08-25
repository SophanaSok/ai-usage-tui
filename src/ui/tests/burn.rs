//! Burn rate over a trailing window, and the projection against a budget.

use super::*;

#[test]
fn burn_rate_counts_only_the_trailing_window() {
    let now = 1_800_000_000;
    let burn = crate::ui::aggregate::burn_rate(
        &[
            burn_usage(30, now, 1000, Some(1.0)),   // inside
            burn_usage(59, now, 1000, Some(1.0)),   // inside
            burn_usage(600, now, 9999, Some(99.0)), // outside
        ],
        60,
        now,
    );
    assert_eq!(burn.requests, 2);
    assert_eq!(burn.tokens, 2000);
    assert!((burn.cost - 2.0).abs() < 1e-9);
}

#[test]
fn future_dated_usage_does_not_inflate_the_rate() {
    // Clock skew between the machine that wrote a log and this one would otherwise be counted
    // as usage that just happened.
    let now = 1_800_000_000;
    let burn =
        crate::ui::aggregate::burn_rate(&[burn_usage(-3600, now, 100_000, Some(50.0))], 3600, now);
    assert_eq!(burn.requests, 0, "a future-dated row was counted");
}

#[test]
fn rates_are_per_minute_and_per_hour() {
    let now = 1_800_000_000;
    let burn = crate::ui::aggregate::burn_rate(&[burn_usage(10, now, 6_000, Some(2.0))], 60, now);
    assert!((burn.tokens_per_minute() - 6_000.0).abs() < 1e-6);
    assert!(
        (burn.cost_per_hour() - 120.0).abs() < 1e-6,
        "got {}",
        burn.cost_per_hour()
    );
}

#[test]
fn a_thin_window_refuses_to_project() {
    // Three requests in an hour does not support "you hit your budget at 4pm". Refusing is the
    // same discipline as never rendering unknown cost as $0.00.
    let now = 1_800_000_000;
    let thin = crate::ui::aggregate::burn_rate(
        &[
            burn_usage(10, now, 100, Some(1.0)),
            burn_usage(20, now, 100, Some(1.0)),
        ],
        3600,
        now,
    );
    assert!(!thin.is_projectable());
    assert_eq!(crate::ui::aggregate::seconds_to_exhaust(&thin, 100.0), None);

    let enough: Vec<Usage> = (0..crate::model::BurnRate::MIN_SAMPLE)
        .map(|i| burn_usage(10 + i as i64, now, 100, Some(1.0)))
        .collect();
    let burn = crate::ui::aggregate::burn_rate(&enough, 3600, now);
    assert!(burn.is_projectable());
    assert!(crate::ui::aggregate::seconds_to_exhaust(&burn, 100.0).is_some());
}

#[test]
fn a_partly_unpriced_window_is_flagged_as_a_floor() {
    let now = 1_800_000_000;
    let burn = crate::ui::aggregate::burn_rate(
        &[
            burn_usage(10, now, 100, Some(1.0)),
            burn_usage(20, now, 100, None),
        ],
        3600,
        now,
    );
    assert!(burn.is_partial());
    assert_eq!(burn.unpriced_requests, 1);
}

#[test]
fn time_to_exhaust_matches_the_rate() {
    let now = 1_800_000_000;
    // 10 requests, $10 total, over a 1-hour window => $10/hour. $25 remaining => 2h30m.
    let usages: Vec<Usage> = (0..10)
        .map(|i| burn_usage(10 + i, now, 100, Some(1.0)))
        .collect();
    let burn = crate::ui::aggregate::burn_rate(&usages, 3600, now);
    let secs = crate::ui::aggregate::seconds_to_exhaust(&burn, 25.0).expect("projectable");
    assert_eq!(crate::ui::aggregate::format_duration(secs), "2h 30m");
}

#[test]
fn durations_read_coarsely() {
    use crate::ui::aggregate::format_duration;
    assert_eq!(format_duration(30), "<1m");
    assert_eq!(format_duration(90), "1m");
    assert_eq!(format_duration(3600 + 14 * 60), "1h 14m");
    assert_eq!(format_duration(3 * 86400 + 3600), "3d 1h");
}

#[test]
fn the_burn_panel_projects_against_a_budget() {
    use crate::budget::{Alert, AlertLevel, BudgetPeriod, BudgetScope};
    let now = crate::utils::now();
    let usages: Vec<Usage> = (0..40)
        .map(|i| burn_usage(i * 80, now, 180_000, Some(0.85)))
        .collect();
    let mut app = test_app(usages);
    app.recompute();
    app.alerts = vec![Alert {
        scope: BudgetScope::Global,
        period: BudgetPeriod::Daily,
        spend: 42.0,
        limit: 60.0,
        pct: 70.0,
        level: AlertLevel::Ok,
        unpriced_requests: 0,
        quota_requests: 0,
    }];

    let rendered = render_burn(&app, 64, 9);
    assert!(rendered.contains("BURN RATE"), "{rendered}");
    assert!(
        rendered.contains("/hr"),
        "missing a spend rate:\n{rendered}"
    );
    assert!(
        rendered.contains("left"),
        "missing the time-to-budget projection:\n{rendered}"
    );
    assert!(rendered.contains("$18.00 remaining"), "{rendered}");
    assert!(
        !rendered.contains('≤'),
        "an exact projection was shown as a bound:\n{rendered}"
    );
    // The label column must be wide enough for the period, or `monthly` truncates to `mo`.
    assert!(rendered.contains("global daily"), "{rendered}");
}

#[test]
fn a_floor_on_the_rate_bounds_the_time_but_not_the_figure() {
    // The window has an unpriced row, so the rate is a floor and the time left is "at most";
    // the budget's own spend is exact, so what is left is exactly $18.00. Marking the figure
    // too would disagree with the budgets panel about the same alert.
    use crate::budget::{Alert, AlertLevel, BudgetPeriod, BudgetScope};
    let now = crate::utils::now();
    let mut usages: Vec<Usage> = (0..40)
        .map(|i| burn_usage(i * 80, now, 180_000, Some(0.85)))
        .collect();
    usages.push(burn_usage(500, now, 1000, None));
    let mut app = test_app(usages);
    app.recompute();
    app.alerts = vec![Alert {
        scope: BudgetScope::Global,
        period: BudgetPeriod::Daily,
        spend: 42.0,
        limit: 60.0,
        pct: 70.0,
        level: AlertLevel::Ok,
        unpriced_requests: 0,
        quota_requests: 0,
    }];

    let rendered = render_burn(&app, 72, 9);
    assert!(rendered.contains("≤ "), "{rendered}");
    assert!(
        rendered.contains("($18.00 remaining)"),
        "an exact figure was marked as a bound:\n{rendered}"
    );
}

#[test]
fn the_burn_panel_declines_to_project_from_too_little() {
    use crate::budget::{Alert, AlertLevel, BudgetPeriod, BudgetScope};
    let now = crate::utils::now();
    let mut app = test_app(vec![burn_usage(60, now, 1000, Some(1.0))]);
    app.recompute();
    app.alerts = vec![Alert {
        scope: BudgetScope::Global,
        period: BudgetPeriod::Daily,
        spend: 1.0,
        limit: 60.0,
        pct: 1.7,
        level: AlertLevel::Ok,
        unpriced_requests: 0,
        quota_requests: 0,
    }];

    let rendered = render_burn(&app, 64, 9);
    assert!(
        rendered.contains("too little activity"),
        "one request should not produce a confident projection:\n{rendered}"
    );
    assert!(
        !rendered.contains("left"),
        "it projected anyway:\n{rendered}"
    );
}

#[test]
fn the_burn_panel_says_when_no_budgets_are_configured() {
    let now = crate::utils::now();
    let usages: Vec<Usage> = (0..10)
        .map(|i| burn_usage(i * 60, now, 1000, Some(1.0)))
        .collect();
    let mut app = test_app(usages);
    app.recompute();
    let rendered = render_burn(&app, 70, 9);
    assert!(rendered.contains("no budgets configured"), "{rendered}");
}

#[test]
fn an_idle_window_says_so_rather_than_showing_a_zero_rate() {
    let mut app = test_app(Vec::new());
    app.recompute();
    let rendered = render_burn(&app, 64, 6);
    assert!(
        rendered.contains("No usage in the trailing window"),
        "{rendered}"
    );
}

#[test]
fn a_partly_unpriced_window_renders_the_rate_as_a_floor() {
    let now = crate::utils::now();
    let mut usages: Vec<Usage> = (0..8)
        .map(|i| burn_usage(i * 60, now, 1000, Some(1.0)))
        .collect();
    usages.push(burn_usage(500, now, 1000, None));
    let mut app = test_app(usages);
    app.recompute();
    let rendered = render_burn(&app, 70, 8);
    assert!(
        rendered.contains("≥ $"),
        "a partial rate was shown as exact:\n{rendered}"
    );
    assert!(rendered.contains("unpriced"), "{rendered}");
}

#[test]
fn a_projection_from_a_floor_is_an_upper_bound() {
    // The panel rendered the rate as `≥ $x/hr (N unpriced)` and, two lines down, projected
    // time-to-exhaust from a budget spend that had dropped those same N requests. A floor on
    // the spend is a ceiling on what is left, so the projection is "at most".
    use crate::budget::{Alert, AlertLevel, BudgetPeriod, BudgetScope};
    let now = crate::utils::now();
    let usages: Vec<Usage> = (0..40)
        .map(|i| burn_usage(i * 80, now, 180_000, Some(0.85)))
        .collect();
    let mut app = test_app(usages);
    app.recompute();
    app.alerts = vec![Alert {
        scope: BudgetScope::Global,
        period: BudgetPeriod::Daily,
        spend: 42.0,
        limit: 60.0,
        pct: 70.0,
        level: AlertLevel::Ok,
        unpriced_requests: 3,
        quota_requests: 0,
    }];

    let rendered = render_burn(&app, 72, 9);
    assert!(rendered.contains("left"), "{rendered}");
    assert!(
        rendered.contains("≤ ") && rendered.contains("≤ $18.00 remaining"),
        "a projection from a floor was shown as exact:\n{rendered}"
    );
}

#[test]
fn a_quota_only_window_projects_on_quota_not_too_little_activity() {
    // Measured on a Max account: 550 quota-billed requests in the hour, and the projection row
    // read "too little activity to project (550/5 requests)". The count was fine; the rate was
    // zero because nothing was priced per token, and the message blamed the wrong condition.
    use crate::budget::{Alert, AlertLevel, BudgetPeriod, BudgetScope};
    let now = crate::utils::now();
    let mut app = test_app((0..10).map(|i| quota_usage(50_000, now - i * 60)).collect());
    app.recompute();
    app.alerts = vec![Alert {
        scope: BudgetScope::Global,
        period: BudgetPeriod::Daily,
        spend: 0.0,
        limit: 60.0,
        pct: 0.0,
        level: AlertLevel::Ok,
        unpriced_requests: 0,
        quota_requests: 10,
    }];

    let rendered = render_burn(&app, 84, 10);
    assert!(
        !rendered.contains("too little activity"),
        "ten requests is not too little; the rate is on quota:\n{rendered}"
    );
    // Once for the spend row, once for the projection row. Asserting `contains` alone passed
    // against the old code, because the spend row already said it.
    assert_eq!(
        rendered.matches("on quota").count(),
        2,
        "the projection row should say on quota too:\n{rendered}"
    );
}

#[test]
fn an_unpriced_window_projects_unpriced_not_too_little_activity() {
    // Same shape, other cause: enough requests, none of them priced, so the rate's floor is
    // zero and there is nothing to project from. That is "unpriced", not "too little".
    let now = crate::utils::now();
    let usages: Vec<Usage> = (0..10)
        .map(|i| burn_usage(i * 60, now, 1000, None))
        .collect();
    let mut app = test_app(usages);
    app.recompute();
    app.alerts = vec![crate::budget::Alert {
        scope: crate::budget::BudgetScope::Global,
        period: crate::budget::BudgetPeriod::Daily,
        spend: 0.0,
        limit: 60.0,
        pct: 0.0,
        level: crate::budget::AlertLevel::Ok,
        unpriced_requests: 10,
        quota_requests: 0,
    }];

    let rendered = render_burn(&app, 84, 10);
    assert!(!rendered.contains("too little activity"), "{rendered}");
    assert!(
        rendered.contains("projection") && rendered.matches("unpriced").count() >= 2,
        "the projection row should say unpriced:\n{rendered}"
    );
}

#[test]
fn a_burn_window_of_only_quota_work_reports_no_rate_rather_than_zero() {
    // "$0.00/hr" reads as "this is costing you nothing", which is false.
    let now = crate::utils::now();
    let mut app = test_app((0..10).map(|i| quota_usage(50_000, now - i * 60)).collect());
    app.recompute();

    let rendered = render_burn(&app, 84, 10);
    assert!(rendered.contains("on quota"), "{rendered}");
    assert!(!rendered.contains("$0.00/hr"), "{rendered}");
}
