//! The top bar: the update notice, and what yields to keep the collector status on screen.

use super::*;

/// The header row as it renders at `width`, with `notice` in the update cache slot.
fn header_at(width: u16, notice: Option<&str>) -> String {
    use ratatui::{backend::TestBackend, Terminal};

    let mut app = test_app(vec![usage(None, None, Some(1.0), 100)]);
    app.recompute();
    app.status = "ok".into();
    app.last_refresh = "14:07:22".into();
    app.update_notice = notice.map(str::to_string);
    let mut terminal = Terminal::new(TestBackend::new(width, 30)).expect("backend");
    terminal
        .draw(|frame| crate::ui::draw(frame, &app))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    (0..width)
        .map(|x| buffer[(x, 0)].symbol())
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The whole point of the cache: a user who never runs `--doctor` still learns a release exists.
#[test]
fn a_cached_newer_release_is_named_in_the_header() {
    let row = header_at(120, Some("↑ v0.11.0"));
    assert!(row.contains("↑ v0.11.0"), "{row:?}");
}

/// And says nothing at all when there is nothing to say — no glyph, no gap where one would go.
#[test]
fn nothing_is_drawn_when_no_check_has_found_anything() {
    let row = header_at(120, None);
    assert!(!row.contains('↑'), "{row:?}");
    assert!(row.contains("LIVE PROVIDER MONITOR"), "{row:?}");
}

/// The regression the subtitle exists to absorb. Before the notice, an 80-column header fitted
/// exactly; the notice is 11 columns and a `Paragraph` truncates in silence, so the collector
/// status — the one thing here that must never disappear quietly — went off the end.
#[test]
fn the_status_survives_the_notice_on_an_eighty_column_terminal() {
    let row = header_at(80, Some("↑ v0.11.0"));
    assert!(row.contains("↑ v0.11.0"), "the notice is missing: {row:?}");
    assert!(
        row.ends_with("ok"),
        "the collector status was cut off: {row:?}"
    );
    assert!(
        !row.contains("LIVE PROVIDER MONITOR"),
        "the subtitle should have yielded first: {row:?}"
    );
}

/// A degraded collector is what that status is for, and it is drawn last for emphasis. It must
/// survive the same squeeze.
#[test]
fn a_degraded_status_survives_the_notice_too() {
    use ratatui::{backend::TestBackend, Terminal};

    let mut app = test_app(vec![usage(None, None, Some(1.0), 100)]);
    app.recompute();
    app.degraded = true;
    app.status = "journal failing".into();
    app.last_refresh = "14:07:22".into();
    app.update_notice = Some("↑ v0.11.0".into());
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).expect("backend");
    terminal
        .draw(|frame| crate::ui::draw(frame, &app))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let row: String = (0..80).map(|x| buffer[(x, 0)].symbol()).collect();
    assert!(
        row.contains("journal failing"),
        "a failing collector went off the end: {row:?}"
    );
}

/// Nothing changes for anyone without a notice: the subtitle yields only when it must.
#[test]
fn the_subtitle_is_kept_whenever_the_line_fits() {
    for width in [80u16, 100, 120, 132] {
        let row = header_at(width, None);
        assert!(
            row.contains("LIVE PROVIDER MONITOR"),
            "at {width} columns the subtitle was dropped with room to spare: {row:?}"
        );
    }
    // And it comes back as soon as there is room for both.
    assert!(header_at(120, Some("↑ v0.11.0")).contains("LIVE PROVIDER MONITOR"));
}

/// What the pricing engine said at load reaches the header: a refused cache turns it red, tables
/// that are merely old are named and leave it alone. Until this, both were `--doctor`-only, so a
/// dashboard pricing from a table it had fallen back to looked exactly like one that was not.
#[test]
fn the_pricing_note_reaches_the_header_and_only_a_fault_turns_it_red() {
    use ratatui::{backend::TestBackend, Terminal};

    let render = |note: Option<(String, bool)>| {
        let mut app = test_app(vec![usage(None, None, Some(1.0), 100)]);
        app.recompute();
        app.status = "ok".into();
        app.pricing_note = note;
        let mut terminal = Terminal::new(TestBackend::new(160, 30)).expect("backend");
        terminal
            .draw(|frame| crate::ui::draw(frame, &app))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..160).map(|x| buffer[(x, 0)].symbol()).collect();
        let red = (0..160).any(|x| buffer[(x, 0)].style().fg == Some(crate::model::RED));
        (row, red)
    };

    let (row, red) = render(None);
    assert!(!row.contains("pricing:") && !red, "{row:?}");

    let (row, red) = render(Some((
        "pricing: bundled rates over 90 days old, see --doctor".into(),
        false,
    )));
    assert!(row.contains("bundled rates over 90 days old"), "{row:?}");
    assert!(!red, "old tables are said, not alarmed about");

    let (row, red) = render(Some(("pricing: 1 problem(s), see --doctor".into(), true)));
    assert!(row.contains("pricing: 1 problem(s)"), "{row:?}");
    assert!(red, "a refused cache is a fault");
}
