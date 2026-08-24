//! Every key binding, written down once.
//!
//! There were four copies: the `match` arms in the event loop, the `ROWS` table the `?` overlay
//! draws, the `KEYS` block in `cli::print_help`, and the README's panel table — plus a fifth in
//! prose in `AGENTS.md`. Nothing kept them in step, so adding a panel meant remembering five
//! edits and a reviewer catching the ones that were missed.
//!
//! Now the event loop dispatches from [`BINDINGS`], the overlay and `--help` render from it, and
//! `tests/docs.rs` fails the build when the README's table disagrees.

use crate::model::Range;
use crate::ui::app::Panel;

/// What a key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
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
    /// arrow keys — are handled beside the table in the event loop and named in `alias`.
    pub key: char,
    /// How the key column reads in the overlay and in `--help`.
    pub shown_as: &'static str,
    /// Extra keys that do the same thing, for the description only.
    pub alias: Option<&'static str>,
    pub action: Action,
    /// One line, in the user's terms rather than the code's.
    pub what: &'static str,
}

/// The bindings, in the order they are shown.
pub const BINDINGS: &[Binding] = &[
    Binding {
        key: '1',
        shown_as: "1 2 3 4",
        alias: None,
        action: Action::Range(Range::Today),
        what: "range: today, 7 days, 30 days, all time",
    },
    Binding {
        key: '2',
        shown_as: "",
        alias: None,
        action: Action::Range(Range::Week),
        what: "",
    },
    Binding {
        key: '3',
        shown_as: "",
        alias: None,
        action: Action::Range(Range::Month),
        what: "",
    },
    Binding {
        key: '4',
        shown_as: "",
        alias: None,
        action: Action::Range(Range::All),
        what: "",
    },
    Binding {
        key: 'r',
        shown_as: "r",
        alias: None,
        action: Action::Refresh,
        what: "refresh now",
    },
    Binding {
        key: 'b',
        shown_as: "b",
        alias: None,
        action: Action::Panel(Panel::Budgets),
        what: "budgets",
    },
    Binding {
        key: 't',
        shown_as: "t",
        alias: None,
        action: Action::Panel(Panel::Routing),
        what: "routing analytics and derived escalations",
    },
    Binding {
        key: 'p',
        shown_as: "p",
        alias: None,
        action: Action::Panel(Panel::Projects),
        what: "cost per project",
    },
    Binding {
        key: 'g',
        shown_as: "g",
        alias: None,
        action: Action::Panel(Panel::TimeSeries),
        what: "spend over time",
    },
    Binding {
        key: 'w',
        shown_as: "w",
        alias: None,
        action: Action::Panel(Panel::Burn),
        what: "burn rate and time to budget",
    },
    Binding {
        key: 's',
        shown_as: "s",
        alias: None,
        action: Action::Panel(Panel::Sessions),
        what: "sessions",
    },
    Binding {
        key: 'l',
        shown_as: "l",
        alias: None,
        action: Action::Panel(Panel::Limits),
        what: "subscription limits, from Omarchy's agents panel",
    },
    Binding {
        key: 'j',
        shown_as: "j / k",
        alias: Some("also arrow keys"),
        action: Action::SelectNext,
        what: "move the selection (also arrow keys)",
    },
    Binding {
        key: 'k',
        shown_as: "",
        alias: None,
        action: Action::SelectPrev,
        what: "",
    },
    Binding {
        key: '?',
        shown_as: "?",
        alias: None,
        action: Action::ToggleHelp,
        what: "key reference (press again to close)",
    },
    Binding {
        key: 'q',
        shown_as: "q",
        alias: Some("also Esc, Ctrl-C"),
        action: Action::Quit,
        what: "quit (also Esc, Ctrl-C)",
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
