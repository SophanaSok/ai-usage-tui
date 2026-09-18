//! The left rail: what the tokens were, how full the subscription windows are, and the shape of
//! the range day by day.
//!
//! This was one nine-line list in a pane a third of the screen wide, with everything under it
//! empty. The list is unchanged; the room under it now holds two things a reader otherwise had to
//! leave the default view to see, both already computed into `DerivedView` on refresh.
//!
//! The sections stack, and one that does not fit whole is not drawn -- decided by measuring, from
//! the bottom up, so a short pane loses the day chart first and the token list never. ratatui
//! would otherwise squeeze all three and show a border around nothing.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `keys.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::model::{CYAN, YELLOW};
use crate::ui::app::{App, Panel};
use crate::ui::panels::limits::{figure_style, truncate};
use crate::ui::panels::timeseries::tokens_sparkline;
use crate::ui::theme::{bar_of, meter, panel, MUTED};
use crate::utils::format_count;

/// Rows of the day chart itself, between its borders and above its caption.
const CHART_ROWS: u16 = 3;

pub fn draw_breakdown(frame: &mut Frame, area: Rect, app: &App) {
    let inner_width = usize::from(area.width.saturating_sub(2));
    let mut flow = flow_lines(app, inner_width);
    // At the shortest pane the list is one row too tall once the counterfactual is showing. The
    // blank line goes before anything that says something does.
    if flow.len() as u16 + 2 > area.height {
        flow.retain(|line| line.width() > 0);
    }
    // A section the right-hand pane is already showing in full is left out: the same windows
    // twice on one screen is not more information, and it reads as two sources.
    let limits = if app.panel == Panel::Limits {
        Vec::new()
    } else {
        limit_lines(app, inner_width)
    };
    let has_days = app.panel != Panel::TimeSeries && app.daily().iter().any(|day| day.tokens > 0);

    // Each section with the height it needs, borders included. The token list is first and is
    // always drawn; the others are dropped from the end until what is left fits.
    let mut sections = vec![(Section::Flow, flow.len() as u16 + 2)];
    if !limits.is_empty() {
        sections.push((Section::Limits, limits.len() as u16 + 2));
    }
    if has_days {
        sections.push((Section::Days, CHART_ROWS + 1 + 2));
    }
    while sections.len() > 1 && sections.iter().map(|(_, h)| h).sum::<u16>() > area.height {
        sections.pop();
    }

    // Whatever is left over goes to the last section, so the rail ends where the pane does.
    let last = sections.len() - 1;
    let constraints: Vec<Constraint> = sections
        .iter()
        .enumerate()
        .map(|(index, (_, height))| {
            if index == last {
                Constraint::Min(0)
            } else {
                Constraint::Length(*height)
            }
        })
        .collect();
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut flow = Some(flow);
    let mut limits = Some(limits);
    for ((section, _), area) in sections.iter().zip(areas.iter()) {
        match section {
            Section::Flow => frame.render_widget(
                Paragraph::new(flow.take().unwrap_or_default()).block(panel("TOKEN FLOW", CYAN)),
                *area,
            ),
            Section::Limits => frame.render_widget(
                Paragraph::new(limits.take().unwrap_or_default()).block(panel("LIMITS", CYAN)),
                *area,
            ),
            Section::Days => draw_days(frame, *area, app),
        }
    }
}

#[derive(Clone, Copy)]
enum Section {
    Flow,
    Limits,
    Days,
}

/// The token kinds, each with a bar scaled to the largest of them, then what they cost.
fn flow_lines<'a>(app: &App, width: usize) -> Vec<Line<'a>> {
    let t = app.totals();
    let kinds = [
        ("INPUT", t.input),
        ("OUTPUT", t.output),
        ("REASONING", t.reasoning),
        ("CACHE READ", t.cache_read),
        ("CACHE WRITE", t.cache_write),
    ];
    let peak = kinds.iter().map(|(_, n)| *n).max().unwrap_or(0);
    // Label 12, figure 7, a space either side of the bar. Under four cells a bar says nothing.
    let bar_width = width.saturating_sub(12 + 7 + 2);
    let mut lines: Vec<Line> = kinds
        .iter()
        .map(|(label, count)| {
            let mut spans = vec![
                Span::styled(format!("{label:<12}"), Style::default().fg(MUTED)),
                Span::styled(
                    format!("{:>7}", format_count(*count)),
                    Style::default().fg(Color::White),
                ),
            ];
            if bar_width >= 4 {
                spans.push(Span::styled(
                    format!(" {}", bar_of(*count as f64, peak as f64, bar_width)),
                    Style::default().fg(CYAN),
                ));
            }
            Line::from(spans)
        })
        .collect();

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("EST. PAID COST ", Style::default().fg(YELLOW)),
        // `$0.0000` next to thousands of subscription requests reads as "free". The work
        // cost money; it was billed against a plan. Say that instead of printing a zero.
        Span::styled(
            if t.cost == 0.0 && t.quota_requests > 0 && t.unknown_requests == 0 {
                "on quota".to_string()
            } else {
                format!("${:.4}", t.cost)
            },
            Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
        ),
    ]));
    if t.api_equivalent > 0.0 {
        // Two lines, because one was wider than the rail and a `Paragraph` truncates in silence:
        // what went missing was "not billed", the half that stops the figure reading as a charge.
        lines.push(Line::from(Span::styled(
            format!("API-RATE EQUIV. ≈ ${:.4}", t.api_equivalent),
            Style::default().fg(MUTED),
        )));
        lines.push(Line::from(Span::styled(
            "  (on quota, not billed)",
            Style::default().fg(MUTED),
        )));
    }
    lines.push(Line::from(vec![
        Span::styled("PRICING ", Style::default().fg(MUTED)),
        // "complete" must not absorb quota-billed work: it is accounted for, but it
        // contributes no dollars to the total shown above.
        Span::raw(match (t.unknown_requests, t.quota_requests) {
            (0, 0) => "complete".to_string(),
            (0, quota) => format!("complete · {} on quota", format_count(quota)),
            _ => "partial / unknown".to_string(),
        }),
    ]));
    lines
}

/// One meter per subscription window. Empty when there is nothing to show, which leaves the
/// section out: the limits panel is where "none found, and here is why" is explained.
fn limit_lines<'a>(app: &App, width: usize) -> Vec<Line<'a>> {
    let report = app.limits();
    if !app.roots.limits_enabled {
        return Vec::new();
    }
    const FIGURE: usize = 5;
    // The names get the room they need, up to what leaves the meter eight cells to say anything in.
    let longest = report
        .snapshots
        .iter()
        .flat_map(|s| {
            s.windows
                .iter()
                .map(move |w| window_name(s, w).chars().count())
        })
        .max()
        .unwrap_or(0);
    let name_width = longest.min(width.saturating_sub(FIGURE + 1 + 8));
    let meter_width = width.saturating_sub(name_width + 1 + FIGURE);
    let muted = Style::default().fg(MUTED);
    let mut lines = Vec::new();
    for snapshot in &report.snapshots {
        for window in &snapshot.windows {
            let figure = figure_style(snapshot, window);
            let name = window_name(snapshot, window);
            let mut spans = vec![Span::styled(
                format!("{:<name_width$} ", truncate(&name, name_width)),
                muted,
            )];
            // An unstyled figure is plain text; its meter still wants a colour of its own.
            let fill = if figure == Style::default() {
                Style::default().fg(CYAN)
            } else {
                figure
            };
            spans.extend(meter(window.fraction, meter_width, fill));
            spans.push(Span::styled(
                format!("{:>4}%", window.percent_used().round() as u64),
                figure,
            ));
            lines.push(Line::from(spans));
        }
        if snapshot.windows.is_empty() && !snapshot.status_text.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", snapshot.agent), muted),
                Span::styled(snapshot.status_text.clone(), Style::default().fg(YELLOW)),
            ]));
        }
    }
    // A record that would not parse has no meter to draw, and must not vanish for that.
    if !report.problems.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("{} unreadable · see limits", report.problems.len()),
            Style::default().fg(YELLOW),
        )));
    }
    lines
}

fn window_name(
    snapshot: &crate::omarchy::LimitsSnapshot,
    window: &crate::omarchy::LimitWindow,
) -> String {
    format!("{} {}", snapshot.agent, window.label.to_lowercase())
}

/// Tokens per day across the range, newest at the right, with the busiest day named under it.
///
/// Tokens rather than dollars: on a subscription every day's dollars are unknown, and a chart of
/// them would be a flat line that reads as "nothing happened". Tokens are always measured.
fn draw_days(frame: &mut Frame, area: Rect, app: &App) {
    let days = app.daily();
    let block = panel("TOKENS PER DAY", CYAN);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(CHART_ROWS), Constraint::Length(1)])
        .split(inner);
    frame.render_widget(tokens_sparkline(days), rows[0]);
    if let Some(peak) = days.iter().max_by_key(|day| day.tokens) {
        let shown = days.len().min(usize::from(inner.width));
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(
                    "{shown}d · peak {} on {}",
                    format_count(peak.tokens),
                    peak.day
                ),
                Style::default().fg(MUTED),
            ))),
            rows[1],
        );
    }
}
