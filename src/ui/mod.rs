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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    backend::Backend,
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
use crate::model::{CYAN, RED};
use crate::utils::journal_path;

pub use aggregate::{coverage, project_labels, project_totals};
pub use app::{App, Coverage, DerivedView, Flow, Panel};
pub use svg::{buffer_to_svg, render_svg};
pub use theme::cost_display;

use panels::{
    alerts::draw_alert_banner, breakdown::draw_breakdown, budgets::draw_budgets, burn::draw_burn,
    header::draw_header, limits::draw_limits, metrics::draw_metrics, models::draw_models,
    projects::draw_projects, routing::draw_routing, sessions::draw_sessions, tabs::draw_tabs,
    timeseries::draw_timeseries,
};
use theme::{panel, MUTED};

/// Run the dashboard until the user quits or `stop` is raised.
///
/// `stop` is how a termination signal reaches the loop: the caller's handler only sets the flag,
/// and the loop checks it at least every 250ms, so a `kill` leaves through the same exit as `q`
/// and the terminal is restored rather than left in raw mode on the alternate screen.
pub fn run<B>(
    terminal: &mut Terminal<B>,
    cli: &Cli,
    collector: Option<Arc<CollectorHandle>>,
    budget_engine: BudgetEngine,
    mut dispatcher: AlertDispatcher,
    stop: &AtomicBool,
) -> Result<()>
where
    B: Backend,
    B::Error: Send + Sync + 'static,
{
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
    while !stop.load(Ordering::Relaxed) {
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
                if let Some(action) = action {
                    if app.apply(action) == app::Flow::Quit {
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

/// The fewest rows the dashboard lays out in: header 1, tab strip 1, hero row 5, body 11,
/// footer 2 -- plus one for the alert banner while a budget alert is actionable
/// (`required_height`).
///
/// Below it ratatui does not complain -- it squeezes the constraints, and panels collapse to zero
/// height one after another in silence, which on a short pane reads as a dashboard with nothing in
/// it rather than a dashboard with no room. Width has no such floor: the footer is measured and
/// reflows down to 16 columns, and a test sweeps every width.
pub const MIN_HEIGHT: u16 = 20;

/// Lay out one frame and dispatch to the panel renderers.
pub(super) fn draw(frame: &mut Frame, app: &App) {
    draw_in_colour(frame, app);
    if app.no_color {
        strip_colour(frame.buffer_mut());
    } else if app.colour_depth != crate::utils::ColourDepth::TrueColour {
        downgrade_colour(frame.buffer_mut(), app.colour_depth);
    }
}

/// The palette, mapped down to what the terminal draws -- the same single pass as
/// `strip_colour`, for the same reason: a branch in every panel's styles is a branch the next
/// panel forgets, and a pass over the finished frame cannot be forgotten.
fn downgrade_colour(buffer: &mut ratatui::buffer::Buffer, depth: crate::utils::ColourDepth) {
    for cell in buffer.content.iter_mut() {
        cell.fg = theme::downgrade(cell.fg, depth, false);
        cell.bg = theme::downgrade(cell.bg, depth, true);
    }
}

/// Everything colour is, removed after the frame is drawn -- one pass here rather than a branch
/// in every panel's styles, which is how a new panel would have come to ignore `NO_COLOR`.
/// Bold, reverse and the rest stay: they are not colour. The one thing colour alone carried is
/// the selected row, drawn as a background, so that becomes reverse video.
fn strip_colour(buffer: &mut ratatui::buffer::Buffer) {
    use ratatui::style::Color;
    for cell in buffer.content.iter_mut() {
        if cell.bg == theme::SELECTED {
            cell.modifier.insert(Modifier::REVERSED);
        }
        cell.fg = Color::Reset;
        cell.bg = Color::Reset;
    }
}

/// Whether a budget alert is showing, which costs the layout its banner row.
fn has_alert_banner(app: &App) -> bool {
    app.alerts.iter().any(|a| a.is_actionable())
}

/// `MIN_HEIGHT`, plus the banner row when there is one. Checking the bare constant let a 20-row pane
/// with an alert run the full layout one row short, squeezing the body below its minimum -- the
/// silent collapse the check exists to prevent (found in review of #104).
fn required_height(app: &App) -> u16 {
    MIN_HEIGHT + u16::from(has_alert_banner(app))
}

fn draw_in_colour(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.height < required_height(app) {
        draw_too_short(frame, area, app);
        return;
    }
    let alert_banner_height = u16::from(has_alert_banner(app));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(alert_banner_height),
            Constraint::Length(5),
            Constraint::Min(11),
            Constraint::Length(2),
        ])
        .split(area);
    draw_header(frame, chunks[0], app);
    draw_tabs(frame, chunks[1], app);
    if alert_banner_height > 0 {
        draw_alert_banner(frame, chunks[2], app);
    }
    draw_metrics(frame, chunks[3], app);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(rail_width(area.width)),
            Constraint::Min(0),
        ])
        .split(chunks[4]);
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
    frame.render_widget(footer(area.width, app.search_status()), chunks[5]);
    if app.show_help {
        draw_help(frame, area);
    }
}

/// The left rail's width: a little over a third of the screen, and no more than its contents
/// use. It was a flat 36%, which on a wide terminal handed a nine-line list fifty columns and
/// took them from the table beside it -- the pane that does have more to show.
fn rail_width(width: u16) -> u16 {
    const WIDEST: u16 = 40;
    (u32::from(width) * 36 / 100).min(u32::from(WIDEST)) as u16
}

/// What a pane shorter than `required_height` shows: why nothing else is there, that a budget
/// alert is active if one is -- a too-short pane must not be how an alert goes unseen -- and, on the
/// last line as always, how to leave.
fn draw_too_short(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines = vec![
        Line::from(Span::styled(
            "Terminal too short for the dashboard",
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("needs {} rows, has {}", required_height(app), area.height),
            Style::default().fg(MUTED),
        )),
    ];
    if has_alert_banner(app) {
        lines.push(Line::from(Span::styled(
            "A budget alert is active.",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )));
    }
    let message = Paragraph::new(lines);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    frame.render_widget(message, rows[0]);
    frame.render_widget(footer(area.width, app.search_status()), rows[1]);
}

/// Key hints, sized to the terminal.
///
/// The full list is wider than an 80-column terminal, and a `Paragraph` truncates without saying
/// so: when the graph, burn and sessions panels were added the tail — including how to quit —
/// was simply cut off. The forms come from `keys::footer_forms`, widest first, and this takes
/// the first that measures as fitting. There used to be a `width >= 120` here instead, which
/// was the full line's width on the day it was written and a copy of the table's contents in
/// disguise.
pub(super) fn footer<'a>(width: u16, search: Option<(&str, usize, usize)>) -> Paragraph<'a> {
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
    let forms = keys::footer_forms();
    let mut lines = forms.iter().map(|form| hint_line(form));
    // The narrowest form is the fallback whatever the width: below it there is nothing shorter
    // to say than how to get help and how to leave.
    let last = lines
        .next_back()
        .expect("footer_forms has a narrowest form");
    let line = lines
        .find(|line| line.width() <= usize::from(width))
        .unwrap_or(last);
    Paragraph::new(line).style(Style::default().fg(MUTED))
}

/// ` k word  k word  …`, the key in the accent colour.
fn hint_line<'a>(hints: &[keys::Hint]) -> Line<'a> {
    let mut spans = vec![Span::raw(" ")];
    for (index, hint) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            hint.key.clone(),
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(format!(" {}", hint.word)));
    }
    Line::from(spans)
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
