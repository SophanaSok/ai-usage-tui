//! Colours and small shared widgets.
//!
//! Everything visual that more than one panel needs lives here, so a new panel does not have
//! to rediscover the palette or re-derive how a bordered box is built.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::model::{CostStatus, Usage};

pub const MUTED: Color = Color::Rgb(125, 145, 160);
pub const PANEL: Color = Color::Rgb(18, 28, 37);

pub fn panel<'a>(title: &'a str, color: Color) -> Block<'a> {
    Block::default()
        .title(Span::styled(
            format!(" {} ", title),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(48, 72, 84)))
        .style(Style::default().bg(PANEL))
}

pub fn metric<'a>(label: &'a str, value: String, color: Color, subtitle: String) -> Paragraph<'a> {
    Paragraph::new(vec![
        Line::from(Span::styled(
            value,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(subtitle, Style::default().fg(MUTED))),
    ])
    .block(panel(label, color))
    .style(Style::default().fg(Color::White))
}

pub fn cost_display(usage: &Usage) -> String {
    match usage.cost_status {
        CostStatus::Local => "LOCAL".into(),
        CostStatus::Free => "FREE".into(),
        CostStatus::ProviderReported => usage
            .cost
            .map(|cost| format!("${:.4} reported", cost))
            .unwrap_or_else(|| "REPORTED / NO COST".into()),
        CostStatus::Calculated => usage
            .cost
            .map(|cost| format!("${:.4} calculated", cost))
            .unwrap_or_else(|| "CALCULATED / NO COST".into()),
        CostStatus::Estimated => usage
            .cost
            .map(|cost| format!("${:.4} estimated", cost))
            .unwrap_or_else(|| "ESTIMATED / NO COST".into()),
        // Not "$0.00" and not "FREE": this usage costs money, it is simply billed against a
        // plan rather than per token. Eight characters so it is not truncated by the COST
        // column's width.
        CostStatus::Quota => "ON QUOTA".into(),
        CostStatus::Unavailable => "UNKNOWN COST".into(),
    }
}

/// The figure `cost_display` puts in the cell, or `None` where it shows no figure at all.
///
/// Lives beside `cost_display` because the two have to agree. Sorting a COST column by a number
/// the cell never shows is how `ON QUOTA` ends up ranked as the cheapest work on the machine --
/// the cell saying the cost is unknown while the ordering says it is $0.00, about the same row.
///
/// `Free` and `Local` genuinely are zero and sort as zero. `Quota` and `Unavailable` are costs
/// this tool refuses to invent, and refusing to invent one is not the same as knowing it is zero.
/// Matched exhaustively on purpose: a new `CostStatus` should not be able to acquire a sort
/// position by falling through a wildcard.
pub fn cost_sort_key(usage: &Usage) -> Option<f64> {
    match usage.cost_status {
        CostStatus::Free | CostStatus::Local => Some(0.0),
        CostStatus::Quota | CostStatus::Unavailable => None,
        CostStatus::ProviderReported | CostStatus::Calculated | CostStatus::Estimated => usage.cost,
    }
}
