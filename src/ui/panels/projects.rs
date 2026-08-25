//! Per-project cost. Range-wide pricing coverage lives in the header.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Cell, Row, Table, TableState},
    Frame,
};

use crate::model::{CLOUD, CYAN, YELLOW};
use crate::ui::aggregate::project_labels;
use crate::ui::app::App;
use crate::ui::theme::{panel, MUTED};
use crate::utils::format_count;

pub fn draw_projects(frame: &mut Frame, area: Rect, app: &App) {
    let projects = app.projects();
    let paths: Vec<String> = projects.iter().map(|p| p.project.clone()).collect();
    let labels = project_labels(&paths);
    let rows = projects
        .iter()
        .zip(labels)
        .enumerate()
        .map(|(index, (p, label))| {
            // The highlight the model and session tables use. This table had none, so `j`/`k`
            // moved a cursor nothing drew — and `Enter` drilled into whatever row it had reached.
            let style = if index == app.selected {
                Style::default().bg(Color::Rgb(37, 57, 67))
            } else {
                Style::default()
            };
            // A project with unpriced requests gets its cost shown as a floor, not a total. The
            // never-render-unknown-cost-as-zero invariant applies to partial sums too.
            let cost = if p.unpriced_requests > 0 {
                Span::styled(format!("≥ ${:.2}", p.cost), Style::default().fg(YELLOW))
            } else if p.cost == 0.0 && p.quota_requests > 0 {
                // Entirely quota-billed: real cost, no per-request price, never `$0.00`.
                Span::styled("quota", Style::default().fg(CLOUD))
            } else {
                Span::raw(format!("${:.2}", p.cost))
            };
            Row::new(vec![
                Cell::from(label),
                Cell::from(format_count(p.tokens)),
                Cell::from(cost),
                Cell::from(p.requests.to_string()),
                Cell::from(p.sessions.to_string()),
            ])
            .style(style)
        });
    let header = Row::new(super::sorted_header(app, crate::ui::app::Panel::Projects))
        .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Min(24),
        Constraint::Length(11),
        Constraint::Length(12),
        Constraint::Length(7),
        Constraint::Length(6),
    ];
    // Coverage is a range-wide figure, not a per-project one, and the header now carries it on
    // every screen. Repeating it in this title invited reading it as "of this project's spend".
    let title = "PROJECT COST";
    // `TableState` so a selection below the fold scrolls into view, as the model table does. A
    // plain `render_widget` draws the first rows that fit and never the one the cursor is on.
    let mut state = TableState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(
        Table::new(rows, widths)
            .header(header)
            .column_spacing(1)
            .block(panel(title, CYAN)),
        area,
        &mut state,
    );
}
