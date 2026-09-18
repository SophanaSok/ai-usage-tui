//! The hero row: a tile per category, and under them one strip showing how the tokens divide.
//!
//! The tiles used to be seven rows tall for two lines of text, and four of the six spent their
//! second line repeating the first (`3.3M` over `3.3M tokens`). The second line now says what the
//! first cannot: the category's share of the whole and how many requests made it.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `keys.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::model::{Category, Totals, CYAN};
use crate::ui::aggregate::{share_cells, share_label};
use crate::ui::app::App;
use crate::ui::theme::{metric, PANEL};
use crate::utils::format_count;

/// Rows the tiles take: a border, the figure, the subtitle, a border.
const TILE_HEIGHT: u16 = 4;

pub fn draw_metrics(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(TILE_HEIGHT), Constraint::Min(0)])
        .split(area);
    draw_tiles(frame, rows[0], app);
    if rows[1].height > 0 {
        draw_share_strip(frame, rows[1], app);
    }
}

fn draw_tiles(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.totals();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(24),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(16),
        ])
        .split(area);
    let total = metric(
        "TOTAL TOKENS",
        format_count(t.tokens()),
        CYAN,
        format!("{} requests", t.requests),
    );
    frame.render_widget(total, cols[0]);
    for (i, (category, cat)) in app.category_totals().iter().enumerate() {
        frame.render_widget(
            metric(
                category.label(),
                format_count(cat.tokens()),
                category.color(),
                subtitle(*category, cat, t.tokens()),
            ),
            cols[i + 1],
        );
    }
}

/// A tile's second line.
fn subtitle(category: Category, cat: &Totals, all_tokens: u64) -> String {
    if category == Category::Paid && cat.requests > 0 {
        // Subscription work is PAID work with no per-request dollars; a zero here would
        // contradict the tile's own label.
        return if cat.cost == 0.0 && cat.quota_requests > 0 && cat.unknown_requests == 0 {
            format!("{} on quota", format_count(cat.quota_requests))
        } else {
            format!("${:.4}", cat.cost)
        };
    }
    match share_label(cat.tokens(), all_tokens) {
        Some(share) => format!("{share} · {} req", format_count(cat.requests)),
        // No tokens is not a share of zero percent; it is no share.
        None => "—".to_string(),
    }
}

/// One row, as wide as the tiles, divided between the categories by tokens.
///
/// Each segment is its category's colour with the category's name inside it when there is room.
/// The name is what survives `NO_COLOR`, and the thin rule at the start of every segment but the
/// first is what keeps two neighbours apart once the colours are gone.
fn draw_share_strip(frame: &mut Frame, area: Rect, app: &App) {
    // Inset to sit under the tiles' borders rather than the screen edge.
    let width = area.width.saturating_sub(2);
    let categories = app.category_totals();
    let weights: Vec<u64> = categories.iter().map(|(_, t)| t.tokens()).collect();
    let cells = share_cells(&weights, width);
    if cells.is_empty() {
        return;
    }
    let whole: u64 = weights.iter().sum();
    let mut spans = vec![Span::raw(" ")];
    let mut first = true;
    for ((category, totals), cells) in categories.iter().zip(cells) {
        if cells == 0 {
            continue;
        }
        let style = Style::default()
            .fg(Color::Black)
            .bg(category.color())
            .add_modifier(Modifier::BOLD);
        spans.push(Span::styled(
            segment_text(*category, totals.tokens(), whole, cells, !first),
            style,
        ));
        first = false;
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(PANEL)),
        area,
    );
}

/// What one segment holds: a leading rule unless it is the first, then the widest of
/// `LOCAL 21%`, `LOCAL` and nothing that fits, padded to exactly `cells`.
fn segment_text(category: Category, tokens: u64, whole: u64, cells: u16, rule: bool) -> String {
    let cells = usize::from(cells);
    let lead = if rule { "▏" } else { " " };
    let share = share_label(tokens, whole).unwrap_or_default();
    let candidates = [
        format!("{lead}{} {share}", category.label()),
        format!("{lead}{}", category.label()),
        lead.to_string(),
    ];
    let text = candidates
        .into_iter()
        .find(|text| text.chars().count() <= cells)
        .unwrap_or_default();
    format!("{text:<cells$}")
}
