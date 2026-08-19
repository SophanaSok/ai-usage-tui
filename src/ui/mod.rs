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
pub mod panels;
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
use crate::model::{Range, CYAN};
use crate::utils::journal_path;

pub use aggregate::{coverage, project_labels, project_totals};
pub use app::{App, Coverage, DerivedView, Panel};
pub use theme::cost_display;

use panels::{
    alerts::draw_alert_banner, breakdown::draw_breakdown, budgets::draw_budgets, burn::draw_burn,
    header::draw_header, metrics::draw_metrics, models::draw_models, projects::draw_projects,
    routing::draw_routing, sessions::draw_sessions, timeseries::draw_timeseries,
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
                let _ = dispatcher.dispatch(&alerts);
            }
        });
        tx
    });

    let mut app = App::new(
        cli.db_path.clone(),
        journal,
        cli.claude_dir.clone(),
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
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('?') => app.show_help = !app.show_help,
                    KeyCode::Char('r') => app.refresh(),
                    KeyCode::Char('b') => app.toggle_panel(Panel::Budgets),
                    KeyCode::Char('t') => app.toggle_panel(Panel::Routing),
                    KeyCode::Char('p') => app.toggle_panel(Panel::Projects),
                    KeyCode::Char('g') => app.toggle_panel(Panel::TimeSeries),
                    KeyCode::Char('w') => app.toggle_panel(Panel::Burn),
                    KeyCode::Char('s') => app.toggle_panel(Panel::Sessions),
                    KeyCode::Char('1') => app.set_range(Range::Today),
                    KeyCode::Char('2') => app.set_range(Range::Week),
                    KeyCode::Char('3') => app.set_range(Range::Month),
                    KeyCode::Char('4') => app.set_range(Range::All),
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.selected = (app.selected + 1).min(app.visible_rows().saturating_sub(1))
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.selected = app.selected.saturating_sub(1)
                    }
                    _ => {}
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
        Panel::Models => draw_models(frame, body[1], app),
    }
    frame.render_widget(footer(area.width), chunks[4]);
    if app.show_help {
        draw_help(frame, area);
    }
}

/// Key hints, sized to the terminal.
///
/// The full list is 106 columns. It fit an 80-column terminal until the graph, burn and sessions
/// panels were added, after which the tail — including how to quit — was simply cut off, because
/// a `Paragraph` truncates without saying so. Below 110 columns this shows a compact form and
/// leans on `?` for the rest.
pub(super) fn footer<'a>(width: u16) -> Paragraph<'a> {
    let key = |k: &'a str| Span::styled(k, Style::default().fg(CYAN).add_modifier(Modifier::BOLD));
    let spans = if width >= 110 {
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
            key("j/k"),
            Span::raw(" navigate  "),
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
            key("btpgws"),
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
    const ROWS: &[(&str, &str)] = &[
        ("1 2 3 4", "range: today, 7 days, 30 days, all time"),
        ("r", "refresh now"),
        ("b", "budgets"),
        ("t", "routing analytics and derived escalations"),
        ("p", "cost per project"),
        ("g", "spend over time"),
        ("w", "burn rate and time to budget"),
        ("s", "sessions"),
        ("j / k", "move the selection (also arrow keys)"),
        ("?", "close this help"),
        ("q", "quit (also Esc, Ctrl-C)"),
    ];

    let width = 56.min(area.width.saturating_sub(4));
    let height = (ROWS.len() as u16 + 2).min(area.height.saturating_sub(2));
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    let lines: Vec<Line> = ROWS
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
