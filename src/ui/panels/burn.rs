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
use crate::model::{BurnRate, CLOUD, CYAN, RED, YELLOW};
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
    let value = if burn.is_quota_only() {
        // A rate of `$0.00/hr` would read as "this is costing you nothing", which is false.
        Span::styled(
            format!("on quota  ({} requests)", burn.quota_requests),
            Style::default().fg(CLOUD),
        )
    } else if burn.is_partial() {
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
        // Say which of the two conditions failed. This read "too little activity to project
        // (550/5 requests)" over a window of 550 quota-billed requests: the count was fine, the
        // rate was zero because nothing in the window is priced per token, and the message
        // blamed the one thing that was not wrong.
        let (text, colour) = if burn.requests < BurnRate::MIN_SAMPLE {
            (
                format!(
                    "too little activity to project ({}/{} requests)",
                    burn.requests,
                    BurnRate::MIN_SAMPLE
                ),
                MUTED,
            )
        } else if burn.is_quota_only() {
            ("on quota".to_string(), CLOUD)
        } else if burn.is_partial() {
            // A rate whose priced part is zero is not a floor to project from; it is unknown.
            ("unpriced".to_string(), YELLOW)
        } else {
            ("no per-token spend in the window".to_string(), MUTED)
        };
        return vec![Row::new(vec![
            Cell::from(Span::styled("projection", Style::default().fg(MUTED))),
            Cell::from(Span::styled(text, Style::default().fg(colour))),
        ])];
    }

    alerts
        .iter()
        .map(|alert| {
            let remaining = alert.limit - alert.spend;
            // A floor on the spend is a ceiling on what is left; a floor on the rate is a
            // ceiling on how long that lasts. The two markers say which: `≤` on the figure means
            // the budget's spend is a floor, which the budgets panel marks the same way; `≤` on
            // the duration alone means the rate is, which the spend row above marks. Before
            // this the panel rendered the rate as `≥ $4.10/hr (37 unpriced)` and then projected
            // from a spend that had dropped those same 37 requests.
            let remaining_bound = if alert.is_partial() { "≤ " } else { "" };
            let time_bound = if alert.is_partial() || burn.is_partial() {
                "≤ "
            } else {
                ""
            };
            let (text, colour) = match seconds_to_exhaust(burn, remaining) {
                Some(secs) => (
                    format!(
                        "{time_bound}{} left  ({remaining_bound}{} remaining)",
                        format_duration(secs),
                        money(remaining)
                    ),
                    if secs < 3600 { RED } else { YELLOW },
                ),
                None if remaining <= 0.0 => ("already over".to_string(), RED),
                None => ("not projectable".to_string(), MUTED),
            };
            let label = format!("{} {}", alert.scope.label(), alert.period.label());
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
