//! Subscription rate-limit windows, from every source that reports them.
//!
//! The rows come from `crate::limits::load`, which merges Omarchy's agents panel with the
//! utilisation Claude Code caches in its own config. This module only draws them.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::model::{CYAN, RED, YELLOW};
use crate::omarchy::{LimitWindow, LimitsSnapshot};
use crate::ui::aggregate::format_duration;
use crate::ui::app::App;
use crate::ui::panels::timeseries::bar;
use crate::ui::theme::{panel, MUTED};

pub fn draw_limits(frame: &mut Frame, area: Rect, app: &App) {
    let report = app.limits();
    let muted = Style::default().fg(MUTED);
    let mut lines: Vec<Line> = Vec::new();

    if !app.roots.limits_enabled {
        lines.push(Line::from(Span::styled(
            "Limits disabled in config ([omarchy] limits = false).",
            muted,
        )));
    } else if report.snapshots.is_empty() {
        // Gated on having no rows, not on Omarchy's directory being absent. It used to be the
        // latter, which meant that once a second source existed its windows were unreachable on
        // exactly the machines it was added for: every non-Omarchy machine short-circuited here
        // before the rows were ever considered.
        if report.present {
            lines.push(Line::from(Span::styled(
                "Records found, but none carry rate-limit windows.",
                muted,
            )));
        } else {
            lines.push(Line::from(Span::styled(
                format!("No rate-limit windows to show at {}.", report.dir.display()),
                muted,
            )));
            lines.push(Line::from(Span::styled(
                "Omarchy's Agents panel writes them, and Claude Code caches its own in \
                 ~/.claude.json once it has run.",
                muted,
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            format!(
                "{:<9} {:<28} {:<12} {:>5}  {:<11} {}",
                "AGENT", "WINDOW", "", "USED", "RESETS IN", "TIER"
            ),
            Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
        )));
        for snapshot in &report.snapshots {
            for window in &snapshot.windows {
                lines.push(window_line(snapshot, window));
            }
            if snapshot.windows.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(format!("{:<9} ", snapshot.agent), muted),
                    Span::styled(snapshot.status_text.clone(), Style::default().fg(YELLOW)),
                ]));
            }
        }
        lines.push(Line::from(""));
        for snapshot in &report.snapshots {
            lines.push(Line::from(Span::styled(snapshot_footer(snapshot), muted)));
        }
    }

    for problem in &report.problems {
        lines.push(Line::from(Span::styled(
            format!("unreadable: {problem}"),
            Style::default().fg(YELLOW),
        )));
    }

    frame.render_widget(Paragraph::new(lines).block(panel("LIMITS", CYAN)), area);
}

fn window_line<'a>(snapshot: &LimitsSnapshot, window: &LimitWindow) -> Line<'a> {
    let muted = Style::default().fg(MUTED);
    let figure = if snapshot.stale {
        muted
    } else if window.is_alarming() {
        Style::default().fg(RED).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let resets = match window.resets_in_secs {
        Some(secs) if secs <= 0 => "reset passed".to_string(),
        Some(secs) => format_duration(secs),
        None => "—".to_string(),
    };
    Line::from(vec![
        Span::styled(format!("{:<9} ", snapshot.agent), muted),
        Span::styled(format!("{:<28} ", truncate(&window.label, 28)), figure),
        Span::styled(format!("{:<12} ", bar(window.fraction, 1.0)), figure),
        Span::styled(
            format!("{:>4}%  ", window.percent_used().round() as u64),
            figure,
        ),
        Span::styled(format!("{:<11} ", resets), muted),
        Span::styled(snapshot.tier.clone(), muted),
    ])
}

fn snapshot_footer(snapshot: &LimitsSnapshot) -> String {
    let mut parts = vec![snapshot.name.clone()];
    if !snapshot.tier.is_empty() {
        parts.push(snapshot.tier.clone());
    }
    if !snapshot.status_text.is_empty() {
        parts.push(snapshot.status_text.clone());
    }
    match snapshot.age_secs {
        Some(age) if snapshot.stale => {
            parts.push(format!("stale, updated {} ago", format_duration(age)))
        }
        Some(age) => parts.push(format!("updated {} ago", format_duration(age))),
        None => parts.push("undated".to_string()),
    }
    parts.join(" · ")
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        let head: String = text.chars().take(width.saturating_sub(1)).collect();
        format!("{head}…")
    }
}
