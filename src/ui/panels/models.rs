//! Per-model activity table — the default right-hand panel.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};

use crate::model::CYAN;
use crate::ui::app::App;
use crate::ui::theme::{cost_display, panel, MUTED};
use crate::utils::format_count;

pub fn draw_models(frame: &mut Frame, area: Rect, app: &App) {
    let rows = app.rows();
    // A `/` filter that matches nothing keeps the table: the footer already says "showing 0 of N".
    if rows.is_empty() && app.search_status().is_none() {
        frame.render_widget(empty_state(app).block(panel("MODEL ACTIVITY", CYAN)), area);
        return;
    }
    let table_rows = rows.iter().enumerate().map(|(index, u)| {
        let style = if index == app.selected {
            Style::default().bg(crate::ui::theme::SELECTED)
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
    let header = Row::new(super::sorted_header(app, crate::ui::app::Panel::Models))
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

/// What the default panel says when it has no rows, and what to do about it.
///
/// This is the first screen of every new install on a machine with no agent logs yet, and of
/// every install whose sources live somewhere unexpected. It used to be a header row over nothing
/// and tiles reading `0`, which looks like a working dashboard with nothing to report -- the one
/// reading of an empty screen that is least likely to be true. Two cases, because the fixes differ.
fn empty_state(app: &App) -> Paragraph<'static> {
    let dim = Style::default().fg(MUTED);
    let key = Style::default().fg(CYAN).add_modifier(Modifier::BOLD);
    let lines = if app.usages.is_empty() {
        vec![
            Line::from(Span::styled("No usage collected yet.", key)),
            Line::from(""),
            Line::from(Span::styled(
                "Sources are read as they are found, so this can take a poll. If it stays empty, run",
                dim,
            )),
            Line::from(vec![
                Span::styled("ai-usage-tui --doctor", key),
                Span::styled(
                    " to see which sources were looked for, where, and what each found.",
                    dim,
                ),
            ]),
        ]
    } else {
        let mut scope = app.range.label();
        if let Some(provider) = &app.provider_filter {
            scope.push_str(&format!(", provider {provider}"));
        }
        if let Some(model) = &app.model_filter {
            scope.push_str(&format!(", model {model}"));
        }
        vec![
            Line::from(Span::styled(format!("Nothing in {scope}."), key)),
            Line::from(""),
            if app.range == crate::model::Range::All {
                // Already all time, so only a `--provider` / `--model` filter can empty it.
                Line::from(Span::styled(
                    format!(
                        "{} usage records were collected; the --provider / --model filter excludes all of them.",
                        app.usages.len()
                    ),
                    dim,
                ))
            } else {
                Line::from(vec![
                    Span::styled(
                        format!(
                            "{} usage records were collected in all. Press ",
                            app.usages.len()
                        ),
                        dim,
                    ),
                    Span::styled("4", key),
                    Span::styled(" for all time.", dim),
                ])
            },
        ]
    };
    Paragraph::new(lines).wrap(Wrap { trim: false })
}
