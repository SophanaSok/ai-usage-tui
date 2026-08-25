//! Budget limits and current spend against them.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Cell, Paragraph, Row, Table},
    Frame,
};

use crate::budget::{Alert, AlertLevel};
use crate::model::{CLOUD, CYAN, RED, YELLOW};
use crate::ui::app::App;
use crate::ui::theme::{panel, MUTED};

pub fn draw_budgets(frame: &mut Frame, area: Rect, app: &App) {
    let budgets = app.budget_engine.budgets();
    if budgets.is_empty() {
        frame.render_widget(
            Paragraph::new("No budgets configured.\nAdd [[budgets.entry]] to your config.toml.")
                .style(Style::default().fg(MUTED))
                .block(panel("BUDGETS", CYAN)),
            area,
        );
        return;
    }
    let alerts_map: std::collections::HashMap<_, _> = app
        .alerts
        .iter()
        .map(|a| ((a.scope.clone(), a.period), a))
        .collect();
    let table_rows = budgets.iter().map(|budget| {
        let alert = alerts_map.get(&(budget.scope.clone(), budget.period));
        let (spend, pct, status) = match alert {
            Some(alert) => {
                let colour = match alert.level {
                    AlertLevel::Warn => YELLOW,
                    AlertLevel::Critical | AlertLevel::Exceeded => RED,
                    AlertLevel::Ok => MUTED,
                };
                (
                    spend_cell(alert),
                    pct_cell(alert),
                    Cell::from(alert.level.label()).style(Style::default().fg(colour)),
                )
            }
            // Alerts are computed in `refresh`; before the first one this budget has no figure
            // yet, and "$0.00 / 0% / OK" is a claim about it that nothing has checked.
            None => (
                Cell::from(Span::styled("—", Style::default().fg(MUTED))),
                Cell::from(Span::styled("—", Style::default().fg(MUTED))),
                Cell::from(Span::styled("—", Style::default().fg(MUTED))),
            ),
        };
        Row::new(vec![
            Cell::from(budget.scope.label().to_string()),
            Cell::from(budget.period.label()),
            spend,
            Cell::from(format!("${:.2}", budget.limit)),
            pct,
            status,
        ])
    });
    let header = Row::new(vec!["SCOPE", "PERIOD", "SPEND", "LIMIT", "PCT", "STATUS"])
        .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(10),
    ];
    frame.render_widget(
        Table::new(table_rows, widths)
            .header(header)
            .column_spacing(1)
            .block(panel("BUDGETS", CYAN)),
        area,
    );
}

/// The spend, in the vocabulary every other panel uses for a figure that is not the whole
/// story: `on quota` when the period's work is all plan quota, `≥ $x` when some of it should
/// carry a price and does not. A bare `$0.00` beside `OK` reads as "untouched", which on a
/// subscription account is false for every request.
fn spend_cell<'a>(alert: &Alert) -> Cell<'a> {
    if alert.is_quota_only() {
        Cell::from(Span::styled("on quota", Style::default().fg(CLOUD)))
    } else if alert.is_partial() {
        Cell::from(Span::styled(
            format!("≥ ${:.2}", alert.spend),
            Style::default().fg(YELLOW),
        ))
    } else {
        Cell::from(format!("${:.2}", alert.spend))
    }
}

/// The percentage carries the same marker as the spend it was taken from.
fn pct_cell<'a>(alert: &Alert) -> Cell<'a> {
    if alert.is_quota_only() {
        Cell::from(Span::styled("—", Style::default().fg(CLOUD)))
    } else if alert.is_partial() {
        Cell::from(Span::styled(
            format!("≥ {}%", alert.pct as u64),
            Style::default().fg(YELLOW),
        ))
    } else {
        Cell::from(format!("{}%", alert.pct as u64))
    }
}
