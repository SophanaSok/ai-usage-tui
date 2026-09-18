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
use crate::utils::ColourDepth;

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

/// A palette colour as a terminal with fewer colours can draw it. `TrueColour` is the identity.
///
/// Only `Rgb` needs mapping, with one exception: on sixteen colours `White` becomes the
/// terminal's default foreground. Backgrounds are handed back to the terminal there, and white
/// text on a light theme's background is no text at all.
pub fn downgrade(color: Color, depth: ColourDepth, background: bool) -> Color {
    match (depth, color) {
        (ColourDepth::TrueColour, _) => color,
        (ColourDepth::Indexed256, Color::Rgb(r, g, b)) => Color::Indexed(nearest_256(r, g, b)),
        (ColourDepth::Indexed256, _) => color,
        (ColourDepth::Ansi16, Color::White) if !background => Color::Reset,
        (ColourDepth::Ansi16, Color::Rgb(..)) => ansi16(color),
        (ColourDepth::Ansi16, _) => color,
    }
}

/// The named palette first, by meaning rather than by distance: nearest-match puts `MUTED` and
/// `BORDER` on the same grey, and the alarm red is the one colour here a test pins. Anything
/// unnamed falls through to the nearest of the sixteen.
fn ansi16(color: Color) -> Color {
    use crate::model::{CLOUD, CYAN, GREEN, RED, YELLOW};
    match color {
        // The chrome backgrounds are the terminal's own; the selection is the one background
        // that carries meaning, so it stays a colour.
        c if c == PANEL || c == HEADER_BG => Color::Reset,
        c if c == SELECTED => Color::DarkGray,
        c if c == BORDER => Color::DarkGray,
        c if c == MUTED => Color::Gray,
        c if c == CYAN => Color::LightCyan,
        c if c == GREEN => Color::LightGreen,
        c if c == YELLOW => Color::LightYellow,
        c if c == RED => Color::LightRed,
        c if c == CLOUD => Color::LightMagenta,
        Color::Rgb(r, g, b) => nearest_ansi(r, g, b),
        other => other,
    }
}

/// xterm's 256: a 6x6x6 cube at 16..=231 and a grey ramp at 232..=255. The nearer of the two.
fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let level = |v: u8| {
        (0..6usize)
            .min_by_key(|i| (i32::from(LEVELS[*i]) - i32::from(v)).abs())
            .unwrap_or(0)
    };
    let distance = |a: (u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a.0, r) + d(a.1, g) + d(a.2, b)
    };
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let mean = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    let step = ((mean.saturating_sub(8) + 5) / 10).min(23) as u8;
    let grey = 8 + 10 * step;
    if distance((grey, grey, grey)) < distance(cube) {
        232 + step
    } else {
        (16 + 36 * ri + 6 * gi + bi) as u8
    }
}

fn nearest_ansi(r: u8, g: u8, b: u8) -> Color {
    const TABLE: [(Color, (u8, u8, u8)); 16] = [
        (Color::Black, (0, 0, 0)),
        (Color::Red, (205, 0, 0)),
        (Color::Green, (0, 205, 0)),
        (Color::Yellow, (205, 205, 0)),
        (Color::Blue, (0, 0, 238)),
        (Color::Magenta, (205, 0, 205)),
        (Color::Cyan, (0, 205, 205)),
        (Color::Gray, (229, 229, 229)),
        (Color::DarkGray, (127, 127, 127)),
        (Color::LightRed, (255, 0, 0)),
        (Color::LightGreen, (0, 255, 0)),
        (Color::LightYellow, (255, 255, 0)),
        (Color::LightBlue, (92, 92, 255)),
        (Color::LightMagenta, (255, 0, 255)),
        (Color::LightCyan, (0, 255, 255)),
        (Color::White, (255, 255, 255)),
    ];
    let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
    TABLE
        .iter()
        .min_by_key(|(_, (tr, tg, tb))| d(*tr, r) + d(*tg, g) + d(*tb, b))
        .map(|(color, _)| *color)
        .unwrap_or(Color::Reset)
}

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
