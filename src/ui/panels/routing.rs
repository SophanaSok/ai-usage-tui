//! Routing analytics: whether the expensive model is earning its cost.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
    Frame,
};

use crate::escalation::{Escalations, Transition};
use crate::model::{RoutingAggregates, CYAN, GREEN, YELLOW};
use crate::routing::{cost_per_success, defect_rate, escalation_rate, retry_rate, success_rate};
use crate::ui::app::App;
use crate::ui::theme::{panel, MUTED};
use crate::utils::format_count;

pub fn draw_routing(frame: &mut Frame, area: Rect, app: &App) {
    // Two blocks, kept visibly apart. The top is derived from usage this tool already
    // collected; the bottom is what a harness recorded. Merging them would produce one table
    // where a measured pass rate and an inferred transition look identical, which is the
    // failure `CostStatus` exists to prevent, one level up.
    let escalations = app.escalations();
    let area = if escalations.is_empty() {
        area
    } else {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(escalation_height(escalations, area.height)),
                Constraint::Min(3),
            ])
            .split(area);
        frame.render_widget(escalation_block(escalations), split[0]);
        split[1]
    };

    let aggregates = app.routing();
    if aggregates.is_empty() {
        frame.render_widget(empty_state(), area);
        return;
    }

    // Cheapest per delivered result first — that ordering *is* the answer. Agents with nothing
    // passing sort last rather than appearing free.
    let mut ranked: Vec<&RoutingAggregates> = aggregates.iter().collect();
    ranked.sort_by(|a, b| {
        cost_per_success(a)
            .unwrap_or(f64::INFINITY)
            .partial_cmp(&cost_per_success(b).unwrap_or(f64::INFINITY))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let rows = ranked.iter().map(|agg| {
        Row::new(vec![
            Cell::from(agg.agent.clone()),
            Cell::from(short_model(&agg.model)),
            cost_per_success_cell(agg),
            success_cell(agg),
            Cell::from(format!("{:.0}%", retry_rate(agg))),
            Cell::from(format!("{:.0}%", escalation_rate(agg))),
            Cell::from(format!("{:.0}%", defect_rate(agg))),
            Cell::from(format_count(agg.tokens)),
            Cell::from(agg.tasks.to_string()),
        ])
    });

    let header = Row::new(vec![
        "AGENT",
        "MODEL",
        "$/SUCCESS",
        "PASS",
        "RETRY",
        "ESC",
        "DEFECT",
        "TOKENS",
        "TASKS",
    ])
    .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));

    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Min(12),
                Constraint::Length(18),
                Constraint::Length(10),
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Length(5),
                Constraint::Length(7),
                Constraint::Length(8),
                Constraint::Length(6),
            ],
        )
        .header(header)
        .column_spacing(1)
        .block(panel("ROUTING — cost per delivered result", CYAN)),
        area,
    );
}

/// Dollars per task that actually passed.
///
/// The one number that makes "is the expensive model worth it?" answerable rather than a matter
/// of taste. Two cases are deliberately not rendered as a figure:
///
/// A free model divides to `$0.0000` regardless of how badly it performs, so the metric cannot
/// discriminate between free models at all. Printing four decimal places implies a comparison
/// that is not being made — it says `free`, and the reader looks at the quality columns instead.
///
/// An agent with nothing passing shows `—`, never `$0.00`.
fn cost_per_success_cell<'a>(agg: &RoutingAggregates) -> Cell<'a> {
    match cost_per_success(agg) {
        Some(cost) if cost <= 0.0 => Cell::from(Span::styled("free", Style::default().fg(GREEN))),
        Some(cost) => Cell::from(Span::styled(
            format!("${cost:.4}"),
            Style::default().fg(GREEN),
        )),
        None => Cell::from(Span::styled("—", Style::default().fg(MUTED))),
    }
}

/// Test pass rate, or `—` when the agent never reported one.
///
/// An uninstrumented agent must not read as one that fails everything.
fn success_cell<'a>(agg: &RoutingAggregates) -> Cell<'a> {
    match success_rate(agg) {
        Some(rate) => {
            let colour = if rate >= 80.0 { GREEN } else { YELLOW };
            Cell::from(Span::styled(
                format!("{rate:.0}%"),
                Style::default().fg(colour),
            ))
        }
        None => Cell::from(Span::styled("—", Style::default().fg(MUTED))),
    }
}

fn short_model(model: &str) -> String {
    model.rsplit('/').next().unwrap_or(model).to_string()
}

/// What the panel would show, and how to make it show anything.
///
/// This is the state almost every user sees, because routing events come from the user's own
/// harness rather than being collected automatically. A bare "no events recorded" told them the
/// feature was empty without telling them it existed or why they would want it — which is why
/// the most differentiated thing this project does was also its least visible.
fn empty_state<'a>() -> Paragraph<'a> {
    let dim = Style::default().fg(MUTED);
    Paragraph::new(vec![
        Line::from(Span::styled(
            "Is the expensive model actually earning its cost?",
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "This panel ranks agents and models by dollars per task that passed its",
            dim,
        )),
        Line::from(Span::styled(
            "tests — alongside retry, escalation and review-defect rates. A model at",
            dim,
        )),
        Line::from(Span::styled(
            "twice the token price that gets it right first time can be cheaper per",
            dim,
        )),
        Line::from(Span::styled(
            "delivered result than a cheap one that needs three attempts.",
            dim,
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Nothing here is collected automatically — routing events come from your",
            dim,
        )),
        Line::from(Span::styled("own harness:", dim)),
        Line::from(""),
        Line::from(Span::styled(
            "  echo '{\"task\":\"fix-auth\",\"agent\":\"reviewer\",\"model\":\"claude-opus-5\",",
            Style::default().fg(YELLOW),
        )),
        Line::from(Span::styled(
            "         \"provider\":\"anthropic\",\"tokens\":12000,\"cost\":0.06,",
            Style::default().fg(YELLOW),
        )),
        Line::from(Span::styled(
            "         \"retries\":0,\"test_result\":true}' \\",
            Style::default().fg(YELLOW),
        )),
        Line::from(Span::styled(
            "    | ai-usage-tui --record-routing",
            Style::default().fg(YELLOW),
        )),
        Line::from(""),
        Line::from(Span::styled("  docs/routing-analytics.md", dim)),
    ])
    .block(panel("ROUTING — cost per delivered result", CYAN))
}

/// The derived block's height: one line per shown transition, plus the summary, plus borders.
///
/// Capped at a third of the pane. The recorded table is the panel's headline and must not be
/// squeezed off-screen by a long tail of one-off transitions.
fn escalation_height(escalations: &Escalations, available: u16) -> u16 {
    let shown = shown_transitions(escalations).len() as u16;
    let note = u16::from(escalations.unclassified_changes > 0);
    let wanted = shown + note + 3;
    wanted.min((available / 3).max(4))
}

/// The transitions worth a line, most frequent first.
fn shown_transitions(escalations: &Escalations) -> &[Transition] {
    let end = escalations.transitions.len().min(3);
    &escalations.transitions[..end]
}

fn escalation_block<'a>(escalations: &Escalations) -> Paragraph<'a> {
    let dim = Style::default().fg(MUTED);
    let mut lines = vec![Line::from(vec![
        Span::styled(
            match escalations.rate() {
                Some(rate) => format!("{:.0}%", rate),
                None => "—".to_string(),
            },
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " of {} sessions used a pricier model than they opened with",
                escalations.sessions_examined
            ),
            dim,
        ),
    ])];

    for transition in shown_transitions(escalations) {
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "  {} → {}",
                    short_model(&transition.from),
                    short_model(&transition.to)
                ),
                Style::default().fg(YELLOW),
            ),
            Span::styled(
                match transition.sessions {
                    1 => "  1 session".to_string(),
                    n => format!("  {n} sessions"),
                },
                dim,
            ),
            Span::styled(
                // A floor, not a total, when part of the spend that followed has no price.
                match transition.unpriced_after {
                    0 => format!("  ${:.2} after", transition.cost_after),
                    _ => format!("  ≥ ${:.2} after", transition.cost_after),
                },
                Style::default().fg(GREEN),
            ),
        ]));
    }

    if escalations.unclassified_changes > 0 {
        // Reported rather than dropped: a low escalation count and a count taken with one eye
        // shut look the same on screen otherwise.
        lines.push(Line::from(Span::styled(
            format!(
                "  {} model changes unranked (no price on one side)",
                escalations.unclassified_changes
            ),
            dim,
        )));
    }

    Paragraph::new(lines).block(panel("ESCALATIONS — derived from sessions", CYAN))
}
