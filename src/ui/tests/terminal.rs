//! How the dashboard meets the terminal it is in: no colour, too few rows, and nothing to show.

use super::*;
use ratatui::{backend::TestBackend, Terminal};

fn render(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("backend");
    terminal
        .draw(|frame| crate::ui::draw(frame, app))
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

/// `NO_COLOR` was not read at all: every panel drew hard-coded RGB, backgrounds included.
#[test]
fn no_color_draws_no_colour_and_keeps_the_selection_visible() {
    let mut app = test_app(vec![
        usage(None, None, Some(1.0), 100),
        Usage {
            model: "claude-opus-5".into(),
            ..usage(None, None, Some(2.0), 200)
        },
    ]);
    app.recompute();
    app.no_color = true;

    let buffer = render(&app, 120, 30);
    let coloured: Vec<_> = buffer
        .content()
        .iter()
        .filter(|cell| cell.fg != Color::Reset || cell.bg != Color::Reset)
        .collect();
    assert!(
        coloured.is_empty(),
        "{} cells still carry colour",
        coloured.len()
    );
    assert!(
        buffer
            .content()
            .iter()
            .any(|cell| cell.modifier.contains(Modifier::REVERSED)),
        "the selected row was only ever a background colour; without colour it must be reverse video"
    );

    // And the coloured frame is unchanged by the option existing.
    app.no_color = false;
    let coloured = render(&app, 120, 30);
    assert!(coloured
        .content()
        .iter()
        .any(|cell| cell.bg == crate::ui::theme::SELECTED));
}

/// Below the height the layout needs, ratatui squeezed the panels to nothing in silence.
#[test]
fn a_pane_too_short_says_so_and_still_says_how_to_quit() {
    let mut app = test_app(vec![usage(None, None, Some(1.0), 100)]);
    app.recompute();
    for height in [3u16, 12, crate::ui::MIN_HEIGHT - 1] {
        let buffer = render(&app, 80, height);
        let rendered = text(&buffer);
        assert!(
            rendered.contains("Terminal too short") && rendered.contains(&format!("has {height}")),
            "at {height} rows: {rendered}"
        );
        let last: String = (0..80).map(|x| buffer[(x, height - 1)].symbol()).collect();
        assert!(
            last.trim_end().ends_with("q quit"),
            "at {height} rows: {last:?}"
        );
    }
    let rendered = text(&render(&app, 80, crate::ui::MIN_HEIGHT));
    assert!(!rendered.contains("Terminal too short"), "{rendered}");
    assert!(rendered.contains("MODEL ACTIVITY"), "{rendered}");
}

/// The first screen of a new install was a header row over nothing and tiles reading `0`.
#[test]
fn an_empty_dashboard_points_at_doctor() {
    let mut app = test_app(Vec::new());
    app.recompute();
    let rendered = text(&render(&app, 140, 30));
    assert!(rendered.contains("No usage collected yet."), "{rendered}");
    assert!(rendered.contains("ai-usage-tui --doctor"), "{rendered}");
}

#[test]
fn a_range_with_nothing_in_it_is_told_apart_from_no_data() {
    let mut app = test_app(vec![usage_created_at(86_400, 100, Some(1.0))]);
    app.range = Range::Today;
    app.recompute();
    let rendered = text(&render(&app, 140, 30));
    assert!(rendered.contains("Nothing in TODAY."), "{rendered}");
    assert!(rendered.contains("for all time"), "{rendered}");
    assert!(
        !rendered.contains("--doctor"),
        "data exists; doctor is the wrong advice"
    );
}
