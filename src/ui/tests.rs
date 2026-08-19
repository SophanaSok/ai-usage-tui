//! Tests for the dashboard's state and aggregation.

#![cfg(test)]

use super::*;
use crate::budget::BudgetEngine;
use crate::escalation::{Escalations, Transition};
use crate::model::{Category, Usage};
use crate::ui::app::DerivedView;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::model::{CostStatus, Range};
use crate::utils::now;

fn usage(project: Option<&str>, session: Option<&str>, cost: Option<f64>, tokens: u64) -> Usage {
    Usage {
        provider: "anthropic".into(),
        model: "claude-sonnet-5".into(),
        category: Category::Paid,
        cost_status: if cost.is_some() {
            CostStatus::Calculated
        } else {
            CostStatus::Unavailable
        },
        requests: 1,
        input: tokens,
        cost,
        created: now(),
        session_id: session.map(str::to_string),
        project: project.map(str::to_string),
        ..Default::default()
    }
}

#[test]
fn projects_are_ranked_by_cost_and_count_distinct_sessions() {
    let rows = project_totals(&[
        usage(Some("/w/api"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/api"), Some("s1"), Some(2.0), 100),
        usage(Some("/w/api"), Some("s2"), Some(3.0), 100),
        usage(Some("/w/docs"), Some("s3"), Some(10.0), 50),
    ]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].project, "/w/docs");
    assert_eq!(rows[1].project, "/w/api");
    assert_eq!(rows[1].cost, 6.0);
    assert_eq!(rows[1].sessions, 2, "two sessions, three requests");
    assert_eq!(rows[1].requests, 3);
}

#[test]
fn usage_without_a_project_is_still_accounted_for() {
    // Dropping unattributed rows would make the per-project totals quietly disagree with
    // the headline total — the same class of bug as two panels disagreeing on PAID.
    let rows = project_totals(&[
        usage(Some("/w/api"), None, Some(1.0), 100),
        usage(None, None, Some(4.0), 100),
    ]);
    let total: f64 = rows.iter().map(|r| r.cost).sum();
    assert_eq!(total, 5.0);
    assert!(rows.iter().any(|r| r.project == "(unattributed)"));
}

#[test]
fn a_project_with_unpriced_requests_reports_them() {
    let rows = project_totals(&[
        usage(Some("/w/api"), None, Some(1.0), 100),
        usage(Some("/w/api"), None, None, 100),
    ]);
    assert_eq!(rows[0].cost, 1.0);
    assert_eq!(
        rows[0].unpriced_requests, 1,
        "an unpriced request must not vanish into a confident total"
    );
}

#[test]
fn coverage_reports_the_priced_share_of_billable_requests() {
    let c = coverage(&[
        usage(Some("/w"), None, Some(1.0), 100),
        usage(Some("/w"), None, Some(1.0), 100),
        usage(Some("/w"), None, None, 100),
    ]);
    assert_eq!(c.billable_requests, 3);
    assert_eq!(c.priced_requests, 2);
    assert!((c.pct().unwrap() - 66.666).abs() < 0.01);
}

#[test]
fn coverage_of_nothing_is_not_a_hundred_percent() {
    assert_eq!(coverage(&[]).pct(), None);
}

/// A bare `App` with no collector and no I/O, for testing view logic.
fn test_app(usages: Vec<Usage>) -> App {
    App {
        range: Range::All,
        usages,
        selected: 0,
        status: String::new(),
        degraded: false,
        last_refresh: String::new(),
        pulse: 0,
        refresh_interval: Duration::from_secs(30),
        refreshed_at: Instant::now(),
        db_path: None,
        journal_path: PathBuf::from("/tmp/unused-journal.db"),
        claude_dir: None,
        provider_filter: None,
        model_filter: None,
        collector: None,
        panel: Panel::Models,
        budget_engine: BudgetEngine::empty(),
        // Bundled, not loaded: a refreshed cache on the developer's machine must not change
        // how a test ranks two models.
        pricing: crate::pricing::PricingEngine::bundled(),
        alerts: Vec::new(),
        alert_sink: None,
        view: DerivedView::default(),
    }
}

#[test]
fn the_projects_view_and_the_headline_total_agree() {
    // Two panels reporting different numbers for the same data is the exact bug 1.8b was.
    let mut app = test_app(vec![
        usage(Some("/w/api"), Some("s1"), Some(1.5), 100),
        usage(Some("/w/docs"), Some("s2"), Some(2.5), 100),
        usage(None, None, Some(1.0), 100),
    ]);
    app.recompute();
    let per_project: f64 = app.projects().iter().map(|p| p.cost).sum();
    assert!((per_project - app.totals().cost).abs() < 1e-9);
}

#[test]
fn project_labels_lengthen_only_where_they_would_collide() {
    let labels = project_labels(&[
        "/home/dev/api/build".to_string(),
        "/home/dev/web/build".to_string(),
        "/home/dev/ai-usage-tui".to_string(),
    ]);
    assert_eq!(labels[0], "api/build");
    assert_eq!(labels[1], "web/build");
    assert_eq!(
        labels[2], "ai-usage-tui",
        "an unambiguous name should not be lengthened"
    );
}

#[test]
fn project_labels_handle_a_single_project_and_windows_paths() {
    assert_eq!(project_labels(&["/home/dev/app".into()]), vec!["app"]);
    assert_eq!(project_labels(&["C:\\src\\my-app".into()]), vec!["my-app"]);
    assert_eq!(
        project_labels(&["(unattributed)".into()]),
        vec!["(unattributed)"]
    );
}

#[test]
fn two_projects_sharing_a_basename_are_not_merged() {
    // The rollup keys on the full path; only the label is shortened.
    let mut a = usage(Some("/home/dev/api/build"), None, Some(1.0), 100);
    a.project = Some("/home/dev/api/build".into());
    let mut b = usage(Some("/home/dev/web/build"), None, Some(2.0), 100);
    b.project = Some("/home/dev/web/build".into());
    let rows = project_totals(&[a, b]);
    assert_eq!(rows.len(), 2, "distinct projects were merged by basename");
}

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
        db_path: None,
        journal_path: PathBuf::from("/tmp/unused-journal.db"),
        claude_dir: None,
        provider_filter: None,
        model_filter: None,
        collector: None,
        panel: Panel::Models,
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

/// A usage row with an explicit creation timestamp.
fn usage_created_at(created: i64, tokens: u64, cost: Option<f64>) -> Usage {
    Usage {
        provider: "anthropic".into(),
        model: "claude-opus-5".into(),
        category: Category::Paid,
        cost_status: if cost.is_some() {
            CostStatus::Calculated
        } else {
            CostStatus::Unavailable
        },
        requests: 1,
        input: tokens,
        cost,
        created,
        ..Default::default()
    }
}

/// A usage row dated at local noon on `day`, for the daily aggregation.
fn usage_at(day: &str, tokens: u64, cost: Option<f64>) -> Usage {
    use chrono::TimeZone;
    let date = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("valid date");
    let noon = date.and_hms_opt(12, 0, 0).expect("valid time");
    let created = chrono::Local
        .from_local_datetime(&noon)
        .single()
        .expect("unambiguous local time")
        .timestamp();
    usage_created_at(created, tokens, cost)
}

/// A usage row `secs_ago` seconds before `now`, for the burn-rate window.
fn burn_usage(secs_ago: i64, now: i64, tokens: u64, cost: Option<f64>) -> Usage {
    usage_created_at(now - secs_ago, tokens, cost)
}

#[test]
fn daily_totals_bucket_by_local_day_oldest_first() {
    let days = crate::ui::aggregate::daily_totals(&[
        usage_at("2026-08-03", 100, Some(1.0)),
        usage_at("2026-08-01", 200, Some(2.0)),
        usage_at("2026-08-01", 300, Some(3.0)),
    ]);
    assert_eq!(days.len(), 3, "expected the 1st, 2nd and 3rd");
    assert_eq!(days[0].day, "2026-08-01");
    assert_eq!(days[0].tokens, 500);
    assert!((days[0].cost - 5.0).abs() < 1e-9);
    assert_eq!(days[2].day, "2026-08-03");
}

#[test]
fn quiet_days_are_kept_as_zero_bars() {
    // A chart that drops empty days compresses a quiet week to the width of a busy one and
    // reads as steady activity. The gap is a real observation.
    let days = crate::ui::aggregate::daily_totals(&[
        usage_at("2026-08-01", 100, Some(1.0)),
        usage_at("2026-08-05", 100, Some(1.0)),
    ]);
    assert_eq!(
        days.len(),
        5,
        "expected 01..05 inclusive, got {:?}",
        days.iter().map(|d| &d.day).collect::<Vec<_>>()
    );
    assert_eq!(days[1].tokens, 0);
    assert_eq!(days[1].requests, 0);
    assert_eq!(days[1].day, "2026-08-02");
}

#[test]
fn a_partly_priced_day_reports_its_unpriced_requests() {
    let days = crate::ui::aggregate::daily_totals(&[
        usage_at("2026-08-01", 100, Some(1.0)),
        usage_at("2026-08-01", 100, None),
    ]);
    assert_eq!(days[0].unpriced_requests, 1);
    assert!((days[0].cost - 1.0).abs() < 1e-9);
}

#[test]
fn undated_usage_is_left_out_rather_than_bucketed_at_the_epoch() {
    // created == 0 would otherwise create a 1970 bucket and, with gap filling, ~20000 days.
    let days = crate::ui::aggregate::daily_totals(&[
        Usage {
            input: 100,
            ..Default::default()
        },
        usage_at("2026-08-01", 100, Some(1.0)),
    ]);
    assert_eq!(days.len(), 1);
    assert_eq!(days[0].day, "2026-08-01");
}

#[test]
fn no_usage_produces_no_days() {
    assert!(crate::ui::aggregate::daily_totals(&[]).is_empty());
}

#[test]
fn the_bar_never_vanishes_for_a_nonzero_day() {
    // A whole-cell bar renders empty for anything under 1/12 of the peak, so a chart of mostly
    // small days would read as no activity. Sub-cell resolution is the point.
    let tiny = crate::ui::panels::timeseries::bar(0.01, 100.0);
    assert!(
        !tiny.is_empty(),
        "a day with real spend rendered as nothing"
    );
    assert_eq!(crate::ui::panels::timeseries::bar(0.0, 100.0), "");
    assert_eq!(
        crate::ui::panels::timeseries::bar(1.0, 0.0),
        "",
        "no peak means no scale"
    );
}

#[test]
fn the_bar_is_full_width_at_the_peak() {
    let full = crate::ui::panels::timeseries::bar(100.0, 100.0);
    assert_eq!(full.chars().count(), 12);
    assert!(
        full.chars().all(|c| c == '\u{2588}'),
        "expected all full blocks, got {full}"
    );
}

#[test]
fn the_time_series_panel_is_reachable_and_toggles_back() {
    let mut app = test_app(Vec::new());
    app.toggle_panel(Panel::TimeSeries);
    assert_eq!(app.panel, Panel::TimeSeries);
    app.toggle_panel(Panel::TimeSeries);
    assert_eq!(app.panel, Panel::Models);
}

#[test]
fn the_time_series_panel_renders_days_costs_and_bars() {
    // A rendering test, not just a data test: the panel is the deliverable, and a chart that
    // computes correct numbers but draws nothing is still broken.
    use ratatui::{backend::TestBackend, Terminal};

    let mut app = test_app(vec![
        usage_at("2026-08-01", 1_000_000, Some(10.0)),
        usage_at("2026-08-03", 500_000, Some(2.5)),
    ]);
    app.recompute();
    app.toggle_panel(Panel::TimeSeries);

    let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("backend");
    terminal
        .draw(|frame| crate::ui::panels::timeseries::draw_timeseries(frame, frame.area(), &app))
        .expect("draw");

    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();

    assert!(
        rendered.contains("SPEND OVER TIME"),
        "missing title:\n{rendered}"
    );
    assert!(
        rendered.contains("2026-08-03"),
        "missing the most recent day"
    );
    assert!(
        rendered.contains("2026-08-02"),
        "quiet day was dropped from the chart"
    );
    assert!(rendered.contains("$10.00"), "missing the peak day's cost");
    assert!(rendered.contains('\u{2588}'), "no bar was drawn");
}

#[test]
fn an_empty_range_says_so_rather_than_drawing_an_empty_chart() {
    use ratatui::{backend::TestBackend, Terminal};

    let mut app = test_app(Vec::new());
    app.recompute();
    let mut terminal = Terminal::new(TestBackend::new(60, 8)).expect("backend");
    terminal
        .draw(|frame| crate::ui::panels::timeseries::draw_timeseries(frame, frame.area(), &app))
        .expect("draw");
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("No dated usage"), "{rendered}");
}

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

/// Render the burn panel and return its text, for the assertions below.
fn render_timeseries(app: &App, w: u16, h: u16) -> String {
    render_panel(w, h, |frame, area| {
        crate::ui::panels::timeseries::draw_timeseries(frame, area, app)
    })
}

fn render_projects(app: &App, w: u16, h: u16) -> String {
    render_panel(w, h, |frame, area| {
        crate::ui::panels::projects::draw_projects(frame, area, app)
    })
}

fn render_panel(
    w: u16,
    h: u16,
    draw: impl FnOnce(&mut ratatui::Frame, ratatui::layout::Rect),
) -> String {
    use ratatui::{backend::TestBackend, Terminal};
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend");
    terminal
        .draw(|frame| draw(frame, frame.area()))
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn render_burn(app: &App, w: u16, h: u16) -> String {
    use ratatui::{backend::TestBackend, Terminal};
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend");
    terminal
        .draw(|frame| crate::ui::panels::burn::draw_burn(frame, frame.area(), app))
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
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
    // The label column must be wide enough for the period, or `monthly` truncates to `mo`.
    assert!(rendered.contains("global daily"), "{rendered}");
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

fn session_usage(
    id: &str,
    project: Option<&str>,
    model: &str,
    created: i64,
    cost: Option<f64>,
) -> Usage {
    Usage {
        provider: "anthropic".into(),
        model: model.into(),
        category: Category::Paid,
        cost_status: if cost.is_some() {
            CostStatus::Calculated
        } else {
            CostStatus::Unavailable
        },
        requests: 1,
        input: 1000,
        cost,
        created,
        session_id: Some(id.into()),
        project: project.map(str::to_string),
        ..Default::default()
    }
}

#[test]
fn sessions_are_listed_most_recently_active_first() {
    let sessions = crate::ui::aggregate::session_totals(&[
        session_usage("older", Some("/w/a"), "claude-opus-5", 1_000, Some(1.0)),
        session_usage("newer", Some("/w/b"), "claude-opus-5", 9_000, Some(2.0)),
    ]);
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].session_id, "newer");
    assert_eq!(sessions[1].session_id, "older");
}

#[test]
fn a_session_spans_its_first_and_last_request() {
    let sessions = crate::ui::aggregate::session_totals(&[
        session_usage("s", Some("/w"), "claude-opus-5", 5_000, Some(1.0)),
        session_usage("s", Some("/w"), "claude-opus-5", 1_000, Some(1.0)),
        session_usage("s", Some("/w"), "claude-opus-5", 9_000, Some(1.0)),
    ]);
    assert_eq!(sessions[0].first_seen, 1_000);
    assert_eq!(sessions[0].last_seen, 9_000);
    assert_eq!(sessions[0].duration_secs(), 8_000);
    assert_eq!(sessions[0].requests, 3);
}

#[test]
fn a_session_records_every_model_it_used_once_each() {
    let sessions = crate::ui::aggregate::session_totals(&[
        session_usage("s", Some("/w"), "claude-opus-5", 1_000, Some(1.0)),
        session_usage("s", Some("/w"), "claude-haiku-4-5", 2_000, Some(0.1)),
        session_usage("s", Some("/w"), "claude-opus-5", 3_000, Some(1.0)),
    ]);
    assert_eq!(sessions[0].models.len(), 2, "{:?}", sessions[0].models);
}

#[test]
fn usage_without_a_session_id_is_skipped_not_grouped_together() {
    // Journal and OpenCode rows carry no session. Bucketing them under one empty key would
    // invent a session that never existed and attribute unrelated work to it.
    let sessions = crate::ui::aggregate::session_totals(&[
        Usage {
            input: 100,
            ..Default::default()
        },
        session_usage("s", Some("/w"), "claude-opus-5", 1_000, Some(1.0)),
    ]);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "s");
}

#[test]
fn a_partly_priced_session_reports_its_unpriced_requests() {
    let sessions = crate::ui::aggregate::session_totals(&[
        session_usage("s", Some("/w"), "claude-opus-5", 1_000, Some(2.0)),
        session_usage("s", Some("/w"), "claude-opus-5", 2_000, None),
    ]);
    assert_eq!(sessions[0].unpriced_requests, 1);
    assert!((sessions[0].cost - 2.0).abs() < 1e-9);
}

#[test]
fn a_session_keeps_its_project_when_a_later_row_lacks_one() {
    let sessions = crate::ui::aggregate::session_totals(&[
        session_usage("s", Some("/w/api"), "claude-opus-5", 1_000, Some(1.0)),
        session_usage("s", None, "claude-opus-5", 2_000, Some(1.0)),
    ]);
    assert_eq!(sessions[0].project.as_deref(), Some("/w/api"));
}

fn render_sessions(app: &App, w: u16, h: u16) -> String {
    use ratatui::{backend::TestBackend, Terminal};
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend");
    terminal
        .draw(|frame| crate::ui::panels::sessions::draw_sessions(frame, frame.area(), app))
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn a_session_row_is_identifiable_without_its_uuid() {
    // The whole point of the panel: `0b9a76d5-b923-4b4f-8f20-51cea4534407` tells a reader
    // nothing, so the row has to carry when it ran, where, and on what instead.
    let base = crate::utils::now() - 90_000;
    let mut app = test_app(vec![
        session_usage(
            "0b9a76d5-b923-4b4f",
            Some("/home/x/Projects/ai-usage-tui"),
            "claude-opus-5",
            base,
            Some(4.10),
        ),
        session_usage(
            "0b9a76d5-b923-4b4f",
            Some("/home/x/Projects/ai-usage-tui"),
            "claude-opus-5",
            base + 6300,
            Some(8.30),
        ),
    ]);
    app.recompute();

    let rendered = render_sessions(&app, 84, 6);
    assert!(rendered.contains("ai-usage-tui"), "no project:\n{rendered}");
    assert!(rendered.contains("claude-opus-5"), "no model:\n{rendered}");
    assert!(rendered.contains("$12.40"), "no cost:\n{rendered}");
    assert!(rendered.contains("1h 45m"), "no duration:\n{rendered}");
    assert!(
        !rendered.contains("0b9a76d5"),
        "the raw uuid was rendered:\n{rendered}"
    );
}

#[test]
fn a_session_using_several_models_says_how_many() {
    let base = crate::utils::now() - 5_000;
    let mut app = test_app(vec![
        session_usage("s", Some("/w"), "claude-opus-5", base, Some(1.0)),
        session_usage("s", Some("/w"), "claude-haiku-4-5", base + 60, Some(0.1)),
    ]);
    app.recompute();
    assert!(render_sessions(&app, 84, 6).contains("2 models"));
}

#[test]
fn an_unpriced_session_says_so_rather_than_showing_zero() {
    let base = crate::utils::now() - 5_000;
    let mut app = test_app(vec![session_usage(
        "s",
        Some("/w"),
        "claude-sonnet-5",
        base,
        None,
    )]);
    app.recompute();
    let rendered = render_sessions(&app, 84, 6);
    assert!(rendered.contains("unpriced"), "{rendered}");
    assert!(
        !rendered.contains("$0.00"),
        "unknown cost rendered as zero:\n{rendered}"
    );
}

#[test]
fn no_sessions_explains_which_sources_provide_them() {
    let mut app = test_app(Vec::new());
    app.recompute();
    let rendered = render_sessions(&app, 84, 6);
    assert!(rendered.contains("No sessions"), "{rendered}");
    assert!(
        rendered.contains("Claude Code"),
        "should say where sessions come from:\n{rendered}"
    );
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

fn routing_agg(
    agent: &str,
    model: &str,
    tasks: u64,
    cost: f64,
    passes: u32,
    failures: u32,
) -> crate::model::RoutingAggregates {
    crate::model::RoutingAggregates {
        agent: agent.into(),
        model: model.into(),
        provider: "p".into(),
        tasks,
        tokens: 100_000,
        cost,
        test_passes: passes,
        test_failures: failures,
        ..Default::default()
    }
}

fn render_routing(app: &App, w: u16, h: u16) -> String {
    use ratatui::{backend::TestBackend, Terminal};
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend");
    terminal
        .draw(|frame| crate::ui::panels::routing::draw_routing(frame, frame.area(), app))
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

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
fn the_header_shows_how_much_of_the_spend_is_actually_priced() {
    // Provenance is the project's differentiator and it lived in an internal enum. A reader
    // could take a total at face value without learning it covered two thirds of the requests.
    use ratatui::{backend::TestBackend, Terminal};

    let render = |app: &App| -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 3)).expect("backend");
        terminal
            .draw(|frame| crate::ui::panels::header::draw_header(frame, frame.area(), app))
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    };

    let mut partial = test_app(vec![
        usage_at("2026-08-01", 1000, Some(1.0)),
        usage_at("2026-08-01", 1000, Some(1.0)),
        usage_at("2026-08-01", 1000, None),
    ]);
    partial.recompute();
    assert!(
        render(&partial).contains("67% priced"),
        "{}",
        render(&partial)
    );

    let mut complete = test_app(vec![usage_at("2026-08-01", 1000, Some(1.0))]);
    complete.recompute();
    let rendered = render(&complete);
    assert!(rendered.contains("all priced"), "{rendered}");
    assert!(
        !rendered.contains('%'),
        "a fully priced range should not shout a percentage:\n{rendered}"
    );
}

fn escalations_for_test(
    examined: u64,
    escalated: u64,
    transitions: Vec<Transition>,
) -> Escalations {
    Escalations {
        sessions_examined: examined,
        sessions_escalated: escalated,
        unclassified_changes: 0,
        transitions,
    }
}

fn transition(
    from: &str,
    to: &str,
    sessions: u64,
    cost_after: f64,
    unpriced_after: u64,
) -> Transition {
    Transition {
        from: from.to_string(),
        to: to.to_string(),
        sessions,
        cost_after,
        unpriced_after,
    }
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

// ---------------------------------------------------------------------------------------------
// Quota-billed usage: cost this tool deliberately will not attribute per request.
// ---------------------------------------------------------------------------------------------

/// A cloud row as the collectors produce it — unpriced and `Category::Cloud` — normalised by the
/// real pricing pass rather than by hand-setting the enum, so these tests exercise the seam.
fn quota_usage(tokens: u64, created: i64) -> Usage {
    let mut usage = Usage {
        provider: "ollama-cloud".into(),
        model: "glm-5.2:cloud".into(),
        category: Category::Cloud,
        cost_status: CostStatus::Unavailable,
        requests: 1,
        input: tokens,
        cost: None,
        created,
        session_id: Some("cloud-session".into()),
        project: Some("/w/cloud".into()),
        ..Default::default()
    };
    let engine = crate::pricing::PricingEngine::bundled();
    crate::pricing::apply_estimated_pricing(std::slice::from_mut(&mut usage), &engine);
    assert_eq!(
        usage.cost_status,
        CostStatus::Quota,
        "fixture must go through the real normalisation"
    );
    usage
}

#[test]
fn quota_billed_usage_does_not_reduce_pricing_coverage() {
    // Measured against real data: the header read "71.6% priced" when every unpriced request was
    // Ollama Cloud usage the tool deliberately refuses to price. A correct refusal to invent a
    // number was being reported as a failure to produce one.
    let now = crate::utils::now();
    let mut usages: Vec<Usage> = (0..7).map(|_| usage(None, None, Some(1.0), 100)).collect();
    usages.extend((0..3).map(|i| quota_usage(100, now - i)));

    let c = coverage(&usages);
    assert_eq!(
        c.pct(),
        Some(100.0),
        "quota-billed work is not a pricing gap; every priceable request here was priced"
    );
    assert_eq!(
        c.quota_requests, 3,
        "and it must still be counted, or the volume silently disappears"
    );
}

#[test]
fn a_billable_model_with_no_rate_still_counts_against_coverage() {
    // The anti-test for the one above: the fix must not make coverage unconditionally 100% by
    // swallowing the real case the figure exists to report.
    let mut unpriceable = usage(None, None, None, 100);
    unpriceable.model = "a-model-no-table-has".into();
    let usages = vec![usage(None, None, Some(1.0), 100), unpriceable];

    let c = coverage(&usages);
    assert_eq!(c.pct(), Some(50.0), "a genuine missing rate is still a gap");
    assert_eq!(c.quota_requests, 0);
}

#[test]
fn a_day_of_only_quota_work_is_not_rendered_as_free() {
    // Discriminating in both directions. Before the fix this said "unpriced"; the naive fix —
    // dropping quota rows from unpriced_requests with no replacement counter — makes the
    // arithmetic produce 0.0 and renders "$0.00" for usage that genuinely costs money.
    let now = crate::utils::now();
    let mut app = test_app((0..3).map(|i| quota_usage(50_000, now - i)).collect());
    app.recompute();

    let rendered = render_timeseries(&app, 84, 8);
    assert!(
        rendered.contains("quota"),
        "a quota-billed day must say so:\n{rendered}"
    );
    assert!(
        !rendered.contains("$0.00"),
        "never render cost this tool declined to attribute as zero:\n{rendered}"
    );
}

#[test]
fn a_session_of_only_quota_work_is_not_rendered_as_free() {
    let now = crate::utils::now();
    let mut app = test_app((0..3).map(|i| quota_usage(50_000, now - i)).collect());
    app.recompute();

    let rendered = render_sessions(&app, 100, 8);
    assert!(rendered.contains("quota"), "{rendered}");
    assert!(!rendered.contains("$0.00"), "{rendered}");
}

#[test]
fn a_project_of_only_quota_work_is_not_rendered_as_free() {
    let now = crate::utils::now();
    let mut app = test_app((0..3).map(|i| quota_usage(50_000, now - i)).collect());
    app.recompute();

    let rendered = render_projects(&app, 84, 8);
    assert!(rendered.contains("quota"), "{rendered}");
    assert!(!rendered.contains("$0.00"), "{rendered}");
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

#[test]
fn the_header_discloses_quota_volume_rather_than_dropping_it() {
    // "all priced" while thousands of requests sit outside the ratio is true and unhelpful.
    use ratatui::{backend::TestBackend, Terminal};
    let now = crate::utils::now();
    let mut usages = vec![usage(None, None, Some(1.0), 100)];
    usages.extend((0..3).map(|i| quota_usage(100, now - i)));
    let mut app = test_app(usages);
    app.recompute();

    let mut terminal = Terminal::new(TestBackend::new(120, 3)).expect("backend");
    terminal
        .draw(|frame| crate::ui::panels::header::draw_header(frame, frame.area(), &app))
        .expect("draw");
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();

    assert!(rendered.contains("all priced"), "{rendered}");
    assert!(
        rendered.contains("on quota"),
        "the denominator the percentage was taken over must stay visible:\n{rendered}"
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
