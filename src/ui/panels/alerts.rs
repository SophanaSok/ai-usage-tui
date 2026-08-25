//! One-line banner for actionable budget alerts.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::budget::{Alert, AlertLevel};
use crate::model::{RED, YELLOW};
use crate::ui::app::App;
use crate::ui::theme::MUTED;

pub fn draw_alert_banner(frame: &mut Frame, area: Rect, app: &App) {
    let actionable: Vec<&Alert> = app.alerts.iter().filter(|a| a.is_actionable()).collect();
    if actionable.is_empty() {
        return;
    }
    let spans: Vec<Span> = actionable
        .iter()
        .flat_map(|alert| {
            let color = match alert.level {
                AlertLevel::Warn => YELLOW,
                AlertLevel::Critical | AlertLevel::Exceeded => RED,
                AlertLevel::Ok => MUTED,
            };
            // A banner is the loudest place this figure appears, so it is the last place a
            // floor may pass as a total.
            let floor = if alert.is_partial() { "≥ " } else { "" };
            vec![
                Span::styled(
                    format!(" {} ", alert.level.label()),
                    Style::default()
                        .fg(Color::Black)
                        .bg(color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "{} {}/{floor}${:.2}/${:.2} ({floor}{}%)  ",
                        alert.scope.label(),
                        alert.period.label(),
                        alert.spend,
                        alert.limit,
                        alert.pct as u64
                    ),
                    Style::default().fg(color),
                ),
            ]
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(10, 18, 24))),
        area,
    );
}
