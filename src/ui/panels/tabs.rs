//! The tab strip: which panel is showing, and that there are others.
//!
//! Before this row existed the only sign that the right-hand pane could show anything else was
//! the footer's key hints, and nothing at all said which of the eight it was showing beyond the
//! pane's own title.
//!
//! The words come from the footer hints in `keys::BINDINGS`, so a panel added to the table is in
//! the strip without anyone remembering it, and the strip cannot call a panel something the
//! footer does not.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::model::CYAN;
use crate::ui::app::{App, Panel};
use crate::ui::keys::{self, Action};
use crate::ui::theme::{HEADER_BG, MUTED};

pub fn draw_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let strip = Paragraph::new(tab_line(app.panel, area.width))
        .style(Style::default().bg(HEADER_BG).fg(MUTED));
    frame.render_widget(strip, area);
}

/// The key and word a panel goes by. `Models` has neither: it is where the dashboard starts and
/// where pressing a panel's key a second time returns to, so no key of its own is bound to it.
fn name(panel: Panel) -> (Option<char>, &'static str) {
    keys::BINDINGS
        .iter()
        .find(|binding| binding.action == Action::Panel(panel))
        .and_then(|binding| binding.hint.map(|(_, word)| (Some(binding.key), word)))
        .unwrap_or((None, "models"))
}

/// The strip at the widest form that fits: every panel by name; the active one by name and the
/// rest by key; the active one alone. Measured rather than thresholded, as the footer is -- a
/// width written down here would be a copy of the bindings table that goes stale with it.
///
/// Whatever the width, the active panel's name is in it.
pub(crate) fn tab_line<'a>(active: Panel, width: u16) -> Line<'a> {
    let forms = [Form::Words, Form::Keys, Form::ActiveOnly];
    let mut lines = forms.iter().map(|form| build(active, *form));
    let last = lines.next_back().expect("there is a narrowest form");
    lines
        .find(|line| line.width() <= usize::from(width))
        .unwrap_or(last)
}

#[derive(Clone, Copy)]
enum Form {
    Words,
    Keys,
    ActiveOnly,
}

fn build<'a>(active: Panel, form: Form) -> Line<'a> {
    let mut spans = vec![Span::raw(" ")];
    for panel in Panel::ALL {
        let (key, word) = name(*panel);
        if *panel == active {
            // Reverse video rather than a background colour: `NO_COLOR` strips every colour from
            // the frame, and a tab marked only by its background would stop being marked.
            spans.push(Span::styled(
                format!(" {word} "),
                Style::default()
                    .fg(CYAN)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            ));
            spans.push(Span::raw(" "));
            continue;
        }
        match (form, key) {
            (Form::Words, _) => spans.push(Span::raw(format!(" {word}  "))),
            (Form::Keys, Some(key)) => spans.push(Span::raw(format!("{key} "))),
            _ => {}
        }
    }
    Line::from(spans)
}
