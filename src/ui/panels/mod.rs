//! Panel renderers. One module per panel, each exposing a single `draw_*` function.

pub mod alerts;
pub mod breakdown;
pub mod budgets;
pub mod burn;
pub mod header;
pub mod limits;
pub mod metrics;
pub mod models;
pub mod projects;
pub mod routing;
pub mod sessions;
pub mod timeseries;

/// A header row whose sorted column carries a direction marker.
///
/// The column names come from `Panel::sort_columns`, so the header a user reads and the column
/// a keypress sorts by cannot drift — they are the same list. A sort nobody can see on screen is
/// worse than no sort at all: the rows move and nothing says why.
pub(crate) fn sorted_header(
    app: &crate::ui::app::App,
    panel: crate::ui::app::Panel,
) -> Vec<String> {
    panel
        .sort_columns()
        .iter()
        .enumerate()
        .map(|(index, name)| format!("{name}{}", app.sort_marker(panel, index)))
        .collect()
}
