//! Per-model activity table — the default right-hand panel.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Cell, Row, Table, TableState},
    Frame,
};

use crate::model::CYAN;
use crate::ui::app::App;
use crate::ui::theme::{cost_display, panel, MUTED};
use crate::utils::format_count;

pub fn draw_models(frame: &mut Frame, area: Rect, app: &App) {
    let rows = app.rows();
    let table_rows = rows.iter().enumerate().map(|(index, u)| {
        let style = if index == app.selected {
            Style::default().bg(Color::Rgb(37, 57, 67))
        } else {
            Style::default()
        };
        Row::new(vec![
            Cell::from(format!("{} / {}", u.provider, u.model)),
            Cell::from(u.category.label()),
            Cell::from(format_count(u.total_tokens())),
            Cell::from(cost_display(u)),
            Cell::from(u.requests.to_string()),
        ])
        .style(style)
    });
    let header = Row::new(vec!["PROVIDER / MODEL", "CLASS", "TOKENS", "COST", "REQS"])
        .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Min(24),
        Constraint::Length(9),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Length(7),
    ];
    // A plain `render_widget` has no viewport offset, so a selection below the fold simply
    // vanished. `TableState` scrolls the viewport to keep it visible.
    let mut state = TableState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(
        Table::new(table_rows, widths)
            .header(header)
            .column_spacing(1)
            .block(panel("MODEL ACTIVITY", CYAN)),
        area,
        &mut state,
    );
}
