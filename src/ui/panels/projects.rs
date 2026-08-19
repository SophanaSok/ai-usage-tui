//! Per-project cost, with a pricing-coverage figure in the title.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Cell, Row, Table},
    Frame,
};

use crate::model::{CYAN, YELLOW};
use crate::ui::aggregate::project_labels;
use crate::ui::app::App;
use crate::ui::theme::{panel, MUTED};
use crate::utils::format_count;

pub fn draw_projects(frame: &mut Frame, area: Rect, app: &App) {
    let projects = app.projects();
    let paths: Vec<String> = projects.iter().map(|p| p.project.clone()).collect();
    let labels = project_labels(&paths);
    let rows = projects.iter().zip(labels).map(|(p, label)| {
        // A project with unpriced requests gets its cost shown as a floor, not a total. The
        // never-render-unknown-cost-as-zero invariant applies to partial sums too.
        let cost = if p.unpriced_requests > 0 {
            Span::styled(format!("≥ ${:.2}", p.cost), Style::default().fg(YELLOW))
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
    });
    let header = Row::new(vec!["PROJECT", "TOKENS", "COST", "REQS", "SESS"])
        .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Min(24),
        Constraint::Length(11),
        Constraint::Length(12),
        Constraint::Length(7),
        Constraint::Length(6),
    ];
    let title = match app.coverage().pct() {
        Some(pct) => format!("PROJECT COST  ({:.0}% priced)", pct),
        None => "PROJECT COST".to_string(),
    };
    frame.render_widget(
        Table::new(rows, widths)
            .header(header)
            .column_spacing(1)
            .block(panel(&title, CYAN)),
        area,
    );
}
