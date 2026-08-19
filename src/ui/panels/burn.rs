//! Burn rate over a trailing window, and what it implies for your budgets.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Rect},
    style::Style,
    text::Span,
    widgets::{Cell, Paragraph, Row, Table},
    Frame,
};

use crate::budget::Alert;
use crate::model::{BurnRate, CYAN, RED, YELLOW};
use crate::ui::aggregate::{format_duration, seconds_to_exhaust};
use crate::ui::app::App;
use crate::ui::theme::{panel, MUTED};
use crate::utils::format_count;

pub fn draw_burn(frame: &mut Frame, area: Rect, app: &App) {
    let burn = app.burn();
    let title = format!("BURN RATE  (last {})", window_label(burn.window_secs));
    let block = panel(&title, CYAN);

    if burn.requests == 0 {
        frame.render_widget(
            Paragraph::new("No usage in the trailing window.")
                .style(Style::default().fg(MUTED))
                .block(block),
            area,
        );
        return;
    }

    let mut rows = vec![
        rate_row("tokens/min", format_count(burn.tokens_per_minute() as u64)),
        rate_row("requests", burn.requests.to_string()),
        spend_row(burn),
    ];

    rows.push(Row::new(vec![Cell::from(""), Cell::from("")]));
    rows.extend(projection_rows(burn, app.alerts()));

    frame.render_widget(
        // Wide enough for `model:<a long model id> monthly`, which is the longest label the
        // budget scopes produce. Truncating it to `mo` made the period unreadable.
        Table::new(rows, [Constraint::Length(28), Constraint::Min(18)])
            .column_spacing(1)
            .block(block),
        area,
    );
}

fn rate_row<'a>(label: &'a str, value: String) -> Row<'a> {
    Row::new(vec![
        Cell::from(Span::styled(label, Style::default().fg(MUTED))),
        Cell::from(value),
    ])
}

/// Spend per hour, marked as a floor when part of the window is unpriced.
fn spend_row<'a>(burn: &BurnRate) -> Row<'a> {
    let value = if burn.is_partial() {
        Span::styled(
            format!(
                "≥ ${:.2}/hr  ({} unpriced)",
                burn.cost_per_hour(),
                burn.unpriced_requests
            ),
            Style::default().fg(YELLOW),
        )
    } else {
        Span::raw(format!("${:.2}/hr", burn.cost_per_hour()))
    };
    Row::new(vec![
        Cell::from(Span::styled("spend", Style::default().fg(MUTED))),
        Cell::from(value),
    ])
}

/// Time until each budget is exhausted at the current rate.
///
/// This is the reason the panel exists. A rate on its own is trivia; a rate measured against a
/// limit you set is an answer — and it is only possible because the budget engine and the
/// collectors are in the same process.
fn projection_rows<'a>(burn: &BurnRate, alerts: &[Alert]) -> Vec<Row<'a>> {
    if alerts.is_empty() {
        return vec![Row::new(vec![
            Cell::from(Span::styled("projection", Style::default().fg(MUTED))),
            Cell::from(Span::styled(
                "no budgets configured — see [budgets] in config.toml",
                Style::default().fg(MUTED),
            )),
        ])];
    }

    if !burn.is_projectable() {
        return vec![Row::new(vec![
            Cell::from(Span::styled("projection", Style::default().fg(MUTED))),
            Cell::from(Span::styled(
                format!(
                    "too little activity to project ({}/{} requests)",
                    burn.requests,
                    BurnRate::MIN_SAMPLE
                ),
                Style::default().fg(MUTED),
            )),
        ])];
    }

    alerts
        .iter()
        .map(|alert| {
            let remaining = alert.limit - alert.spend;
            let (text, colour) = match seconds_to_exhaust(burn, remaining) {
                Some(secs) => (
                    format!(
                        "{} left  ({} remaining)",
                        format_duration(secs),
                        money(remaining)
                    ),
                    if secs < 3600 { RED } else { YELLOW },
                ),
                None if remaining <= 0.0 => ("already over".to_string(), RED),
                None => ("not projectable".to_string(), MUTED),
            };
            let label = format!("{} {}", alert.scope.label(), period(alert));
            Row::new(vec![
                Cell::from(Span::styled(label, Style::default().fg(MUTED))),
                Cell::from(Span::styled(text, Style::default().fg(colour))),
            ])
        })
        .collect()
}

/// `1h`, `30m` — a whole-hour window reads better without a `00m` on the end.
fn window_label(seconds: i64) -> String {
    if seconds > 0 && seconds % 3600 == 0 {
        format!("{}h", seconds / 3600)
    } else {
        format_duration(seconds)
    }
}

fn money(value: f64) -> String {
    format!("${value:.2}")
}

fn period(alert: &Alert) -> &'static str {
    match alert.period {
        crate::budget::BudgetPeriod::Daily => "daily",
        crate::budget::BudgetPeriod::Monthly => "monthly",
    }
}
