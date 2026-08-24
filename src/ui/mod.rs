//! Terminal dashboard.
//!
//! Layout of this module, for anyone finding their way in:
//!
//! | file | holds |
//! |---|---|
//! | `app.rs` | `App` state, the `Panel` enum, and the derived views recomputed each refresh |
//! | `aggregate.rs` | pure functions over `Usage` — per-project totals, pricing coverage |
//! | `theme.rs` | palette and the small shared widgets (`panel`, `metric`, `cost_display`) |
//! | `panels/` | one module per panel, each exposing a single `draw_*` function |
//! | `svg.rs` | renders a frame to SVG off-screen, for the README images |
//! | this file | the event loop and the frame layout that dispatches to those panels |
//!
//! **To add a panel:** write `panels/yours.rs` with one `draw_yours(frame, area, app)`, add a
//! `Panel` variant, a key binding in `run`, and a match arm in `draw`. Nothing else needs to
//! know about it.
//!
//! Two invariants hold throughout. Nothing here reads the clock, opens a database, or performs
//! I/O — everything a panel needs is computed once per refresh into `DerivedView`, because this
//! code runs several times a second. And unknown cost is never rendered as `$0.00`; see
//! `cost_display`.

pub mod aggregate;
pub mod app;
pub mod keys;
pub mod panels;
pub mod svg;
pub mod theme;

#[cfg(test)]
mod tests;

use std::io;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame, Terminal,
};

use crate::budget::{Alert, AlertDispatcher, BudgetEngine};
use crate::cli::Cli;
use crate::collector::background::CollectorHandle;
use crate::collector::SourceRoots;
use crate::model::CYAN;
use crate::utils::journal_path;

pub use aggregate::{coverage, project_labels, project_totals};
pub use app::{App, Coverage, DerivedView, Panel};
pub use svg::{buffer_to_svg, render_svg};
pub use theme::cost_display;

use panels::{
    alerts::draw_alert_banner, breakdown::draw_breakdown, budgets::draw_budgets, burn::draw_burn,
    header::draw_header, limits::draw_limits, metrics::draw_metrics, models::draw_models,
    projects::draw_projects, routing::draw_routing, sessions::draw_sessions,
    timeseries::draw_timeseries,
};
use theme::{panel, MUTED};

pub fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cli: &Cli,
    collector: Option<CollectorHandle>,
    budget_engine: BudgetEngine,
    mut dispatcher: AlertDispatcher,
) -> Result<()> {
    let journal = cli
        .journal_path
        .clone()
        .or_else(journal_path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not determine a home directory; pass an explicit path (see --help)"
            )
        })?;
    // The dispatcher owns a blocking HTTP client; give it its own thread and talk to it over
    // a channel. Dropping the sender when `run` returns ends the worker.
    let alert_sink = dispatcher.webhook_url.is_some().then(|| {
        let (tx, rx) = mpsc::channel::<Vec<Alert>>();
        std::thread::spawn(move || {
            while let Ok(alerts) = rx.recv() {
                if let Err(error) = dispatcher.dispatch(&alerts) {
                    crate::logging::error("budget", &format!("webhook dispatch failed: {}", error));
                }
            }
        });
        tx
    });

    let mut app = App::new(
        SourceRoots::from_cli(cli, journal),
        cli.range,
        cli.refresh_interval,
        cli.provider_filter.clone(),
        cli.model_filter.clone(),
        collector,
        budget_engine,
        alert_sink,
    );
    loop {
        app.refresh_if_due();
        terminal.draw(|frame| draw(frame, &app))?;
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                // Ctrl-C first, and it works even mid-search: a user who cannot get out of a
                // text field is stuck in a full-screen program.
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }
                // While a `/` filter is being typed every printable key belongs to it, not to
                // the dashboard -- otherwise typing "budget" toggles four panels and quits.
                if app.is_typing_search() {
                    match key.code {
                        KeyCode::Esc => app.cancel_search(),
                        KeyCode::Enter => app.accept_search(),
                        KeyCode::Backspace => app.search_backspace(),
                        KeyCode::Char(c) => {
                            app.search_key(c);
                        }
                        _ => {}
                    }
                    app.pulse = app.pulse.wrapping_add(1);
                    continue;
                }
                let action = match key.code {
                    KeyCode::Char(c) => keys::action_for(c),
                    // Esc asks to go back, and `Action::Back` falls through to quitting when
                    // there is nowhere to go. That keeps the documented "Esc quits" true
                    // everywhere except inside a drilldown, where going back is what a reader
                    // means by it.
                    KeyCode::Esc => Some(keys::Action::Back),
                    KeyCode::Enter => Some(keys::Action::DrillIn),
                    KeyCode::Backspace => Some(keys::Action::Back),
                    KeyCode::Down => Some(keys::Action::SelectNext),
                    KeyCode::Up => Some(keys::Action::SelectPrev),
                    _ => None,
                };
                match action {
                    Some(keys::Action::Quit) => break,
                    Some(keys::Action::Search) => app.begin_search(),
                    Some(keys::Action::SortNext) => app.cycle_sort_column(true),
                    Some(keys::Action::SortPrev) => app.cycle_sort_column(false),
                    Some(keys::Action::SortReverse) => app.reverse_sort(),
                    Some(keys::Action::DrillIn) => {
                        app.drill_into_selected_project();
                    }
                    // Innermost thing first: clear a filter, then leave a drilldown, and only
                    // quit when there is nothing left to back out of.
                    Some(keys::Action::Back) => {
                        if app.search_status().is_some() {
                            app.cancel_search();
                        } else if !app.leave_drilldown() {
                            break;
                        }
                    }
                    Some(keys::Action::ToggleHelp) => app.show_help = !app.show_help,
                    Some(keys::Action::Refresh) => app.refresh(),
                    Some(keys::Action::Panel(panel)) => app.toggle_panel(panel),
                    Some(keys::Action::Range(range)) => app.set_range(range),
                    Some(keys::Action::SelectNext) => {
                        app.selected = (app.selected + 1).min(app.visible_rows().saturating_sub(1))
                    }
                    Some(keys::Action::SelectPrev) => app.selected = app.selected.saturating_sub(1),
                    None => {}
                }
            }
        }
        app.pulse = app.pulse.wrapping_add(1);
    }
    Ok(())
}

/// Lay out one frame and dispatch to the panel renderers.
pub(super) fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let alert_banner_height = if app.alerts.iter().any(|a| a.is_actionable()) {
        1u16
    } else {
        0u16
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(alert_banner_height),
            Constraint::Length(7),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);
    draw_header(frame, chunks[0], app);
    if alert_banner_height > 0 {
        draw_alert_banner(frame, chunks[1], app);
    }
    draw_metrics(frame, chunks[2], app);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
        .split(chunks[3]);
    draw_breakdown(frame, body[0], app);
    match app.panel {
        Panel::Routing => draw_routing(frame, body[1], app),
        Panel::Budgets => draw_budgets(frame, body[1], app),
        Panel::Projects => draw_projects(frame, body[1], app),
        Panel::TimeSeries => draw_timeseries(frame, body[1], app),
        Panel::Burn => draw_burn(frame, body[1], app),
        Panel::Sessions => draw_sessions(frame, body[1], app),
        Panel::Limits => draw_limits(frame, body[1], app),
        Panel::Models => draw_models(frame, body[1], app),
    }
    frame.render_widget(footer(area.width, app.search_status()), chunks[4]);
    if app.show_help {
        draw_help(frame, area);
    }
}

/// Key hints, sized to the terminal.
///
/// The full list is 120 columns. It fit an 80-column terminal until the graph, burn and sessions
/// panels were added, after which the tail — including how to quit — was simply cut off, because
/// a `Paragraph` truncates without saying so. Below 120 columns this shows a compact form and
/// leans on `?` for the rest.
pub(super) fn footer<'a>(width: u16, search: Option<(&str, usize, usize)>) -> Paragraph<'a> {
    let key = |k: &'a str| Span::styled(k, Style::default().fg(CYAN).add_modifier(Modifier::BOLD));

    // A filter replaces the hints while it is on. Rows disappearing with nothing on screen to
    // say why is the whole failure mode this line exists to prevent, and it carries the counts
    // so a shortened list is never mistaken for a shrunken bill.
    if let Some((query, shown, total)) = search {
        return Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" /{query}"),
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("   showing {shown} of {total} rows   ")),
            Span::styled("Enter", Style::default().fg(CYAN)),
            Span::raw(" keep  "),
            Span::styled("Esc", Style::default().fg(CYAN)),
            Span::raw(" clear"),
        ]))
        .style(Style::default().fg(MUTED));
    }
    let spans = if width >= 120 {
        vec![
            key(" 1-4 "),
            Span::raw("range  "),
            key("r"),
            Span::raw(" refresh  "),
            key("b"),
            Span::raw(" budgets  "),
            key("t"),
            Span::raw(" routing  "),
            key("p"),
            Span::raw(" projects  "),
            key("g"),
            Span::raw(" graph  "),
            key("w"),
            Span::raw(" burn  "),
            key("s"),
            Span::raw(" sessions  "),
            key("l"),
            Span::raw(" limits  "),
            key("j/k"),
            Span::raw(" move  "),
            key("?"),
            Span::raw(" help  "),
            key("q"),
            Span::raw(" quit"),
        ]
    } else {
        vec![
            key(" 1-4 "),
            Span::raw("range  "),
            key("r"),
            Span::raw(" refresh  "),
            key("btpgwsl"),
            Span::raw(" panels  "),
            key("j/k"),
            Span::raw(" move  "),
            key("?"),
            Span::raw(" help  "),
            key("q"),
            Span::raw(" quit"),
        ]
    };
    Paragraph::new(Line::from(spans)).style(Style::default().fg(MUTED))
}

/// Full key reference, centred over the dashboard.
///
/// Exists because there are more bindings than fit on one line, and truncating the line silently
/// is how `q quit` became invisible on an 80-column terminal.
fn draw_help(frame: &mut Frame, area: Rect) {
    let rows_source: Vec<(&str, &str)> = keys::rows().collect();

    let width = 56.min(area.width.saturating_sub(4));
    let height = (rows_source.len() as u16 + 2).min(area.height.saturating_sub(2));
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    let lines: Vec<Line> = rows_source
        .iter()
        .map(|(k, what)| {
            Line::from(vec![
                Span::styled(
                    format!("  {k:<9}"),
                    Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
                ),
                Span::raw(*what),
            ])
        })
        .collect();

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("KEYS", CYAN))
            .style(Style::default().fg(MUTED)),
        popup,
    );
}
