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
use crate::utils::format_count;

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
    let coverage = app.coverage();
    // "all priced" while thousands of quota-billed requests sit outside the ratio is technically
    // true and unhelpful. A rate is only readable next to what it was taken over.
    let quota = match coverage.quota_requests {
        0 => String::new(),
        n => format!(" · {} on quota", format_count(n)),
    };
    match coverage.pct() {
        Some(pct) if pct >= 99.95 => {
            Span::styled(format!("all priced{quota}"), Style::default().fg(MUTED))
        }
        Some(pct) => Span::styled(
            format!("{pct:.0}% priced{quota}"),
            Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
        ),
        // Nothing billable in range, but there may still be quota work to disclose.
        None if coverage.quota_requests > 0 => Span::styled(
            quota.trim_start_matches(" · ").to_string(),
            Style::default().fg(MUTED),
        ),
        None => Span::raw(""),
    }
}
