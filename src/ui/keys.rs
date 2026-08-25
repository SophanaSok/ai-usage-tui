//! Every key binding, written down once.
//!
//! There were four copies: the `match` arms in the event loop, the `ROWS` table the `?` overlay
//! draws, the `KEYS` block in `cli::print_help`, and the README's panel table — plus a fifth in
//! prose in `AGENTS.md`. Nothing kept them in step, so adding a panel meant remembering five
//! edits and a reviewer catching the ones that were missed.
//!
//! Now the event loop dispatches from [`BINDINGS`], the overlay, `--help` and the footer render
//! from it, and `tests/docs.rs` fails the build when the README's table disagrees.
//!
//! The footer was the holdout: it abbreviates (`1-4`, `j/k`) and folds the panel keys into one
//! run below a certain width, so it kept its own copy — and its own idea of the width at which
//! the full line stops fitting, which was a number that drifted as panels were added. Each
//! binding now carries its footer spelling in [`Binding::hint`], [`footer_forms`] derives the
//! three forms from the table, and the footer takes the widest one that measures as fitting.

use crate::model::Range;
use crate::ui::app::Panel;

/// What a key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    /// Scope the sessions view to the project under the cursor.
    DrillIn,
    /// Start typing a row filter.
    Search,
    /// Move the sort to the next column of the visible panel.
    SortNext,
    /// Move it to the previous column.
    SortPrev,
    /// Reverse the visible panel's sort.
    SortReverse,
    /// Leave a drilldown. Falls through to `Quit` when there is nothing to leave.
    Back,
    ToggleHelp,
    Refresh,
    /// Show this panel, or return to the model list if it is already showing.
    Panel(Panel),
    Range(Range),
    SelectNext,
    SelectPrev,
}

pub struct Binding {
    /// The character that triggers it. Aliases that are not characters — `Esc`, `Ctrl-C`, the
    /// arrow keys — are handled beside the table in the event loop and named in `what`.
    pub key: char,
    /// How the key column reads in the overlay and in `--help`.
    pub shown_as: &'static str,
    pub action: Action,
    /// One line, in the user's terms rather than the code's.
    pub what: &'static str,
    /// How the footer names it: the key as the footer spells it, and one word. `None` keeps it
    /// out of the footer — the sort keys, `/`, `Enter` and `Backspace` live in the `?` overlay,
    /// which the footer's `? help` points at.
    pub hint: Option<(&'static str, &'static str)>,
}

/// The bindings, in the order they are shown.
pub const BINDINGS: &[Binding] = &[
    Binding {
        key: '1',
        shown_as: "1 2 3 4",
        action: Action::Range(Range::Today),
        what: "range: today, 7 days, 30 days, all time",
        hint: Some(("1-4", "range")),
    },
    Binding {
        key: '2',
        shown_as: "",
        action: Action::Range(Range::Week),
        what: "",
        hint: None,
    },
    Binding {
        key: '3',
        shown_as: "",
        action: Action::Range(Range::Month),
        what: "",
        hint: None,
    },
    Binding {
        key: '4',
        shown_as: "",
        action: Action::Range(Range::All),
        what: "",
        hint: None,
    },
    Binding {
        key: 'r',
        shown_as: "r",
        action: Action::Refresh,
        what: "refresh now",
        hint: Some(("r", "refresh")),
    },
    Binding {
        key: 'b',
        shown_as: "b",
        action: Action::Panel(Panel::Budgets),
        what: "budgets",
        hint: Some(("b", "budgets")),
    },
    Binding {
        key: 't',
        shown_as: "t",
        action: Action::Panel(Panel::Routing),
        what: "routing analytics and derived escalations",
        hint: Some(("t", "routing")),
    },
    Binding {
        key: 'p',
        shown_as: "p",
        action: Action::Panel(Panel::Projects),
        what: "cost per project",
        hint: Some(("p", "projects")),
    },
    Binding {
        key: 'g',
        shown_as: "g",
        action: Action::Panel(Panel::TimeSeries),
        what: "spend over time",
        hint: Some(("g", "graph")),
    },
    Binding {
        key: 'w',
        shown_as: "w",
        action: Action::Panel(Panel::Burn),
        what: "burn rate and time to budget",
        hint: Some(("w", "burn")),
    },
    Binding {
        key: 's',
        shown_as: "s",
        action: Action::Panel(Panel::Sessions),
        what: "sessions",
        hint: Some(("s", "sessions")),
    },
    Binding {
        key: 'l',
        shown_as: "l",
        action: Action::Panel(Panel::Limits),
        what: "subscription limits, from Omarchy's agents panel",
        hint: Some(("l", "limits")),
    },
    Binding {
        key: 'j',
        shown_as: "j / k",
        action: Action::SelectNext,
        what: "move the selection (also arrow keys)",
        hint: Some(("j/k", "move")),
    },
    Binding {
        key: 'k',
        shown_as: "",
        action: Action::SelectPrev,
        what: "",
        hint: None,
    },
    Binding {
        key: '>',
        shown_as: "< >",
        action: Action::SortNext,
        what: "sort by the next / previous column",
        hint: None,
    },
    Binding {
        key: '<',
        shown_as: "",
        action: Action::SortPrev,
        what: "",
        hint: None,
    },
    // The unshifted keys in the same place, folded into the row above rather than listed twice.
    Binding {
        key: '.',
        shown_as: "",
        action: Action::SortNext,
        what: "",
        hint: None,
    },
    Binding {
        key: ',',
        shown_as: "",
        action: Action::SortPrev,
        what: "",
        hint: None,
    },
    Binding {
        key: 'o',
        shown_as: "o",
        action: Action::SortReverse,
        what: "reverse the sort order",
        hint: None,
    },
    Binding {
        key: '/',
        shown_as: "/",
        action: Action::Search,
        what: "filter the rows shown (Enter keeps it, Esc clears it)",
        hint: None,
    },
    Binding {
        key: '\n',
        shown_as: "Enter",
        action: Action::DrillIn,
        what: "on a project row: show just that project's sessions",
        hint: None,
    },
    Binding {
        key: '\u{8}',
        shown_as: "Backspace",
        action: Action::Back,
        what: "leave a project and go back to the list",
        hint: None,
    },
    Binding {
        key: '?',
        shown_as: "?",
        action: Action::ToggleHelp,
        what: "key reference (press again to close)",
        hint: Some(("?", "help")),
    },
    Binding {
        key: 'q',
        shown_as: "q",
        action: Action::Quit,
        // Esc is no longer unconditionally quit: inside a project it goes back. Saying "also
        // Esc" here would be wrong exactly when a reader is most likely to look.
        what: "quit (also Ctrl-C; Esc clears a filter or leaves a project first)",
        hint: Some(("q", "quit")),
    },
];

/// The action a character is bound to, if any.
pub fn action_for(key: char) -> Option<Action> {
    BINDINGS
        .iter()
        .find(|binding| binding.key == key)
        .map(|binding| binding.action)
}

/// The rows the `?` overlay and `--help` show: the bindings that carry their own description.
///
/// `2`, `3`, `4` and `k` are real bindings folded into a neighbour's row (`1 2 3 4`, `j / k`),
/// so they are dispatched but not listed twice.
pub fn rows() -> impl Iterator<Item = (&'static str, &'static str)> {
    BINDINGS
        .iter()
        .filter(|binding| !binding.shown_as.is_empty())
        .map(|binding| (binding.shown_as, binding.what))
}

/// One footer hint: the key as the footer spells it, and one word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub key: String,
    pub word: &'static str,
}

/// The footer's three forms, widest first: every hint; the panel keys folded into one run
/// (`btpgwsl panels`); and only `? help  q quit`. The footer takes the first that fits its
/// width, measured — not at a threshold, which is a fact about this table that drifted every
/// time a panel was added and once hid `q quit` on an 80-column terminal.
///
/// The fold is by action: every `Action::Panel` binding with a hint is in the run, so a panel
/// added to the table is in the compact footer without anyone remembering it.
pub fn footer_forms() -> [Vec<Hint>; 3] {
    let hint = |binding: &Binding| {
        binding.hint.map(|(key, word)| Hint {
            key: key.to_string(),
            word,
        })
    };
    let is_panel = |binding: &Binding| matches!(binding.action, Action::Panel(_));

    let full: Vec<Hint> = BINDINGS.iter().filter_map(hint).collect();

    let mut compact = Vec::new();
    let mut panels_folded = false;
    for binding in BINDINGS {
        let Some(hint) = hint(binding) else {
            continue;
        };
        if is_panel(binding) {
            if !panels_folded {
                compact.push(Hint {
                    key: BINDINGS
                        .iter()
                        .filter(|b| is_panel(b) && b.hint.is_some())
                        .map(|b| b.key)
                        .collect(),
                    word: "panels",
                });
                panels_folded = true;
            }
            continue;
        }
        compact.push(hint);
    }

    let minimal: Vec<Hint> = BINDINGS
        .iter()
        .filter(|binding| matches!(binding.action, Action::ToggleHelp | Action::Quit))
        .filter_map(hint)
        .collect();

    [full, compact, minimal]
}

/// Every panel binding, for the README table check in `tests/docs.rs`.
pub fn panel_keys() -> impl Iterator<Item = (char, Panel)> {
    BINDINGS.iter().filter_map(|binding| match binding.action {
        Action::Panel(panel) => Some((binding.key, panel)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_binding_is_unique_and_resolvable() {
        let mut seen = std::collections::HashSet::new();
        for binding in BINDINGS {
            assert!(
                seen.insert(binding.key),
                "duplicate binding {:?}",
                binding.key
            );
            assert_eq!(action_for(binding.key), Some(binding.action));
        }
        assert!(action_for('X').is_none());
    }

    /// Every panel must be reachable. A `Panel` variant added without a key is a panel the user
    /// cannot open, and nothing else would say so.
    #[test]
    fn every_panel_except_the_default_has_a_key() {
        let bound: Vec<Panel> = panel_keys().map(|(_, panel)| panel).collect();
        for panel in [
            Panel::Budgets,
            Panel::Routing,
            Panel::Projects,
            Panel::TimeSeries,
            Panel::Burn,
            Panel::Sessions,
            Panel::Limits,
        ] {
            assert!(bound.contains(&panel), "{panel:?} has no key binding");
        }
        // `Panel::Models` is the default view, returned to by pressing an active panel's key
        // again; it deliberately has no key of its own.
        assert!(!bound.contains(&Panel::Models));
    }

    /// A panel without a footer hint is a panel the footer cannot name in any form; the overlay
    /// would still list it, but the footer is what a user sees without asking.
    #[test]
    fn every_panel_binding_has_a_footer_hint_and_the_fold_names_them_all() {
        for binding in BINDINGS {
            if let Action::Panel(panel) = binding.action {
                assert!(binding.hint.is_some(), "{panel:?} has no footer hint");
            }
            // The footer's spelling is hand-written beside the key it spells, and the two forms
            // read different ones: the full form prints the spelling, the fold prints the keys.
            // A slip such as `("s", "limits")` on the `l` binding would ship a footer naming the
            // wrong key with every length unchanged, so nothing else would notice.
            if let Some((spelling, _)) = binding.hint {
                assert!(
                    spelling.starts_with(binding.key),
                    "{:?}'s footer hint {spelling:?} does not start with its key {:?}",
                    binding.action,
                    binding.key
                );
            }
        }
        let [full, compact, minimal] = footer_forms();
        let run = compact
            .iter()
            .find(|hint| hint.word == "panels")
            .expect("the compact form folds the panels");
        let expected: String = panel_keys().map(|(key, _)| key).collect();
        assert_eq!(run.key, expected);
        // The fold replaces the panel hints and nothing else.
        assert_eq!(full.len(), compact.len() + panel_keys().count() - 1);
        assert_eq!(
            minimal.iter().map(|h| h.key.as_str()).collect::<Vec<_>>(),
            ["?", "q"]
        );
        // Each form is shorter than the one before it, or the footer would never choose it.
        let width = |form: &[Hint]| {
            form.iter()
                .map(|h| h.key.len() + h.word.len())
                .sum::<usize>()
        };
        assert!(width(&full) > width(&compact) && width(&compact) > width(&minimal));
    }

    #[test]
    fn the_shown_rows_cover_every_action_kind() {
        let rows: Vec<_> = rows().collect();
        assert!(rows
            .iter()
            .all(|(keys, what)| !keys.is_empty() && !what.is_empty()));
        for needle in ["q", "?", "r", "j / k", "1 2 3 4"] {
            assert!(
                rows.iter().any(|(keys, _)| *keys == needle),
                "{needle:?} is missing from the key reference"
            );
        }
    }
}
