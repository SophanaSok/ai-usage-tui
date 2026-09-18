//! Spend and tokens over time, one bar per local calendar day.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Cell, Paragraph, RenderDirection, Row, Sparkline, Table},
    Frame,
};

use crate::model::{DayTotals, CLOUD, CYAN, YELLOW};
use crate::ui::app::App;
use crate::ui::theme::{panel, MUTED};
use crate::utils::format_count;

pub fn draw_timeseries(frame: &mut Frame, area: Rect, app: &App) {
    let days = app.daily();
    let block = panel("SPEND OVER TIME", CYAN);

    if days.is_empty() {
        frame.render_widget(
            Paragraph::new("No dated usage in this range.")
                .style(Style::default().fg(MUTED))
                .block(block),
            area,
        );
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Sparkline for shape, table for the numbers. A sparkline alone shows a trend but not what
    // anything cost, which is the question this tool exists to answer.
    let rows_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);

    frame.render_widget(spend_sparkline(days), rows_area[0]);
    frame.render_widget(day_table(days, rows_area[1].height as usize), rows_area[1]);
}

/// Daily spend as a sparkline, in whole cents, with today at the right edge.
///
/// Cents rather than dollars because `Sparkline` takes integers, and rounding to dollars would
/// flatten every day under a dollar to zero.
///
/// Drawn right-to-left from the newest day so that time runs left to right and "now" sits at
/// the right edge, where a reader looks first. It also makes truncation correct: when the range
/// is wider than the panel it drops the oldest days rather than the most recent ones.
fn spend_sparkline(days: &[DayTotals]) -> Sparkline<'_> {
    let cents: Vec<u64> = days
        .iter()
        .rev()
        .map(|d| (d.cost * 100.0).round().max(0.0) as u64)
        .collect();
    Sparkline::default()
        .data(cents)
        .direction(RenderDirection::RightToLeft)
        .style(Style::default().fg(CYAN))
}

/// Daily tokens as a sparkline, newest at the right edge, for the rail. Same direction and the
/// same truncation rule as `spend_sparkline`: a range wider than the pane loses its oldest days.
pub(crate) fn tokens_sparkline(days: &[DayTotals]) -> Sparkline<'_> {
    let tokens: Vec<u64> = days.iter().rev().map(|d| d.tokens).collect();
    Sparkline::default()
        .data(tokens)
        .direction(RenderDirection::RightToLeft)
        .style(Style::default().fg(CYAN))
}

fn day_table(days: &[DayTotals], height: usize) -> Table<'_> {
    // Newest last is how a chart reads, but a table is scanned from the top, so show the most
    // recent days first and only as many as fit — this panel does not scroll.
    let visible = height.saturating_sub(1).max(1);
    let peak = days.iter().map(|d| d.cost).fold(0.0_f64, f64::max);

    let rows = days.iter().rev().take(visible).map(|day| {
        // Same rule as every other cost in this dashboard: a partial total is never presented
        // as a complete one. A day with *no* priced usage is not "at least $0.00" — that is
        // technically true and tells the reader nothing — so it says plainly that it is
        // unpriced.
        let cost = match (day.unpriced_requests, day.cost) {
            // All of this day's work is billed on a plan, so there is no dollar figure to show.
            // Rendering the `$0.00` that the arithmetic produces would be the exact failure this
            // dashboard exists to avoid.
            (0, cost) if cost == 0.0 && day.quota_requests > 0 => {
                Cell::from(Span::styled("quota", Style::default().fg(CLOUD)))
            }
            (0, cost) => Cell::from(format!("${cost:.2}")),
            (_, cost) if cost > 0.0 => Cell::from(Span::styled(
                format!("≥ ${cost:.2}"),
                Style::default().fg(YELLOW),
            )),
            _ => Cell::from(Span::styled("unpriced", Style::default().fg(YELLOW))),
        };
        Row::new(vec![
            Cell::from(day.day.clone()),
            Cell::from(Span::styled(bar(day.cost, peak), Style::default().fg(CYAN))),
            Cell::from(format_count(day.tokens)),
            cost,
            Cell::from(day.requests.to_string()),
        ])
    });

    let header = Row::new(vec!["DAY", "", "TOKENS", "COST", "REQS"])
        .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));

    Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(9),
            Constraint::Length(11),
            Constraint::Length(6),
        ],
    )
    .header(header)
    .column_spacing(1)
}

/// A twelve-cell bar scaled to the busiest day. See `theme::bar_of` for the glyph rules.
pub(crate) fn bar(value: f64, peak: f64) -> String {
    crate::ui::theme::bar_of(value, peak, 12)
}
