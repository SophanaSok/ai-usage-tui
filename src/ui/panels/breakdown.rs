//! Token and cost breakdown by category.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem},
    Frame,
};

use crate::model::{CYAN, YELLOW};
use crate::ui::app::App;
use crate::ui::theme::{panel, MUTED};
use crate::utils::format_count;

pub fn draw_breakdown(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.totals();
    let rows = vec![
        ListItem::new(Line::from(vec![
            Span::styled("INPUT       ", Style::default().fg(MUTED)),
            Span::styled(format_count(t.input), Style::default().fg(Color::White)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("OUTPUT      ", Style::default().fg(MUTED)),
            Span::styled(format_count(t.output), Style::default().fg(Color::White)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("REASONING   ", Style::default().fg(MUTED)),
            Span::styled(format_count(t.reasoning), Style::default().fg(Color::White)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("CACHE READ  ", Style::default().fg(MUTED)),
            Span::styled(
                format_count(t.cache_read),
                Style::default().fg(Color::White),
            ),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("CACHE WRITE ", Style::default().fg(MUTED)),
            Span::styled(
                format_count(t.cache_write),
                Style::default().fg(Color::White),
            ),
        ])),
        ListItem::new(Line::from("")),
        ListItem::new(Line::from(vec![
            Span::styled("EST. PAID COST ", Style::default().fg(YELLOW)),
            Span::styled(
                format!("${:.4}", t.cost),
                Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
            ),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("PRICING STATUS  ", Style::default().fg(MUTED)),
            // "complete" must not absorb quota-billed work: it is accounted for, but it
            // contributes no dollars to the total shown above.
            Span::raw(match (t.unknown_requests, t.quota_requests) {
                (0, 0) => "complete".to_string(),
                (0, quota) => format!("complete · {} on quota", crate::utils::format_count(quota)),
                _ => "partial / unknown".to_string(),
            }),
        ])),
    ];
    frame.render_widget(List::new(rows).block(panel("TOKEN FLOW", CYAN)), area);
}
