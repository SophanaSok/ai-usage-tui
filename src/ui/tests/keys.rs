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

/// The footer at every width from a phone-sized pane to a wide monitor: never truncated, so
/// `q quit` — the last hint in every form — is always the last thing on the line.
#[test]
fn the_footer_is_never_truncated_at_any_width() {
    use ratatui::{backend::TestBackend, Terminal};

    let mut app = test_app(vec![usage(None, None, Some(1.0), 100)]);
    app.recompute();
    for width in 16u16..=200 {
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).expect("backend");
        terminal
            .draw(|frame| crate::ui::draw(frame, &app))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        // The footer is the bottom-most row with anything on it: its chunk is two rows and the
        // hints are on the first, but which row that is is the layout's business, not this test's.
        let footer = (0..buffer.area.height)
            .rev()
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .find(|row| !row.trim().is_empty())
            .expect("something is drawn");
        assert!(
            footer.trim_end().ends_with("q quit"),
            "at {width} columns the footer is cut off: {footer:?}"
        );
        assert!(footer.contains("? help"), "at {width} columns: {footer:?}");
    }
}

/// The form changes exactly where the wider one stops fitting — measured, not at a number
/// someone wrote down. `full` is the full line's width today; if a panel is added it grows, and
/// this test moves with it.
#[test]
fn the_footer_takes_the_widest_form_that_fits() {
    let [full, compact, minimal] = crate::ui::keys::footer_forms();
    let width = |form: &[crate::ui::keys::Hint]| {
        // ` k word` per hint, two spaces between hints.
        1 + form
            .iter()
            .map(|h| h.key.len() + 1 + h.word.len())
            .sum::<usize>()
            + 2 * (form.len() - 1)
    };
    let (full_w, compact_w, minimal_w) = (width(&full), width(&compact), width(&minimal));
    assert_eq!(
        full_w, 120,
        "the full line is 120 columns today; update this if a hint changed"
    );

    let render = |width: usize| -> String {
        let line = crate::ui::footer(width as u16, None);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width as u16, 1))
                .expect("backend");
        terminal
            .draw(|frame| frame.render_widget(line, frame.area()))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..width as u16)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    };

    assert!(
        render(full_w).contains("b budgets"),
        "the full form fits exactly"
    );
    let at_compact = render(full_w - 1);
    assert!(
        at_compact.contains("btpgwsl panels") && !at_compact.contains("budgets"),
        "one column short of the full form, the panels fold: {at_compact:?}"
    );
    assert!(render(compact_w).contains("panels"));
    let at_minimal = render(compact_w - 1);
    assert!(
        at_minimal == " ? help  q quit" && !at_minimal.contains("range"),
        "one column short of the compact form, only help and quit: {at_minimal:?}"
    );
    assert_eq!(render(minimal_w), " ? help  q quit");
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
