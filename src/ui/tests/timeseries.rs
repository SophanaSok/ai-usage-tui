//! Daily bucketing and the spend-over-time chart.

use super::*;

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
