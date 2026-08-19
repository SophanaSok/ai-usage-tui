//! Budget limits and current spend against them.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Cell, Paragraph, Row, Table},
    Frame,
};

use crate::budget::AlertLevel;
use crate::model::{CYAN, RED, YELLOW};
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
        let (spend, pct, level_str, color) = if let Some(alert) = alert {
            let c = match alert.level {
                AlertLevel::Warn => YELLOW,
                AlertLevel::Critical | AlertLevel::Exceeded => RED,
                AlertLevel::Ok => MUTED,
            };
            (alert.spend, alert.pct, alert.level.label(), c)
        } else {
            (0.0, 0.0, "OK", MUTED)
        };
        Row::new(vec![
            Cell::from(budget.scope.label().to_string()),
            Cell::from(format!("{:?}", budget.period).to_lowercase()),
            Cell::from(format!("${:.2}", spend)),
            Cell::from(format!("${:.2}", budget.limit)),
            Cell::from(format!("{}%", pct as u64)),
            Cell::from(level_str).style(Style::default().fg(color)),
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
