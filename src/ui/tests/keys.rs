//! Key bindings as the user sees them: the footer and the `?` overlay.

use super::*;

#[test]
fn the_quit_binding_is_visible_on_an_eighty_column_terminal() {
    // The footer grew to 106 columns as panels were added and a Paragraph truncates silently, so
    // on a standard 80-column terminal the tail — including how to quit — simply vanished. No
    // test rendered the whole dashboard at any width, which is why nothing caught it.
    use ratatui::{backend::TestBackend, Terminal};

    for width in [80u16, 100, 110, 116, 120] {
        let mut app = test_app(vec![usage(None, None, Some(1.0), 100)]);
        app.recompute();
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).expect("backend");
        terminal
            .draw(|frame| crate::ui::draw(frame, &app))
            .expect("draw");
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(
            rendered.contains("q quit"),
            "at {width} columns a user cannot see how to quit:\n{rendered}"
        );
        assert!(
            rendered.contains("? help"),
            "at {width} columns the help binding is hidden, and it is what carries the rest"
        );
    }
}

#[test]
fn the_help_overlay_lists_every_panel_binding() {
    // The compact footer names the panel keys as a run of letters; the overlay is where a reader
    // finds out which is which. If a panel is added without a line here, that is a dead end.
    use ratatui::{backend::TestBackend, Terminal};

    let mut app = test_app(Vec::new());
    app.show_help = true;
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("backend");
    terminal
        .draw(|frame| crate::ui::draw(frame, &app))
        .expect("draw");
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();

    assert!(rendered.contains("KEYS"), "{rendered}");
    for expected in [
        "budgets",
        "routing",
        "project",
        "spend over time",
        "burn",
        "sessions",
        "subscription limits",
    ] {
        assert!(
            rendered.contains(expected),
            "the overlay does not mention {expected:?}:\n{rendered}"
        );
    }
}
