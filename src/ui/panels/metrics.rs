//! The metric tiles across the top of the dashboard.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a key
//! binding in `mod.rs`, and a match arm in `draw`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

use crate::model::{Category, CYAN};
use crate::ui::app::App;
use crate::ui::theme::metric;
use crate::utils::format_count;

pub fn draw_metrics(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.totals();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(24),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(16),
        ])
        .split(area);
    let total = metric(
        "TOTAL TOKENS",
        format_count(t.tokens()),
        CYAN,
        format!("{} requests", t.requests),
    );
    frame.render_widget(total, cols[0]);
    for (i, (category, cat)) in app.category_totals().iter().enumerate() {
        let subtitle = if *category == Category::Paid {
            format!("${:.4}", cat.cost)
        } else {
            format!("{} tokens", format_count(cat.tokens()))
        };
        frame.render_widget(
            metric(
                category.label(),
                format_count(cat.tokens()),
                category.color(),
                subtitle,
            ),
            cols[i + 1],
        );
    }
}
