//! Subscription rate-limit windows, from every source that reports them.
//!
//! The rows come from `crate::limits::load`, which merges Omarchy's agents panel with what the
//! agents record themselves: the utilisation Claude Code caches in its own config and pushes to
//! its status line, and the windows Codex writes into its rollouts. This module only draws them.
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
                "Omarchy's Agents panel writes them, Claude Code caches its own in \
                 ~/.claude.json once it has run, and Codex writes its own on a ChatGPT plan.",
                muted,
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            format!(
                "{:<9} {:<28} {:<12} {:>5}  {:<11} {:<12} {}",
                "AGENT", "WINDOW", "", "USED", "RESETS IN", "AT", "TIER"
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
    let figure = figure_style(snapshot, window);
    let resets = match window.resets_in_secs {
        Some(secs) if secs <= 0 => "reset passed".to_string(),
        Some(secs) => format_duration(secs),
        None => "—".to_string(),
    };
    Line::from(vec![
        Span::styled(format!("{:<9} ", snapshot.agent), muted),
        Span::styled(format!("{:<28} ", truncate(&window.label, 28)), figure),
        // Plain figures are unstyled text; the bar beside them takes the accent the rail's
        // meters use, and follows `figure` when that says stale or alarming.
        Span::styled(
            format!("{:<12} ", bar(window.fraction, 1.0)),
            if figure == Style::default() {
                Style::default().fg(CYAN)
            } else {
                figure
            },
        ),
        Span::styled(
            format!("{:>4}%  ", window.percent_used().round() as u64),
            figure,
        ),
        Span::styled(format!("{:<11} ", resets), muted),
        Span::styled(
            format!(
                "{:<12} ",
                reset_at_label(window, &chrono::Local).unwrap_or_default()
            ),
            muted,
        ),
        Span::styled(snapshot.tier.clone(), muted),
    ])
}

/// When a window resets, on the wall clock: `Fri 14:00`, or `Oct 03 14:00` once the weekday
/// alone would be ambiguous.
///
/// A countdown answers "how long", and it is stale the moment it is read off a snapshot that is
/// twenty minutes old; "can I start this at three?" is asked of a clock. Nothing for a window
/// with no reset instant, or one whose reset has passed -- the countdown column already says so.
/// Generic over the zone so a test can name one; the panel passes `Local`.
fn reset_at_label<Tz: chrono::TimeZone>(window: &LimitWindow, zone: &Tz) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    let resets_at = window.resets_at?;
    let remaining = window.resets_in_secs.filter(|secs| *secs > 0)?;
    let at = zone.timestamp_opt(resets_at, 0).single()?;
    // Six days, not seven: a reset a week out lands on today's weekday.
    let format = if remaining < 6 * 86_400 {
        "%a %H:%M"
    } else {
        "%b %d %H:%M"
    };
    Some(at.format(format).to_string())
}

/// How a window's figures are drawn: dimmed when the snapshot is stale, red when the window is
/// alarming, plain otherwise. Stale wins -- a number describing some earlier moment must not
/// raise an alarm about this one.
///
/// Shared with the rail's meters in `breakdown`, so the two cannot disagree about the same window.
pub(crate) fn figure_style(snapshot: &LimitsSnapshot, window: &LimitWindow) -> Style {
    if snapshot.stale {
        Style::default().fg(MUTED)
    } else if window.is_alarming() {
        Style::default().fg(RED).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
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

pub(crate) fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        let head: String = text.chars().take(width.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(resets_at: Option<i64>, resets_in_secs: Option<i64>) -> LimitWindow {
        LimitWindow {
            label: "Session (5-hour)".into(),
            fraction: 0.5,
            resets_at,
            resets_in_secs,
        }
    }

    /// 2026-09-18 17:29:27 UTC, a Friday.
    const FRIDAY: i64 = 1_789_752_567;

    #[test]
    fn a_reset_is_named_on_the_wall_clock_of_the_zone_it_is_read_in() {
        let soon = window(Some(FRIDAY + 3 * 3600), Some(3 * 3600));
        assert_eq!(
            reset_at_label(&soon, &chrono::Utc).as_deref(),
            Some("Fri 20:29")
        );
        // The same instant, read five and a half hours east: already Saturday.
        let east = chrono::FixedOffset::east_opt(5 * 3600 + 1800).unwrap();
        assert_eq!(reset_at_label(&soon, &east).as_deref(), Some("Sat 01:59"));
    }

    #[test]
    fn a_reset_a_week_out_carries_its_date() {
        let weekly = window(Some(FRIDAY + 7 * 86_400), Some(7 * 86_400));
        assert_eq!(
            reset_at_label(&weekly, &chrono::Utc).as_deref(),
            Some("Sep 25 17:29"),
            "`Fri 17:29` would read as today"
        );
        let five_days = window(Some(FRIDAY + 5 * 86_400), Some(5 * 86_400));
        assert_eq!(
            reset_at_label(&five_days, &chrono::Utc).as_deref(),
            Some("Wed 17:29")
        );
    }

    #[test]
    fn no_instant_or_one_that_has_passed_gets_no_clock_time() {
        assert_eq!(reset_at_label(&window(None, None), &chrono::Utc), None);
        assert_eq!(
            reset_at_label(&window(Some(FRIDAY), Some(0)), &chrono::Utc),
            None
        );
        assert_eq!(
            reset_at_label(&window(Some(FRIDAY), Some(-60)), &chrono::Utc),
            None
        );
    }
}
