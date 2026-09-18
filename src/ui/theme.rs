//! Colours and small shared widgets.
//!
//! Everything visual that more than one panel needs lives here, so a new panel does not have
//! to rediscover the palette or re-derive how a bordered box is built.

use ratatui::{
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
    Frame,
};

use crate::model::{CostStatus, Usage};

pub const MUTED: Color = Color::Rgb(125, 145, 160);
pub const PANEL: Color = Color::Rgb(18, 28, 37);
/// The selected row's background. Named because `NO_COLOR` has to recognise it: stripped of
/// colour, a selection drawn only as a background is no selection at all.
pub const SELECTED: Color = Color::Rgb(37, 57, 67);
/// Panel borders. Was an inline literal in `panel`, which is how two copies of the header's
/// background came to exist.
pub const BORDER: Color = Color::Rgb(48, 72, 84);
/// The strip behind the header, the tab row and the alert banner.
pub const HEADER_BG: Color = Color::Rgb(10, 18, 24);

pub fn panel<'a>(title: &'a str, color: Color) -> Block<'a> {
    Block::default()
        .title(Span::styled(
            format!(" {} ", title),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(PANEL))
}

/// `panel`, with a second title at the right of the top border: how many rows the pane holds.
///
/// A table that scrolls shows a reader some of its rows and, until now, nothing saying how many
/// there were.
pub fn panel_with_count<'a>(title: &'a str, color: Color, count: String) -> Block<'a> {
    panel(title, color).title(
        Line::from(Span::styled(
            format!(" {count} "),
            Style::default().fg(MUTED),
        ))
        .right_aligned(),
    )
}

/// A bar `width` cells wide scaled to `peak`, in eighth-block increments.
///
/// Sub-cell resolution matters: without it every value below one cell's worth of the peak
/// renders empty, and a chart of mostly-small values looks like no activity at all. Anything
/// above zero draws at least the thinnest glyph; zero, and a peak of zero, draw nothing.
pub fn bar_of(value: f64, peak: f64, width: usize) -> String {
    const EIGHTHS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

    if peak <= 0.0 || value <= 0.0 || width == 0 {
        return String::new();
    }
    let eighths = ((value / peak) * (width * 8) as f64).round().max(1.0) as usize;
    let full = eighths / 8;
    let remainder = eighths % 8;

    let mut out = "█".repeat(full.min(width));
    if full < width && remainder > 0 {
        out.push(EIGHTHS[remainder - 1]);
    }
    out
}

/// A meter: `bar_of` in `style`, then a dim track out to `width`, so a reader sees how far there
/// is to go as well as how far it has gone.
pub fn meter<'a>(fraction: f64, width: usize, style: Style) -> Vec<Span<'a>> {
    let filled = bar_of(fraction.clamp(0.0, 1.0), 1.0, width);
    let track = width.saturating_sub(filled.chars().count());
    vec![
        Span::styled(filled, style),
        Span::styled("░".repeat(track), Style::default().fg(BORDER)),
    ]
}

/// The pane width from which a table's TOKENS column carries a bar. Narrower than this the
/// columns that hold figures need the room more than a picture of them does.
pub const BAR_FROM_WIDTH: u16 = 84;
/// Cells a TOKENS bar takes when it is drawn.
pub const TOKEN_BAR: usize = 8;

/// The width of a TOKENS column: the figure, and the bar when `area` has room for one.
pub fn tokens_column(area: Rect) -> u16 {
    if area.width >= BAR_FROM_WIDTH {
        (8 + TOKEN_BAR) as u16
    } else {
        9
    }
}

/// A TOKENS cell: the count, then a bar scaled to the largest row showing, so a reader sees which
/// rows carry the total without comparing `2.5M` with `688.6K` by eye.
pub fn tokens_cell<'a>(tokens: u64, peak: u64, color: Color, area: Rect) -> Cell<'a> {
    let count = crate::utils::format_count(tokens);
    if area.width < BAR_FROM_WIDTH {
        return Cell::from(count);
    }
    Cell::from(Line::from(vec![
        Span::raw(format!("{count:<8}")),
        Span::styled(
            bar_of(tokens as f64, peak as f64, TOKEN_BAR),
            Style::default().fg(color),
        ),
    ]))
}

/// How many rows a pane holds, for its title: `9 models`, or `4 of 9 models` under a filter.
/// `noun` is the plural; every one used here forms its singular by dropping the `s`.
pub fn row_count(shown: usize, search: Option<(&str, usize, usize)>, noun: &str) -> String {
    let named = |count: usize| match count {
        1 => noun.strip_suffix('s').unwrap_or(noun),
        _ => noun,
    };
    match search {
        Some((_, shown, total)) => format!("{shown} of {total} {}", named(total)),
        None => format!("{shown} {}", named(shown)),
    }
}

/// A scrollbar on a table's right border, drawn only when the rows do not all fit -- a thumb on
/// a list with nothing to scroll says there is more where there is not.
pub fn draw_scrollbar(frame: &mut Frame, area: Rect, rows: usize, selected: usize) {
    // Borders top and bottom, and the header row.
    let visible = usize::from(area.height.saturating_sub(3));
    if rows <= visible || visible == 0 {
        return;
    }
    let mut state = ScrollbarState::new(rows).position(selected);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("┃")
            .track_style(Style::default().fg(BORDER))
            .thumb_style(Style::default().fg(MUTED)),
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

pub fn metric<'a>(label: &'a str, value: String, color: Color, subtitle: String) -> Paragraph<'a> {
    Paragraph::new(vec![
        Line::from(Span::styled(
            value,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(subtitle, Style::default().fg(MUTED))),
    ])
    .block(panel(label, color))
    .style(Style::default().fg(Color::White))
}

pub fn cost_display(usage: &Usage) -> String {
    match usage.cost_status {
        CostStatus::Local => "LOCAL".into(),
        CostStatus::Free => "FREE".into(),
        CostStatus::ProviderReported => usage
            .cost
            .map(|cost| format!("${:.4} reported", cost))
            .unwrap_or_else(|| "REPORTED / NO COST".into()),
        CostStatus::Calculated => usage
            .cost
            .map(|cost| format!("${:.4} calculated", cost))
            .unwrap_or_else(|| "CALCULATED / NO COST".into()),
        CostStatus::Estimated => usage
            .cost
            .map(|cost| format!("${:.4} estimated", cost))
            .unwrap_or_else(|| "ESTIMATED / NO COST".into()),
        // Not "$0.00" and not "FREE": this usage costs money, it is simply billed against a
        // plan rather than per token. Eight characters so it is not truncated by the COST
        // column's width.
        CostStatus::Quota => "ON QUOTA".into(),
        CostStatus::Unavailable => "UNKNOWN COST".into(),
    }
}

/// The figure `cost_display` puts in the cell, or `None` where it shows no figure at all.
///
/// Lives beside `cost_display` because the two have to agree. Sorting a COST column by a number
/// the cell never shows is how `ON QUOTA` ends up ranked as the cheapest work on the machine --
/// the cell saying the cost is unknown while the ordering says it is $0.00, about the same row.
///
/// `Free` and `Local` genuinely are zero and sort as zero. `Quota` and `Unavailable` are costs
/// this tool refuses to invent, and refusing to invent one is not the same as knowing it is zero.
/// Matched exhaustively on purpose: a new `CostStatus` should not be able to acquire a sort
/// position by falling through a wildcard.
pub fn cost_sort_key(usage: &Usage) -> Option<f64> {
    match usage.cost_status {
        CostStatus::Free | CostStatus::Local => Some(0.0),
        CostStatus::Quota | CostStatus::Unavailable => None,
        CostStatus::ProviderReported | CostStatus::Calculated | CostStatus::Estimated => usage.cost,
    }
}
