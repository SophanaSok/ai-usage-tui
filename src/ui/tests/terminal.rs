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
    // The selected row itself, not any cell: the active tab is reverse video in every mode, so
    // "some cell is reversed" would hold with the selection gone.
    let width = usize::from(buffer.area.width);
    let selected_row = buffer
        .content()
        .chunks(width)
        .find(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("claude-opus-5")
        })
        .expect("the selected model is on screen");
    assert!(
        selected_row
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

/// A budget alert adds a banner row, so the layout needs one more. Checking the bare minimum let a
/// 20-row pane with an alert squeeze the body -- and the too-short screen must still say the alert
/// is there, or a short pane becomes how one goes unseen. Found in review of #104.
#[test]
fn an_alert_banner_raises_the_height_the_dashboard_needs() {
    use crate::budget::{Alert, AlertLevel, BudgetPeriod, BudgetScope};
    let mut app = test_app(vec![usage(None, None, Some(1.0), 100)]);
    app.recompute();
    app.alerts = vec![Alert {
        scope: BudgetScope::Global,
        period: BudgetPeriod::Monthly,
        spend: 45.0,
        limit: 50.0,
        pct: 90.0,
        level: AlertLevel::Critical,
        unpriced_requests: 0,
        quota_requests: 0,
    }];

    let rendered = text(&render(&app, 80, crate::ui::MIN_HEIGHT));
    assert!(
        rendered.contains("needs 21 rows, has 20"),
        "the banner's row was not counted: {rendered}"
    );
    assert!(rendered.contains("A budget alert is active."), "{rendered}");

    let rendered = text(&render(&app, 80, crate::ui::MIN_HEIGHT + 1));
    assert!(!rendered.contains("Terminal too short"), "{rendered}");
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

/// Every colour went to every terminal as 24-bit. One that says it draws fewer gets the frame
/// mapped down, and a colour path that bypasses the pass shows up here as an `Rgb` cell.
#[test]
fn a_terminal_with_fewer_colours_is_sent_none_it_cannot_draw() {
    use crate::utils::ColourDepth;
    let mut app = test_app(vec![
        usage(None, None, Some(1.0), 100),
        Usage {
            model: "claude-opus-5".into(),
            ..usage(None, None, Some(2.0), 200)
        },
    ]);
    app.recompute();
    app.set_limits_for_test(fixture_limits(false));

    for depth in [ColourDepth::Indexed256, ColourDepth::Ansi16] {
        app.colour_depth = depth;
        let buffer = render(&app, 120, 30);
        let too_deep = buffer
            .content()
            .iter()
            .filter(|cell| {
                let rgb = |c: Color| matches!(c, Color::Rgb(..));
                let indexed = |c: Color| matches!(c, Color::Indexed(_));
                rgb(cell.fg)
                    || rgb(cell.bg)
                    || (depth == ColourDepth::Ansi16 && (indexed(cell.fg) || indexed(cell.bg)))
            })
            .count();
        assert_eq!(too_deep, 0, "{depth:?}");

        // The selection is a background and nothing else, so it has to survive as one.
        let width = usize::from(buffer.area.width);
        let selected = buffer
            .content()
            .chunks(width)
            .find(|row| {
                row.iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .contains("claude-opus-5")
            })
            .expect("the selected model is on screen");
        assert!(
            selected.iter().any(|cell| cell.bg != Color::Reset),
            "{depth:?}: the selected row lost its background"
        );
    }

    // `NO_COLOR` outranks the depth: no colour means none, whatever the terminal can draw.
    app.no_color = true;
    let buffer = render(&app, 120, 30);
    assert!(buffer
        .content()
        .iter()
        .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset));
}

/// The named palette maps by meaning. Nearest-match alone puts the muted text and the borders
/// on one grey, and lets the alarm red drift to whatever is closest.
#[test]
fn sixteen_colours_keep_the_palettes_distinctions() {
    use crate::model::{CLOUD, CYAN, GREEN, RED, YELLOW};
    use crate::ui::theme::{downgrade, BORDER, MUTED};
    use crate::utils::ColourDepth::Ansi16;
    let mapped: Vec<Color> = [CYAN, GREEN, YELLOW, RED, CLOUD, MUTED, BORDER]
        .into_iter()
        .map(|c| downgrade(c, Ansi16, false))
        .collect();
    for (i, a) in mapped.iter().enumerate() {
        assert!(!matches!(a, Color::Rgb(..) | Color::Reset), "{a:?}");
        for b in &mapped[i + 1..] {
            assert_ne!(a, b, "two palette colours collapsed into one");
        }
    }
    assert_eq!(downgrade(RED, Ansi16, false), Color::LightRed);
    // White text over a background handed back to the terminal would vanish on a light theme.
    assert_eq!(downgrade(Color::White, Ansi16, false), Color::Reset);
}
