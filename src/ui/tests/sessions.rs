//! Grouping usage into sessions, and the panel that lists them.

use super::*;

#[test]
fn sessions_are_listed_most_recently_active_first() {
    let sessions = crate::ui::aggregate::session_totals(&[
        session_usage("older", Some("/w/a"), "claude-opus-5", 1_000, Some(1.0)),
        session_usage("newer", Some("/w/b"), "claude-opus-5", 9_000, Some(2.0)),
    ]);
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].session_id, "newer");
    assert_eq!(sessions[1].session_id, "older");
}

#[test]
fn a_session_spans_its_first_and_last_request() {
    let sessions = crate::ui::aggregate::session_totals(&[
        session_usage("s", Some("/w"), "claude-opus-5", 5_000, Some(1.0)),
        session_usage("s", Some("/w"), "claude-opus-5", 1_000, Some(1.0)),
        session_usage("s", Some("/w"), "claude-opus-5", 9_000, Some(1.0)),
    ]);
    assert_eq!(sessions[0].first_seen, 1_000);
    assert_eq!(sessions[0].last_seen, 9_000);
    assert_eq!(sessions[0].duration_secs(), 8_000);
    assert_eq!(sessions[0].requests, 3);
}

#[test]
fn a_session_records_every_model_it_used_once_each() {
    let sessions = crate::ui::aggregate::session_totals(&[
        session_usage("s", Some("/w"), "claude-opus-5", 1_000, Some(1.0)),
        session_usage("s", Some("/w"), "claude-haiku-4-5", 2_000, Some(0.1)),
        session_usage("s", Some("/w"), "claude-opus-5", 3_000, Some(1.0)),
    ]);
    assert_eq!(sessions[0].models.len(), 2, "{:?}", sessions[0].models);
}

#[test]
fn usage_without_a_session_id_is_skipped_not_grouped_together() {
    // Journal and OpenCode rows carry no session. Bucketing them under one empty key would
    // invent a session that never existed and attribute unrelated work to it.
    let sessions = crate::ui::aggregate::session_totals(&[
        Usage {
            input: 100,
            ..Default::default()
        },
        session_usage("s", Some("/w"), "claude-opus-5", 1_000, Some(1.0)),
    ]);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "s");
}

#[test]
fn a_partly_priced_session_reports_its_unpriced_requests() {
    let sessions = crate::ui::aggregate::session_totals(&[
        session_usage("s", Some("/w"), "claude-opus-5", 1_000, Some(2.0)),
        session_usage("s", Some("/w"), "claude-opus-5", 2_000, None),
    ]);
    assert_eq!(sessions[0].unpriced_requests, 1);
    assert!((sessions[0].cost - 2.0).abs() < 1e-9);
}

#[test]
fn a_session_keeps_its_project_when_a_later_row_lacks_one() {
    let sessions = crate::ui::aggregate::session_totals(&[
        session_usage("s", Some("/w/api"), "claude-opus-5", 1_000, Some(1.0)),
        session_usage("s", None, "claude-opus-5", 2_000, Some(1.0)),
    ]);
    assert_eq!(sessions[0].project.as_deref(), Some("/w/api"));
}

#[test]
fn a_session_row_is_identifiable_without_its_uuid() {
    // The whole point of the panel: `0b9a76d5-b923-4b4f-8f20-51cea4534407` tells a reader
    // nothing, so the row has to carry when it ran, where, and on what instead.
    let base = crate::utils::now() - 90_000;
    let mut app = test_app(vec![
        session_usage(
            "0b9a76d5-b923-4b4f",
            Some("/home/x/Projects/ai-usage-tui"),
            "claude-opus-5",
            base,
            Some(4.10),
        ),
        session_usage(
            "0b9a76d5-b923-4b4f",
            Some("/home/x/Projects/ai-usage-tui"),
            "claude-opus-5",
            base + 6300,
            Some(8.30),
        ),
    ]);
    app.recompute();

    let rendered = render_sessions(&app, 84, 6);
    assert!(rendered.contains("ai-usage-tui"), "no project:\n{rendered}");
    assert!(rendered.contains("claude-opus-5"), "no model:\n{rendered}");
    assert!(rendered.contains("$12.40"), "no cost:\n{rendered}");
    assert!(rendered.contains("1h 45m"), "no duration:\n{rendered}");
    assert!(
        !rendered.contains("0b9a76d5"),
        "the raw uuid was rendered:\n{rendered}"
    );
}

#[test]
fn a_session_using_several_models_says_how_many() {
    let base = crate::utils::now() - 5_000;
    let mut app = test_app(vec![
        session_usage("s", Some("/w"), "claude-opus-5", base, Some(1.0)),
        session_usage("s", Some("/w"), "claude-haiku-4-5", base + 60, Some(0.1)),
    ]);
    app.recompute();
    assert!(render_sessions(&app, 84, 6).contains("2 models"));
}

#[test]
fn an_unpriced_session_says_so_rather_than_showing_zero() {
    let base = crate::utils::now() - 5_000;
    let mut app = test_app(vec![session_usage(
        "s",
        Some("/w"),
        "claude-sonnet-5",
        base,
        None,
    )]);
    app.recompute();
    let rendered = render_sessions(&app, 84, 6);
    assert!(rendered.contains("unpriced"), "{rendered}");
    assert!(
        !rendered.contains("$0.00"),
        "unknown cost rendered as zero:\n{rendered}"
    );
}

#[test]
fn no_sessions_explains_which_sources_provide_them() {
    let mut app = test_app(Vec::new());
    app.recompute();
    let rendered = render_sessions(&app, 84, 6);
    assert!(rendered.contains("No sessions"), "{rendered}");
    assert!(
        rendered.contains("Claude Code"),
        "should say where sessions come from:\n{rendered}"
    );
}

#[test]
fn a_session_of_only_quota_work_is_not_rendered_as_free() {
    let now = crate::utils::now();
    let mut app = test_app((0..3).map(|i| quota_usage(50_000, now - i)).collect());
    app.recompute();

    let rendered = render_sessions(&app, 100, 8);
    assert!(rendered.contains("quota"), "{rendered}");
    assert!(!rendered.contains("$0.00"), "{rendered}");
}
