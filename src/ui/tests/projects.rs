//! Per-project cost attribution and the labels the panel draws for it.

use super::*;

#[test]
fn projects_are_ranked_by_cost_and_count_distinct_sessions() {
    let rows = project_totals(&[
        usage(Some("/w/api"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/api"), Some("s1"), Some(2.0), 100),
        usage(Some("/w/api"), Some("s2"), Some(3.0), 100),
        usage(Some("/w/docs"), Some("s3"), Some(10.0), 50),
    ]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].project, "/w/docs");
    assert_eq!(rows[1].project, "/w/api");
    assert_eq!(rows[1].cost, 6.0);
    assert_eq!(rows[1].sessions, 2, "two sessions, three requests");
    assert_eq!(rows[1].requests, 3);
}

#[test]
fn usage_without_a_project_is_still_accounted_for() {
    // Dropping unattributed rows would make the per-project totals quietly disagree with
    // the headline total — the same class of bug as two panels disagreeing on PAID.
    let rows = project_totals(&[
        usage(Some("/w/api"), None, Some(1.0), 100),
        usage(None, None, Some(4.0), 100),
    ]);
    let total: f64 = rows.iter().map(|r| r.cost).sum();
    assert_eq!(total, 5.0);
    assert!(rows.iter().any(|r| r.project == "(unattributed)"));
}

#[test]
fn a_project_with_unpriced_requests_reports_them() {
    let rows = project_totals(&[
        usage(Some("/w/api"), None, Some(1.0), 100),
        usage(Some("/w/api"), None, None, 100),
    ]);
    assert_eq!(rows[0].cost, 1.0);
    assert_eq!(
        rows[0].unpriced_requests, 1,
        "an unpriced request must not vanish into a confident total"
    );
}

#[test]
fn the_projects_view_and_the_headline_total_agree() {
    // Two panels reporting different numbers for the same data is the exact bug 1.8b was.
    let mut app = test_app(vec![
        usage(Some("/w/api"), Some("s1"), Some(1.5), 100),
        usage(Some("/w/docs"), Some("s2"), Some(2.5), 100),
        usage(None, None, Some(1.0), 100),
    ]);
    app.recompute();
    let per_project: f64 = app.projects().iter().map(|p| p.cost).sum();
    assert!((per_project - app.totals().cost).abs() < 1e-9);
}

#[test]
fn project_labels_lengthen_only_where_they_would_collide() {
    let labels = project_labels(&[
        "/home/dev/api/build".to_string(),
        "/home/dev/web/build".to_string(),
        "/home/dev/ai-usage-tui".to_string(),
    ]);
    assert_eq!(labels[0], "api/build");
    assert_eq!(labels[1], "web/build");
    assert_eq!(
        labels[2], "ai-usage-tui",
        "an unambiguous name should not be lengthened"
    );
}

#[test]
fn project_labels_handle_a_single_project_and_windows_paths() {
    assert_eq!(project_labels(&["/home/dev/app".into()]), vec!["app"]);
    assert_eq!(project_labels(&["C:\\src\\my-app".into()]), vec!["my-app"]);
    assert_eq!(
        project_labels(&["(unattributed)".into()]),
        vec!["(unattributed)"]
    );
}

#[test]
fn two_projects_sharing_a_basename_are_not_merged() {
    // The rollup keys on the full path; only the label is shortened.
    let mut a = usage(Some("/home/dev/api/build"), None, Some(1.0), 100);
    a.project = Some("/home/dev/api/build".into());
    let mut b = usage(Some("/home/dev/web/build"), None, Some(2.0), 100);
    b.project = Some("/home/dev/web/build".into());
    let rows = project_totals(&[a, b]);
    assert_eq!(rows.len(), 2, "distinct projects were merged by basename");
}

#[test]
fn a_project_of_only_quota_work_is_not_rendered_as_free() {
    let now = crate::utils::now();
    let mut app = test_app((0..3).map(|i| quota_usage(50_000, now - i)).collect());
    app.recompute();

    let rendered = render_projects(&app, 84, 8);
    assert!(rendered.contains("quota"), "{rendered}");
    assert!(!rendered.contains("$0.00"), "{rendered}");
}

/// Enter on a project row scopes the sessions view to that project.
#[test]
fn drilling_into_a_project_scopes_the_sessions_view() {
    let mut app = test_app(vec![
        usage(Some("/w/api"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/api"), Some("s2"), Some(2.0), 100),
        usage(Some("/w/docs"), Some("s3"), Some(10.0), 50),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    // Ranked by cost, so /w/docs is first.
    assert_eq!(app.projects()[0].project, "/w/docs");
    assert_eq!(app.sessions().len(), 3, "all sessions before drilling");

    assert!(app.drill_into_selected_project());
    assert_eq!(app.panel, Panel::Sessions);
    assert_eq!(app.drilldown_project(), Some("/w/docs"));
    let sessions = app.sessions();
    assert_eq!(sessions.len(), 1, "only /w/docs' sessions");
    assert_eq!(sessions[0].session_id, "s3");

    // And back, to the row we came from.
    assert!(app.leave_drilldown());
    assert_eq!(app.panel, Panel::Projects);
    assert_eq!(app.drilldown_project(), None);
    assert_eq!(app.sessions().len(), 3, "the full list is restored");
}

/// Leaving lands on the row the user drilled from, not at the top of the list.
#[test]
fn leaving_a_drilldown_returns_to_the_row_it_started_from() {
    let mut app = test_app(vec![
        usage(Some("/w/api"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/docs"), Some("s2"), Some(10.0), 50),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    app.selected = 1; // /w/api, the cheaper one
    assert!(app.drill_into_selected_project());
    assert_eq!(app.drilldown_project(), Some("/w/api"));
    assert_eq!(app.selected, 0, "the session list starts at its own top");

    assert!(app.leave_drilldown());
    assert_eq!(app.selected, 1, "back where the user left the project list");
}

/// Usage with no project is filed under one row, and drilling into it must find the sessions
/// that have none — matching on the label would find nothing at all.
#[test]
fn the_unattributed_row_drills_into_the_sessions_with_no_project() {
    let mut app = test_app(vec![
        usage(None, Some("s-none"), Some(5.0), 100),
        usage(Some("/w/api"), Some("s1"), Some(1.0), 100),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    let row = app
        .projects()
        .iter()
        .position(|p| p.project == crate::ui::aggregate::UNATTRIBUTED)
        .expect("an unattributed row");
    app.selected = row;

    assert!(app.drill_into_selected_project());
    let sessions = app.sessions();
    assert_eq!(sessions.len(), 1, "the session with no project");
    assert_eq!(sessions[0].session_id, "s-none");
    assert_eq!(sessions[0].project, None);
}

/// Enter does nothing outside the Projects panel, and nothing on an empty list.
#[test]
fn drilling_is_refused_where_there_is_nothing_to_drill_into() {
    let mut app = test_app(vec![usage(Some("/w/api"), Some("s1"), Some(1.0), 100)]);

    app.recompute();
    app.toggle_panel(Panel::Sessions);
    assert!(!app.drill_into_selected_project(), "not the projects panel");
    assert_eq!(app.drilldown_project(), None);

    app.toggle_panel(Panel::Projects);
    app.selected = 99; // past the end
    assert!(
        !app.drill_into_selected_project(),
        "no row under the cursor"
    );
    assert_eq!(app.drilldown_project(), None);

    // And leaving when not inside one reports that, so Esc can fall through to quitting.
    assert!(!app.leave_drilldown());
}

/// The cursor is bounded by the project list, not the model table.
///
/// `visible_rows` returned the model-table length for every panel but Sessions. Harmless while
/// nothing acted on the row; wrong the moment Enter drills into whatever is under it.
#[test]
fn the_cursor_is_bounded_by_the_project_list() {
    let mut app = test_app(vec![
        usage(Some("/w/api"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/docs"), Some("s2"), Some(2.0), 100),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    assert_eq!(app.visible_rows(), app.projects().len());
    assert_eq!(app.visible_rows(), 2);
}

/// The cursor is clamped by the panel showing, not by the model table behind it.
///
/// `recompute` ended with `selected.min(view.rows.len() - 1)` regardless of panel, so on a
/// machine with few model groups and many projects the cursor could not reach the later
/// projects at all. Cosmetic until Enter started acting on the row under it.
#[test]
fn the_cursor_is_not_clamped_by_the_model_table() {
    // One provider/model across four projects: the model table has a single row.
    let mut app = test_app(vec![
        usage(Some("/w/a"), Some("s1"), Some(4.0), 100),
        usage(Some("/w/b"), Some("s2"), Some(3.0), 100),
        usage(Some("/w/c"), Some("s3"), Some(2.0), 100),
        usage(Some("/w/d"), Some("s4"), Some(1.0), 100),
    ]);
    app.recompute();
    assert_eq!(app.rows().len(), 1, "one model group behind four projects");

    app.toggle_panel(Panel::Projects);
    app.selected = 3;
    app.recompute();
    assert_eq!(app.selected, 3, "the fourth project must stay reachable");

    // And drilling from it picks that project, not whatever the clamp left behind.
    assert!(app.drill_into_selected_project());
    assert_eq!(app.drilldown_project(), Some("/w/d"));
}

/// Leaving a drilldown finds the project by name, not by the row number it was at.
///
/// Sorting, a `/` filter, or a refresh that adds a project can all move a project to a different
/// index while the user is inside it. Returning them to whatever now sits at the old number
/// would put the cursor on the wrong project without saying so.
#[test]
fn leaving_a_drilldown_finds_the_project_even_if_the_order_changed() {
    let mut app = test_app(vec![
        usage(Some("/w/cheap"), Some("s1"), Some(1.0), 100),
        usage(Some("/w/dear"), Some("s2"), Some(9.0), 100),
    ]);
    app.recompute();
    app.toggle_panel(Panel::Projects);
    // Dearest first, so /w/cheap is row 1.
    assert_eq!(app.projects()[1].project, "/w/cheap");
    app.selected = 1;
    assert!(app.drill_into_selected_project());
    assert_eq!(app.drilldown_project(), Some("/w/cheap"));

    // Reverse the projects sort while inside: /w/cheap is now row 0.
    app.toggle_panel(Panel::Projects);
    app.reverse_sort();
    app.toggle_panel(Panel::Sessions);

    assert!(app.leave_drilldown());
    assert_eq!(
        app.projects()[app.selected].project,
        "/w/cheap",
        "the cursor must land on the project drilled into, not on row 1"
    );
    assert_eq!(app.selected, 0, "which is now the first row");
}
