//! Off-screen SVG rendering, used to produce the README screenshots.

use super::*;

#[test]
fn every_glyph_in_the_buffer_reaches_the_svg() {
    let svg = crate::ui::buffer_to_svg(&styled_buffer("TOTAL 12.4M", 2, Style::default()));

    assert!(svg.contains("TOTAL 12.4M"), "text was dropped:\n{svg}");
}

#[test]
fn markup_in_the_data_is_escaped_rather_than_emitted() {
    // Model and project names come from files this tool does not control. Left unescaped, an
    // angle bracket in one of them produces an SVG that will not parse at all.
    let svg = crate::ui::buffer_to_svg(&styled_buffer("a<b>&c", 0, Style::default()));

    assert!(svg.contains("a&lt;b&gt;&amp;c"), "not escaped:\n{svg}");
    assert!(
        !svg.contains("a<b>"),
        "raw markup reached the output:\n{svg}"
    );
}

#[test]
fn an_unstyled_cell_is_not_drawn_in_the_background_colour() {
    // `Color::Reset` means "the terminal's default", which is a different colour for the
    // foreground and the background. Resolving both to the same value renders the frame blank.
    let svg = crate::ui::buffer_to_svg(&styled_buffer("VISIBLE", 0, Style::default()));

    let fill = svg
        .split("<text")
        .nth(1)
        .and_then(|t| t.split("fill=\"").nth(1))
        .and_then(|t| t.split('"').next())
        .expect("a text run");
    assert_ne!(fill, "#0a1014", "unstyled text is invisible:\n{svg}");
}

#[test]
fn reverse_video_swaps_the_two_colours() {
    let style = Style::default()
        .fg(Color::Rgb(1, 2, 3))
        .bg(Color::Rgb(4, 5, 6))
        .add_modifier(Modifier::REVERSED);
    let svg = crate::ui::buffer_to_svg(&styled_buffer("SELECTED", 0, style));

    assert!(
        svg.contains(
            "<rect x=\"12.00\" y=\"32.00\" width=\"76.80\" height=\"20.00\" fill=\"#010203\"/>"
        ),
        "the selected row's background is not the foreground colour:\n{svg}"
    );
    assert!(
        svg.contains("fill=\"#040506\""),
        "the text is not drawn in the background colour:\n{svg}"
    );
}

#[test]
fn a_run_is_pinned_to_the_cell_grid() {
    // Without `textLength`, a rasteriser that substitutes a font whose advance is not exactly
    // 0.6em walks each row out of alignment and the box-drawing borders come apart.
    let svg = crate::ui::buffer_to_svg(&styled_buffer("abcd", 5, Style::default()));

    assert!(
        svg.contains("x=\"60.00\"") && svg.contains("textLength=\"38.40\""),
        "the run is not placed on the grid:\n{svg}"
    );
}

#[test]
fn whitespace_carries_no_text_run() {
    // The background pass already painted it; emitting a `<text>` per blank stretch triples the
    // file size of a mostly-empty frame for nothing.
    let svg = crate::ui::buffer_to_svg(&Buffer::empty(Rect::new(0, 0, 40, 3)));

    assert!(
        !svg.contains("<text"),
        "blank cells produced text runs:\n{svg}"
    );
}

#[test]
fn the_same_frame_renders_identically_twice() {
    // These are committed files. A renderer that varies run to run makes every regeneration a
    // diff, and a real change impossible to spot among the noise.
    let app = test_app(vec![usage(Some("/w/api"), Some("s1"), Some(0.25), 4_000)]);

    assert_eq!(
        crate::ui::render_svg(&app, 132, 38),
        crate::ui::render_svg(&app, 132, 38)
    );
}

#[test]
fn a_rendered_frame_carries_the_panels_a_reader_came_for() {
    let mut app = test_app(vec![usage(Some("/w/api"), Some("s1"), Some(0.25), 4_000)]);
    app.panel = Panel::Projects;
    let svg = crate::ui::render_svg(&app, 132, 38);

    assert!(svg.starts_with("<svg"), "not an SVG document:\n{svg}");
    for expected in ["AI USAGE", "TOTAL TOKENS", "PROJECT COST", "quit"] {
        assert!(svg.contains(expected), "the frame is missing {expected:?}");
    }
}
