//! Render a frame to SVG, without a terminal or a compositor.
//!
//! The README screenshots used to be photographs of a real terminal window, taken with `scrot`
//! or `grim`. That approach has two defects that cannot be engineered away. It captures a screen
//! *region*, so anything drawn over the terminal — a notification, a floating window — lands in
//! the image; a project that promises to read no message content should not ship pictures of its
//! author's desktop. And it needs a graphical session, so nobody can regenerate the images in CI
//! and they go stale silently, which is exactly what happened to `routing.png`.
//!
//! This renders the same `draw` call through ratatui's own off-screen backend and turns the
//! resulting cell buffer into SVG: same code path as the real dashboard, no screen involved,
//! byte-identical output for identical input. `scripts/render-readme-screenshots.sh` rasterises
//! the result to PNG.

use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

use super::app::App;

/// Advance width of one cell, at `FONT_SIZE`. 0.6 em is the advance of every monospace face this
/// is likely to be rasterised with; `textLength` on each run pins the layout to it regardless.
const CELL_W: f64 = 9.6;
/// Line height. 1.25 em, matching a terminal's default line spacing.
const CELL_H: f64 = 20.0;
const FONT_SIZE: f64 = 16.0;
/// Where the glyph baseline sits within the cell box.
const BASELINE: f64 = 15.0;
/// Breathing room around the grid, so the border does not sit on the image edge.
const PAD: f64 = 12.0;

/// The terminal background. Cells that set no background of their own get this, which is what
/// `Color::Reset` means on a real terminal.
const BACKGROUND: &str = "#0a1014";
/// The default foreground, likewise for `Color::Reset`.
const FOREGROUND: &str = "#d8e2eb";

const FONT_STACK: &str =
    "'JetBrains Mono','JetBrainsMono Nerd Font','DejaVu Sans Mono','Liberation Mono',\
     'Menlo','Consolas',monospace";

/// Render the dashboard at `width` x `height` character cells.
///
/// Takes `&App` rather than a buffer so callers get the real frame layout — the panel split, the
/// footer's width-dependent form, the alert banner's conditional row — rather than a hand-built
/// approximation of it.
pub fn render_svg(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height))
        .expect("the off-screen backend cannot fail to initialise");
    terminal
        .draw(|frame| super::draw(frame, app))
        .expect("the off-screen backend cannot fail to draw");
    buffer_to_svg(terminal.backend().buffer())
}

/// Turn a rendered cell buffer into a standalone SVG document.
pub fn buffer_to_svg(buffer: &Buffer) -> String {
    let cols = buffer.area.width;
    let rows = buffer.area.height;
    let w = PAD * 2.0 + f64::from(cols) * CELL_W;
    let h = PAD * 2.0 + f64::from(rows) * CELL_H;

    let mut out = String::with_capacity(64 * 1024);
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" \
         viewBox=\"0 0 {w:.0} {h:.0}\" font-family=\"{FONT_STACK}\" font-size=\"{FONT_SIZE}\">\n"
    ));
    out.push_str(&format!(
        "<rect width=\"{w:.0}\" height=\"{h:.0}\" fill=\"{BACKGROUND}\" rx=\"6\"/>\n"
    ));

    for y in 0..rows {
        emit_backgrounds(&mut out, buffer, y, cols);
    }
    for y in 0..rows {
        emit_text(&mut out, buffer, y, cols);
    }

    out.push_str("</svg>\n");
    out
}

/// One `<rect>` per run of cells sharing a background, rather than one per cell: a 132x38 frame
/// is 5,016 cells and almost all of them are the default background.
fn emit_backgrounds(out: &mut String, buffer: &Buffer, y: u16, cols: u16) {
    let mut run_start = 0u16;
    let mut run_fill: Option<String> = None;

    let flush = |out: &mut String, fill: &Option<String>, start: u16, end: u16| {
        let Some(fill) = fill else { return };
        if fill == BACKGROUND {
            return;
        }
        let x = PAD + f64::from(start) * CELL_W;
        let top = PAD + f64::from(y) * CELL_H;
        let width = f64::from(end - start) * CELL_W;
        out.push_str(&format!(
            "<rect x=\"{x:.2}\" y=\"{top:.2}\" width=\"{width:.2}\" height=\"{CELL_H:.2}\" \
             fill=\"{fill}\"/>\n"
        ));
    };

    for x in 0..cols {
        let fill = background_of(cell_at(buffer, x, y));
        if run_fill.as_deref() != Some(fill.as_str()) {
            flush(out, &run_fill, run_start, x);
            run_start = x;
            run_fill = Some(fill);
        }
    }
    flush(out, &run_fill, run_start, cols);
}

/// One `<text>` per run of cells sharing a foreground and modifier set.
///
/// Each run carries `textLength`, which pins it to the cell grid even when the rasteriser
/// substitutes a font whose advance width is not exactly 0.6 em. Without it, a substituted font
/// walks the row out of alignment and the box-drawing borders come apart.
fn emit_text(out: &mut String, buffer: &Buffer, y: u16, cols: u16) {
    let baseline = PAD + f64::from(y) * CELL_H + BASELINE;
    let mut run_start = 0u16;
    let mut run_style: Option<(String, Modifier)> = None;
    let mut run = String::new();

    let flush = |out: &mut String, style: &Option<(String, Modifier)>, start: u16, text: &str| {
        let Some((fill, modifier)) = style else {
            return;
        };
        // Whitespace carries no glyph; the background pass already painted it. Runs are broken
        // by style, not by content, so a row of unstyled text is one run from column zero to the
        // right margin — trimming the blank ends is most of the file size of an empty panel.
        let leading = text.chars().take_while(|c| c.is_whitespace()).count();
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let start = start + leading as u16;
        let x = PAD + f64::from(start) * CELL_W;
        let length = text.chars().count() as f64 * CELL_W;
        let mut attrs = String::new();
        if modifier.contains(Modifier::BOLD) {
            attrs.push_str(" font-weight=\"bold\"");
        }
        if modifier.contains(Modifier::ITALIC) {
            attrs.push_str(" font-style=\"italic\"");
        }
        if modifier.contains(Modifier::UNDERLINED) {
            attrs.push_str(" text-decoration=\"underline\"");
        }
        if modifier.contains(Modifier::DIM) {
            attrs.push_str(" opacity=\"0.6\"");
        }
        out.push_str(&format!(
            "<text x=\"{x:.2}\" y=\"{baseline:.2}\" fill=\"{fill}\" textLength=\"{length:.2}\" \
             lengthAdjust=\"spacingAndGlyphs\" xml:space=\"preserve\"{attrs}>{}</text>\n",
            escape(text)
        ));
    };

    for x in 0..cols {
        let cell = cell_at(buffer, x, y);
        let style = (foreground_of(cell), cell.modifier);
        if run_style.as_ref() != Some(&style) {
            flush(out, &run_style, run_start, &run);
            run.clear();
            run_start = x;
            run_style = Some(style);
        }
        // A wide glyph's trailing half has an empty symbol; the leading half already advanced
        // two cells' worth of `textLength`, so skipping it keeps the run aligned.
        let symbol = cell.symbol();
        run.push_str(if symbol.is_empty() { " " } else { symbol });
    }
    flush(out, &run_style, run_start, &run);
}

fn cell_at(buffer: &Buffer, x: u16, y: u16) -> &Cell {
    buffer.cell((x, y)).unwrap_or(&Cell::EMPTY)
}

/// Reverse video is a style, not a colour, so it has to be resolved before either side is read.
fn background_of(cell: &Cell) -> String {
    if cell.modifier.contains(Modifier::REVERSED) {
        return color_to_hex(cell.fg, FOREGROUND);
    }
    color_to_hex(cell.bg, BACKGROUND)
}

fn foreground_of(cell: &Cell) -> String {
    if cell.modifier.contains(Modifier::REVERSED) {
        return color_to_hex(cell.bg, BACKGROUND);
    }
    color_to_hex(cell.fg, FOREGROUND)
}

/// `Reset` means "whatever the terminal's default is", so it resolves to `default` rather than to
/// a colour of its own — that is the difference between a readable frame and white-on-white.
fn color_to_hex(color: Color, default: &str) -> String {
    match color {
        Color::Reset => default.to_string(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => "#0a1014".to_string(),
        Color::Red => "#ff6969".to_string(),
        Color::Green => "#74eb98".to_string(),
        Color::Yellow => "#ffcd5c".to_string(),
        Color::Blue => "#45a0ff".to_string(),
        Color::Magenta => "#c289ff".to_string(),
        Color::Cyan => "#45d3ff".to_string(),
        Color::Gray => "#7d91a0".to_string(),
        Color::DarkGray => "#4a5a68".to_string(),
        Color::LightRed => "#ff8f8f".to_string(),
        Color::LightGreen => "#9df0b5".to_string(),
        Color::LightYellow => "#ffdd8c".to_string(),
        Color::LightBlue => "#7bbcff".to_string(),
        Color::LightMagenta => "#d6abff".to_string(),
        Color::LightCyan => "#8ee4ff".to_string(),
        Color::White => "#ffffff".to_string(),
        Color::Indexed(i) => indexed_to_hex(i),
    }
}

/// The xterm 256-colour cube, computed rather than tabulated.
fn indexed_to_hex(i: u8) -> String {
    let (r, g, b) = match i {
        0..=15 => {
            let base = u8::from(i >= 8) * 85;
            let bit = |n: u8| if i & (1 << n) != 0 { 170 } else { 0 };
            (
                base.saturating_add(bit(0)),
                base.saturating_add(bit(1)),
                base.saturating_add(bit(2)),
            )
        }
        16..=231 => {
            let n = i - 16;
            let step = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (step(n / 36), step((n / 6) % 6), step(n % 6))
        }
        232..=255 => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
        }
    };
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}
