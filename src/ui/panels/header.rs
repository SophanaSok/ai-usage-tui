//! Top bar: range, last refresh, and collector health.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::model::{CYAN, RED, YELLOW};
use crate::ui::app::App;
use crate::ui::theme::MUTED;

pub fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " AI USAGE ",
            Style::default()
                .fg(Color::Black)
                .bg(CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            "LIVE PROVIDER MONITOR",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        coverage_span(app),
        Span::raw("   "),
        Span::styled(
            format!(
                "{}  {}  {} ",
                app.range.label(),
                app.last_refresh,
                app.status
            ),
            if app.degraded {
                Style::default().fg(RED).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            },
        ),
    ]))
    .style(Style::default().bg(Color::Rgb(10, 18, 24)));
    frame.render_widget(title, area);
}

/// How much of the visible spend actually carries a known price.
///
/// Cost provenance is what this project does that the alternatives do not, and until now it
/// lived entirely in an internal enum and one panel's title — a reader could take a total at
/// face value without ever learning it covered two thirds of their requests. It belongs where
/// the total is.
///
/// Below 100% it is yellow, because that is the case worth noticing.
fn coverage_span<'a>(app: &App) -> Span<'a> {
    match app.coverage().pct() {
        Some(pct) if pct >= 99.95 => {
            Span::styled("all priced".to_string(), Style::default().fg(MUTED))
        }
        Some(pct) => Span::styled(
            format!("{pct:.0}% priced"),
            Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
        ),
        None => Span::raw(""),
    }
}
