//! Individual sessions, most recently active first.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::model::{SessionTotals, CLOUD, CYAN, YELLOW};
use crate::ui::aggregate::format_duration;
use crate::ui::app::App;
use crate::ui::theme::{panel, MUTED};
use crate::utils::format_count;

pub fn draw_sessions(frame: &mut Frame, area: Rect, app: &App) {
    let sessions = app.sessions();
    // The title carries the scope. Two views that render identically and mean different things
    // is how a reader ends up reading one project's spend as the whole machine's.
    let title = match app.drilldown_project() {
        Some(project) => format!("SESSIONS · {project}"),
        None => "SESSIONS".to_string(),
    };
    let block = panel(&title, CYAN);

    if sessions.is_empty() {
        let empty = match app.drilldown_project() {
            Some(project) => format!(
                "No sessions for {project} in this range. Backspace goes back to the project list."
            ),
            None => "No sessions in this range. Only sources that record one — Claude Code — \
                     contribute here."
                .to_string(),
        };
        frame.render_widget(
            Paragraph::new(empty)
                .style(Style::default().fg(MUTED))
                .block(block),
            area,
        );
        return;
    }

    let rows = sessions.iter().enumerate().map(|(index, session)| {
        let style = if index == app.selected {
            Style::default().bg(Color::Rgb(37, 57, 67))
        } else {
            Style::default()
        };
        Row::new(vec![
            Cell::from(started(session)),
            Cell::from(if session.duration_secs() > 0 {
                format_duration(session.duration_secs())
            } else {
                String::new()
            }),
            Cell::from(project_label(session)),
            Cell::from(model_label(session)),
            Cell::from(format_count(session.tokens)),
            cost_cell(session),
            Cell::from(session.requests.to_string()),
        ])
        .style(style)
    });

    let header = Row::new(super::sorted_header(app, crate::ui::app::Panel::Sessions))
        .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));

    // `TableState` so a selection below the fold scrolls into view. Sessions accumulate without
    // bound — projects top out in dozens, this list only grows.
    let mut state = TableState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Length(11),
                Constraint::Length(7),
                Constraint::Min(14),
                Constraint::Length(16),
                Constraint::Length(8),
                Constraint::Length(11),
                Constraint::Length(5),
            ],
        )
        .header(header)
        .column_spacing(1)
        .block(block),
        area,
        &mut state,
    );
}

/// `08-19 14:02`, in local time — the same clock the rest of the dashboard uses.
fn started(session: &SessionTotals) -> String {
    use chrono::TimeZone;
    if session.first_seen <= 0 {
        return String::new();
    }
    match chrono::Local.timestamp_opt(session.first_seen, 0) {
        chrono::offset::LocalResult::Single(dt) => dt.format("%m-%d %H:%M").to_string(),
        _ => String::new(),
    }
}

/// The last path segment. The full working directory is what makes two `build` directories
/// distinguishable, but it is too wide for a column that also has to show time and cost.
fn project_label(session: &SessionTotals) -> String {
    session
        .project
        .as_deref()
        .map(|path| {
            path.rsplit(['/', '\\'])
                .find(|segment| !segment.is_empty())
                .unwrap_or(path)
                .to_string()
        })
        .unwrap_or_else(|| "—".to_string())
}

/// One model named, several counted. A session that used three models has no single answer, and
/// listing all of them would push the columns that carry numbers off the screen.
fn model_label(session: &SessionTotals) -> String {
    match session.models.as_slice() {
        [] => String::new(),
        [only] => only.rsplit('/').next().unwrap_or(only).to_string(),
        many => format!("{} models", many.len()),
    }
}

fn cost_cell<'a>(session: &SessionTotals) -> Cell<'a> {
    match (session.unpriced_requests, session.cost) {
        // Entirely quota-billed: real cost, no per-request price, never `$0.00`.
        (0, cost) if cost == 0.0 && session.quota_requests > 0 => {
            Cell::from(Span::styled("quota", Style::default().fg(CLOUD)))
        }
        (0, cost) => Cell::from(format!("${cost:.2}")),
        (_, cost) if cost > 0.0 => Cell::from(Span::styled(
            format!("≥ ${cost:.2}"),
            Style::default().fg(YELLOW),
        )),
        _ => Cell::from(Span::styled("unpriced", Style::default().fg(YELLOW))),
    }
}
