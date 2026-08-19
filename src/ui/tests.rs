//! Tests for the dashboard's state and aggregation.

#![cfg(test)]

use super::*;
use crate::budget::BudgetEngine;
use crate::model::{Category, Usage};
use crate::ui::app::DerivedView;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::model::{CostStatus, Range};
use crate::utils::now;

fn usage(project: Option<&str>, session: Option<&str>, cost: Option<f64>, tokens: u64) -> Usage {
    Usage {
        provider: "anthropic".into(),
        model: "claude-sonnet-5".into(),
        category: Category::Paid,
        cost_status: if cost.is_some() {
            CostStatus::Calculated
        } else {
            CostStatus::Unavailable
        },
        requests: 1,
        input: tokens,
        cost,
        created: now(),
        session_id: session.map(str::to_string),
        project: project.map(str::to_string),
        ..Default::default()
    }
}

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
fn coverage_reports_the_priced_share_of_billable_requests() {
    let c = coverage(&[
        usage(Some("/w"), None, Some(1.0), 100),
        usage(Some("/w"), None, Some(1.0), 100),
        usage(Some("/w"), None, None, 100),
    ]);
    assert_eq!(c.billable_requests, 3);
    assert_eq!(c.priced_requests, 2);
    assert!((c.pct().unwrap() - 66.666).abs() < 0.01);
}

#[test]
fn coverage_of_nothing_is_not_a_hundred_percent() {
    assert_eq!(coverage(&[]).pct(), None);
}

/// A bare `App` with no collector and no I/O, for testing view logic.
fn test_app(usages: Vec<Usage>) -> App {
    App {
        range: Range::All,
        usages,
        selected: 0,
        status: String::new(),
        degraded: false,
        last_refresh: String::new(),
        pulse: 0,
        refresh_interval: Duration::from_secs(30),
        refreshed_at: Instant::now(),
        db_path: None,
        journal_path: PathBuf::from("/tmp/unused-journal.db"),
        claude_dir: None,
        provider_filter: None,
        model_filter: None,
        collector: None,
        panel: Panel::Models,
        budget_engine: BudgetEngine::empty(),
        alerts: Vec::new(),
        alert_sink: None,
        view: DerivedView::default(),
    }
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
fn only_one_panel_can_be_active() {
    // Two independent booleans let "budgets" and "routing" both be on, with one silently
    // winning the draw dispatch.
    let mut app = test_app(Vec::new());
    app.toggle_panel(Panel::Budgets);
    assert_eq!(app.panel, Panel::Budgets);
    app.toggle_panel(Panel::Routing);
    assert_eq!(app.panel, Panel::Routing);
    app.toggle_panel(Panel::Routing);
    assert_eq!(
        app.panel,
        Panel::Models,
        "toggling off returns to the models"
    );
}

#[test]
fn missing_cost_never_displays_as_paid_zero() {
    let usage = Usage {
        cost_status: CostStatus::Calculated,
        cost: None,
        ..Default::default()
    };
    assert_eq!(cost_display(&usage), "CALCULATED / NO COST");
}

#[test]
fn rows_do_not_mix_cost_provenance() {
    let mut app = App {
        range: Range::All,
        usages: vec![
            Usage {
                provider: "zen".into(),
                model: "model".into(),
                category: Category::Paid,
                cost_status: CostStatus::Calculated,
                cost: Some(1.0),
                created: now(),
                ..Default::default()
            },
            Usage {
                provider: "zen".into(),
                model: "model".into(),
                category: Category::Paid,
                cost_status: CostStatus::Estimated,
                cost: Some(2.0),
                created: now(),
                ..Default::default()
            },
        ],
        selected: 0,
        status: String::new(),
        degraded: false,
        last_refresh: String::new(),
        pulse: 0,
        refresh_interval: Duration::from_secs(30),
        refreshed_at: Instant::now(),
        db_path: None,
        journal_path: PathBuf::from("/tmp/unused-journal.db"),
        claude_dir: None,
        provider_filter: None,
        model_filter: None,
        collector: None,
        panel: Panel::Models,
        budget_engine: BudgetEngine::empty(),
        alerts: Vec::new(),
        alert_sink: None,
        view: DerivedView::default(),
    };
    app.recompute();
    assert_eq!(app.rows().len(), 2);
}
