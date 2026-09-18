//! The frame around the panels: the tab strip, the hero row's share strip, the left rail, and
//! what the tables gained -- counts, bars and scrollbars.

use super::*;
use crate::ui::aggregate::{share_cells, share_label};
use crate::ui::panels::tabs::tab_line;

fn line_text(line: &ratatui::text::Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// The word each panel goes by in the strip, read from where the strip reads it.
fn word(panel: Panel) -> &'static str {
    crate::ui::keys::BINDINGS
        .iter()
        .find(|b| b.action == crate::ui::keys::Action::Panel(panel))
        .and_then(|b| b.hint.map(|(_, word)| word))
        .unwrap_or("models")
}

#[test]
fn every_panel_is_in_the_tab_strip_when_there_is_room() {
    let text = line_text(&tab_line(Panel::Models, 132));
    for panel in Panel::ALL {
        assert!(
            text.contains(word(*panel)),
            "{panel:?} is missing from {text:?}"
        );
    }
}

/// Whatever the width, the strip says which panel is showing, and says it in reverse video so
/// that `NO_COLOR` cannot take the marking away.
#[test]
fn the_active_panel_is_named_and_marked_at_every_width() {
    for panel in Panel::ALL {
        for width in 16..=200u16 {
            let line = tab_line(*panel, width);
            let active: Vec<_> = line
                .spans
                .iter()
                .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
                .collect();
            assert_eq!(active.len(), 1, "{panel:?} at {width}");
            assert_eq!(
                active[0].content.trim(),
                word(*panel),
                "{panel:?} at {width}"
            );
            // The narrowest form is the fallback; every wider one must actually fit.
            if line.width() > usize::from(width) {
                assert_eq!(
                    line_text(&line).trim(),
                    word(*panel),
                    "{panel:?} at {width}"
                );
            }
        }
    }
}

#[test]
fn share_cells_fill_the_width_exactly() {
    for width in [5u16, 17, 80, 130, 199] {
        let cells = share_cells(&[3_300, 3_400, 5_500, 3_500, 0], width);
        assert_eq!(cells.iter().sum::<u16>(), width, "{cells:?}");
        assert_eq!(cells[4], 0, "no tokens, no cell: {cells:?}");
    }
}

/// `width * weight / total` rounds a small category to no cells at all, which draws "there was
/// none" about a category that has usage.
#[test]
fn a_tiny_share_still_gets_a_cell_and_an_empty_one_never_does() {
    let cells = share_cells(&[1, 10_000_000, 0], 40);
    assert_eq!(cells, vec![1, 39, 0]);
}

#[test]
fn nothing_to_divide_draws_no_strip() {
    assert!(share_cells(&[0, 0, 0], 40).is_empty());
    assert!(share_cells(&[], 40).is_empty());
    // Fewer cells than categories that need one: no honest strip exists.
    assert!(share_cells(&[1, 1, 1], 2).is_empty());
}

#[test]
fn a_share_is_never_rounded_to_zero_percent() {
    assert_eq!(share_label(1, 10_000).as_deref(), Some("<1%"));
    assert_eq!(share_label(21, 100).as_deref(), Some("21%"));
    assert_eq!(share_label(0, 100), None);
    assert_eq!(share_label(0, 0), None);
}

#[test]
fn a_category_with_no_tokens_shows_a_dash_not_zero_percent() {
    let mut app = test_app(vec![usage(None, None, Some(1.0), 100)]);
    app.recompute();
    let rendered = render_metrics(&app, 130, 5);
    assert!(!rendered.contains(" 0%"), "{rendered}");
    assert!(rendered.contains('—'), "{rendered}");
    // The one category there is names itself in the strip under the tiles.
    assert!(rendered.contains("PAID 100%"), "{rendered}");
}

fn rail_app() -> App {
    let mut app = test_app(vec![
        usage_at("2026-09-01", 1_000, Some(1.0)),
        usage_at("2026-09-02", 3_000, Some(2.0)),
    ]);
    app.recompute();
    app.set_limits_for_test(fixture_limits(false));
    app
}

#[test]
fn a_tall_rail_shows_the_tokens_the_limits_and_the_days() {
    let rendered = render_breakdown(&rail_app(), 40, 30);
    for expected in [
        "TOKEN FLOW",
        "LIMITS",
        "92%",
        "TOKENS PER DAY",
        "peak 3.0K on 2026-09-02",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected:?}:\n{rendered}"
        );
    }
}

/// ratatui squeezes constraints that do not fit, which drew a border around nothing. A section
/// that does not fit whole is left out, last first, and the token list is never the one to go.
#[test]
fn a_short_rail_drops_whole_sections_from_the_bottom() {
    let app = rail_app();
    let shortest = render_breakdown(&app, 40, 11);
    assert!(shortest.contains("PRICING"), "{shortest}");
    assert!(!shortest.contains("LIMITS"), "{shortest}");
    assert!(!shortest.contains("TOKENS PER DAY"), "{shortest}");

    let middling = render_breakdown(&app, 40, 16);
    assert!(
        middling.contains("LIMITS") && middling.contains("41%"),
        "{middling}"
    );
    assert!(!middling.contains("TOKENS PER DAY"), "{middling}");
}

/// The counterfactual makes the list one row taller than the shortest pane. The blank line goes;
/// the pricing status, which is the last line, does not.
#[test]
fn the_shortest_rail_keeps_every_line_that_says_something() {
    let now = crate::utils::now();
    let mut app = test_app(
        (0..3)
            .map(|i| subscription_usage(20_000, now - i))
            .collect(),
    );
    app.recompute();
    let rendered = render_breakdown(&app, 40, 11);
    for expected in ["API-RATE EQUIV.", "not billed", "PRICING"] {
        assert!(
            rendered.contains(expected),
            "missing {expected:?}:\n{rendered}"
        );
    }
}

/// Stale is set by hand on the fresh fixture rather than by loading it three hours later: by
/// then the 92% window has also reset, which stops the alarm on its own and would let this pass
/// with the staleness rule deleted.
#[test]
fn a_stale_window_is_not_alarming_in_the_rail_or_the_panel() {
    let mut app = rail_app();
    let rail = |app: &App| {
        colour_of(
            40,
            30,
            |f, a| crate::ui::panels::breakdown::draw_breakdown(f, a, app),
            "92%",
        )
    };
    let panel = |app: &App| {
        colour_of(
            100,
            12,
            |f, a| crate::ui::panels::limits::draw_limits(f, a, app),
            "92%",
        )
    };
    assert_eq!(rail(&app), Some(crate::model::RED));
    assert_eq!(panel(&app), Some(crate::model::RED));

    let mut limits = fixture_limits(false);
    for snapshot in &mut limits.snapshots {
        snapshot.stale = true;
    }
    app.set_limits_for_test(limits);
    assert_ne!(rail(&app), Some(crate::model::RED));
    assert_ne!(panel(&app), Some(crate::model::RED));
}

/// The same windows twice on one screen reads as two sources that happen to agree.
#[test]
fn the_rail_leaves_out_what_the_open_panel_already_shows() {
    let mut app = rail_app();
    app.panel = Panel::Limits;
    let rendered = render_breakdown(&app, 40, 30);
    assert!(!rendered.contains("92%"), "{rendered}");
    assert!(rendered.contains("TOKENS PER DAY"), "{rendered}");

    app.panel = Panel::TimeSeries;
    let rendered = render_breakdown(&app, 40, 30);
    assert!(rendered.contains("92%"), "{rendered}");
    assert!(!rendered.contains("TOKENS PER DAY"), "{rendered}");
}

#[test]
fn a_rail_with_no_limits_and_no_days_is_just_the_token_list() {
    let app = test_app(Vec::new());
    let rendered = render_breakdown(&app, 40, 30);
    assert!(rendered.contains("TOKEN FLOW"));
    assert!(!rendered.contains("LIMITS") && !rendered.contains("TOKENS PER DAY"));
}

fn models(count: usize) -> App {
    let mut app = test_app(
        (0..count)
            .map(|i| Usage {
                model: format!("model-{i:02}"),
                ..usage(None, None, Some(1.0), 100 * (i as u64 + 1))
            })
            .collect(),
    );
    app.recompute();
    app
}

fn render_models(app: &App, w: u16, h: u16) -> String {
    render_panel(w, h, |frame, area| {
        crate::ui::panels::models::draw_models(frame, area, app)
    })
}

#[test]
fn the_model_table_says_how_many_rows_it_has() {
    assert!(render_models(&models(3), 100, 12).contains("3 models"));
    let one = render_models(&models(1), 100, 12);
    assert!(
        one.contains("1 model ") && !one.contains("1 models"),
        "{one}"
    );
}

/// A thumb on a list with nothing to scroll says there is more where there is not.
#[test]
fn a_scrollbar_appears_only_when_rows_are_off_screen() {
    assert!(!render_models(&models(3), 100, 12).contains('┃'));
    assert!(render_models(&models(40), 100, 12).contains('┃'));
}

#[test]
fn token_bars_need_room_and_the_figures_never_wait_for_it() {
    let app = models(3);
    let wide = render_models(&app, 100, 12);
    let narrow = render_models(&app, 60, 12);
    assert!(wide.contains('█'), "{wide}");
    assert!(!narrow.contains('█'), "{narrow}");
    assert!(narrow.contains("300"), "{narrow}");
}
