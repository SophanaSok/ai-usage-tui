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
