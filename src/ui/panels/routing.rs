//! Routing analytics: cost and quality per agent/model.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Cell, Paragraph, Row, Table},
    Frame,
};

use crate::model::CYAN;
use crate::ui::app::App;
use crate::ui::theme::{panel, MUTED};
use crate::utils::format_count;

pub fn draw_routing(frame: &mut Frame, area: Rect, app: &App) {
    let aggregates = app.routing();
    if aggregates.is_empty() {
        frame.render_widget(
            Paragraph::new("No routing events recorded.\nUse --record-routing to capture.")
                .style(Style::default().fg(MUTED))
                .block(panel("ROUTING", CYAN)),
            area,
        );
        return;
    }
    let table_rows = aggregates.iter().map(|agg| {
        Row::new(vec![
            Cell::from(agg.agent.clone()),
            Cell::from(agg.model.clone()),
            Cell::from(format_count(agg.tokens)),
            Cell::from(format!("${:.4}", agg.cost)),
            Cell::from(format!("{:.0}%", crate::routing::retry_rate(agg))),
            Cell::from(format!("{:.0}%", crate::routing::defect_rate(agg))),
            Cell::from(agg.tasks.to_string()),
        ])
    });
    let header = Row::new(vec![
        "AGENT", "MODEL", "TOKENS", "COST", "RETRY%", "DEFECTS", "TASKS",
    ])
    .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Min(18),
        Constraint::Min(20),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(7),
    ];
    frame.render_widget(
        Table::new(table_rows, widths)
            .header(header)
            .column_spacing(1)
            .block(panel("ROUTING", CYAN)),
        area,
    );
}
