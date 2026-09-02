//! Subscription rate-limit windows read from Omarchy's agents panel.

use super::*;

#[test]
fn the_limits_panel_renders_windows_bars_countdowns_and_the_tier() {
    let mut app = test_app(Vec::new());
    app.set_limits_for_test(fixture_limits(false));
    let rendered = render_limits(&app, 100, 12);
    for expected in [
        "Session (5-hour)",
        "92%",
        "2h 03m",
        "Weekly (7-day)",
        "41%",
        "Max 20x",
        "█",
        "Claude Code · Max 20x · updated 10m ago",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected:?}:\n{rendered}"
        );
    }
    assert!(
        !rendered.contains("Unknown window"),
        "a negative percent is Omarchy's unknown and must not be drawn:\n{rendered}"
    );
    assert!(
        !rendered.contains("codex"),
        "a record with no windows and no status has no row"
    );
}

#[test]
fn a_nearly_full_window_is_drawn_in_the_alarm_colour_and_a_stale_one_is_not() {
    let mut app = test_app(Vec::new());
    app.set_limits_for_test(fixture_limits(false));
    assert_eq!(
        colour_of(
            100,
            12,
            |f, a| crate::ui::panels::limits::draw_limits(f, a, &app),
            "92%"
        ),
        Some(crate::model::RED)
    );
    assert_ne!(
        colour_of(
            100,
            12,
            |f, a| crate::ui::panels::limits::draw_limits(f, a, &app),
            "41%"
        ),
        Some(crate::model::RED)
    );

    let mut app = test_app(Vec::new());
    app.set_limits_for_test(fixture_limits(true));
    let rendered = render_limits(&app, 100, 12);
    assert!(rendered.contains("stale"), "{rendered}");
    assert_ne!(
        colour_of(
            100,
            12,
            |f, a| crate::ui::panels::limits::draw_limits(f, a, &app),
            "92%"
        ),
        Some(crate::model::RED),
        "a stale 92% describes some earlier moment and must not alarm"
    );
}

#[test]
fn the_limits_panel_says_why_it_is_empty() {
    let mut app = test_app(Vec::new());
    app.set_limits_for_test(crate::omarchy::LimitsReport {
        dir: PathBuf::from("/nonexistent/omarchy"),
        present: false,
        ..Default::default()
    });
    let rendered = render_limits(&app, 100, 8);
    assert!(
        rendered.contains("No rate-limit windows to show"),
        "{rendered}"
    );
    assert!(rendered.contains("/nonexistent/omarchy"), "{rendered}");
    // The empty state names both sources now. It used to name only Omarchy, from a branch that
    // also short-circuited the rows -- which is why a second source could not render at all.
    assert!(rendered.contains("Claude Code"), "{rendered}");

    app.set_limits_for_test(crate::omarchy::LimitsReport {
        dir: PathBuf::from("/x"),
        present: true,
        problems: vec!["claude.json: expected value at line 1".into()],
        ..Default::default()
    });
    let rendered = render_limits(&app, 100, 8);
    assert!(
        rendered.contains("none carry rate-limit windows"),
        "{rendered}"
    );
    assert!(rendered.contains("unreadable: claude.json"), "{rendered}");

    app.roots.limits_enabled = false;
    let rendered = render_limits(&app, 100, 8);
    assert!(rendered.contains("disabled in config"), "{rendered}");
}

#[test]
fn the_header_names_the_binding_window_only_when_it_is_fresh() {
    let mut app = test_app(vec![usage(None, None, Some(1.0), 100)]);
    app.recompute();
    app.set_limits_for_test(fixture_limits(false));
    let colour = colour_of(
        120,
        3,
        |f, a| crate::ui::panels::header::draw_header(f, a, &app),
        "claude session 92%",
    );
    assert_eq!(
        colour,
        Some(crate::model::RED),
        "the fullest window sits beside the cost figure, in the alarm colour"
    );

    app.set_limits_for_test(fixture_limits(true));
    let colour = colour_of(
        120,
        3,
        |f, a| crate::ui::panels::header::draw_header(f, a, &app),
        "claude session",
    );
    assert_eq!(colour, None, "a stale window is not a fact about now");
}
