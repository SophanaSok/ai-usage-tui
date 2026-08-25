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
