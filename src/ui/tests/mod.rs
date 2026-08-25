//! Tests for the dashboard's state and aggregation.

#![cfg(test)]

use super::*;
use crate::budget::BudgetEngine;
use crate::escalation::{Escalations, Transition};
use crate::model::{Category, Usage};
use crate::ui::app::DerivedView;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::model::{CostStatus, Range};
use crate::utils::now;
// Named explicitly rather than left to `use super::*`: a name a glob import brought in is not
// re-exported by a second glob, so the submodules below would not see these.
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

// One file per area. This was a single 1837-line module: well named throughout, but the only
// home for the projects, time-series, burn, sessions, routing, breakdown and limits panels plus
// the SVG renderer, with nothing but reading order separating them.
//
// The fixtures below are shared by all of them; each submodule picks them up with `use super::*`.
mod app_state;
mod breakdown;
mod budgets;
mod burn;
mod coverage;
mod keys;
mod limits;
mod projects;
mod routing;
mod sessions;
mod svg;
mod timeseries;

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

/// Usage billed against a plan quota: real cost, no per-token rate, so `cost` stays `None` and
/// the rollups that carry it report zero dollars over a non-zero `quota_requests`.
fn quota_project_usage(project: Option<&str>, session: Option<&str>, tokens: u64) -> Usage {
    Usage {
        cost_status: CostStatus::Quota,
        cost: None,
        ..usage(project, session, None, tokens)
    }
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
        refresh_interval: Duration::from_secs(30),
        refreshed_at: Instant::now(),
        roots: crate::collector::SourceRoots {
            // Never the machine's real records: the tests below plant their own.
            omarchy_dir: Some(PathBuf::from("/nonexistent/omarchy")),
            ..crate::collector::SourceRoots::new(PathBuf::from("/tmp/unused-journal.db"))
        },
        limits_absence_logged: false,
        provider_filter: None,
        model_filter: None,
        collector: None,
        panel: Panel::Models,
        drilldown: None,
        search: Default::default(),
        sorts: Default::default(),
        show_help: false,
        budget_engine: BudgetEngine::empty(),
        // Bundled, not loaded: a refreshed cache on the developer's machine must not change
        // how a test ranks two models.
        pricing: crate::pricing::PricingEngine::bundled(),
        alerts: Vec::new(),
        alert_sink: None,
        view: DerivedView::default(),
    }
}

/// A usage row with an explicit creation timestamp.
fn usage_created_at(created: i64, tokens: u64, cost: Option<f64>) -> Usage {
    Usage {
        provider: "anthropic".into(),
        model: "claude-opus-5".into(),
        category: Category::Paid,
        cost_status: if cost.is_some() {
            CostStatus::Calculated
        } else {
            CostStatus::Unavailable
        },
        requests: 1,
        input: tokens,
        cost,
        created,
        ..Default::default()
    }
}

/// A usage row dated at local noon on `day`, for the daily aggregation.
fn usage_at(day: &str, tokens: u64, cost: Option<f64>) -> Usage {
    use chrono::TimeZone;
    let date = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("valid date");
    let noon = date.and_hms_opt(12, 0, 0).expect("valid time");
    let created = chrono::Local
        .from_local_datetime(&noon)
        .single()
        .expect("unambiguous local time")
        .timestamp();
    usage_created_at(created, tokens, cost)
}

/// A usage row `secs_ago` seconds before `now`, for the burn-rate window.
fn burn_usage(secs_ago: i64, now: i64, tokens: u64, cost: Option<f64>) -> Usage {
    usage_created_at(now - secs_ago, tokens, cost)
}

/// Render the burn panel and return its text, for the assertions below.
fn render_timeseries(app: &App, w: u16, h: u16) -> String {
    render_panel(w, h, |frame, area| {
        crate::ui::panels::timeseries::draw_timeseries(frame, area, app)
    })
}

fn render_projects(app: &App, w: u16, h: u16) -> String {
    render_panel(w, h, |frame, area| {
        crate::ui::panels::projects::draw_projects(frame, area, app)
    })
}

fn render_panel(
    w: u16,
    h: u16,
    draw: impl FnOnce(&mut ratatui::Frame, ratatui::layout::Rect),
) -> String {
    use ratatui::{backend::TestBackend, Terminal};
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend");
    terminal
        .draw(|frame| draw(frame, frame.area()))
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

/// Render a panel and keep the buffer, for assertions on style rather than text.
fn render_panel_buffer(
    w: u16,
    h: u16,
    draw: impl FnOnce(&mut ratatui::Frame, ratatui::layout::Rect),
) -> Buffer {
    use ratatui::{backend::TestBackend, Terminal};
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend");
    terminal
        .draw(|frame| draw(frame, frame.area()))
        .expect("draw");
    terminal.backend().buffer().clone()
}

/// The text of one buffer row, and whether its first inner cell carries the selection
/// highlight the tables share.
fn buffer_row(buffer: &Buffer, y: u16) -> (String, bool) {
    let text: String = (0..buffer.area.width)
        .map(|x| buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
        .collect();
    let highlighted = buffer
        .cell((1, y))
        .is_some_and(|c| c.style().bg == Some(Color::Rgb(37, 57, 67)));
    (text, highlighted)
}

/// The row of the buffer whose text contains `needle`, or a panic naming what was rendered.
fn find_row(buffer: &Buffer, needle: &str) -> (String, bool) {
    for y in 0..buffer.area.height {
        let row = buffer_row(buffer, y);
        if row.0.contains(needle) {
            return row;
        }
    }
    let all: Vec<String> = (0..buffer.area.height)
        .map(|y| buffer_row(buffer, y).0)
        .collect();
    panic!("no row contains {needle:?}:\n{}", all.join("\n"));
}

fn render_burn(app: &App, w: u16, h: u16) -> String {
    use ratatui::{backend::TestBackend, Terminal};
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend");
    terminal
        .draw(|frame| crate::ui::panels::burn::draw_burn(frame, frame.area(), app))
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn session_usage(
    id: &str,
    project: Option<&str>,
    model: &str,
    created: i64,
    cost: Option<f64>,
) -> Usage {
    Usage {
        provider: "anthropic".into(),
        model: model.into(),
        category: Category::Paid,
        cost_status: if cost.is_some() {
            CostStatus::Calculated
        } else {
            CostStatus::Unavailable
        },
        requests: 1,
        input: 1000,
        cost,
        created,
        session_id: Some(id.into()),
        project: project.map(str::to_string),
        ..Default::default()
    }
}

fn render_sessions(app: &App, w: u16, h: u16) -> String {
    use ratatui::{backend::TestBackend, Terminal};
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend");
    terminal
        .draw(|frame| crate::ui::panels::sessions::draw_sessions(frame, frame.area(), app))
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn routing_agg(
    agent: &str,
    model: &str,
    tasks: u64,
    cost: f64,
    passes: u32,
    failures: u32,
) -> crate::model::RoutingAggregates {
    crate::model::RoutingAggregates {
        agent: agent.into(),
        model: model.into(),
        provider: "p".into(),
        tasks,
        tokens: 100_000,
        cost,
        // `cost` is a floor now, read alongside these counters, so an aggregate built without
        // them is one whose every task was free -- which is what a zero cost here means, and
        // what a non-zero cost here must not mean.
        priced_tasks: if cost > 0.0 { tasks } else { 0 },
        free_tasks: if cost > 0.0 { 0 } else { tasks },
        test_passes: passes,
        test_failures: failures,
        ..Default::default()
    }
}

/// An aggregate whose spend is partly or wholly unaccountable: `unpriced` tasks with no rate and
/// `quota` tasks billed against a plan, on top of whatever `cost` was actually priced.
#[allow(clippy::too_many_arguments)]
fn routing_agg_with_gaps(
    agent: &str,
    model: &str,
    tasks: u64,
    cost: f64,
    passes: u32,
    failures: u32,
    unpriced: u64,
    quota: u64,
) -> crate::model::RoutingAggregates {
    crate::model::RoutingAggregates {
        unpriced_tasks: unpriced,
        quota_tasks: quota,
        ..routing_agg(agent, model, tasks, cost, passes, failures)
    }
}

fn render_routing(app: &App, w: u16, h: u16) -> String {
    use ratatui::{backend::TestBackend, Terminal};
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend");
    terminal
        .draw(|frame| crate::ui::panels::routing::draw_routing(frame, frame.area(), app))
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn escalations_for_test(
    examined: u64,
    escalated: u64,
    transitions: Vec<Transition>,
) -> Escalations {
    Escalations {
        sessions_examined: examined,
        sessions_escalated: escalated,
        unclassified_changes: 0,
        transitions,
    }
}

fn transition(
    from: &str,
    to: &str,
    sessions: u64,
    cost_after: f64,
    unpriced_after: u64,
) -> Transition {
    Transition {
        from: from.to_string(),
        to: to.to_string(),
        sessions,
        cost_after,
        unpriced_after,
        quota_after: 0,
    }
}

/// A cloud row as the collectors produce it — unpriced and `Category::Cloud` — normalised by the
/// real pricing pass rather than by hand-setting the enum, so these tests exercise the seam.
fn quota_usage(tokens: u64, created: i64) -> Usage {
    let mut usage = Usage {
        provider: "ollama-cloud".into(),
        model: "glm-5.2:cloud".into(),
        category: Category::Cloud,
        cost_status: CostStatus::Unavailable,
        requests: 1,
        input: tokens,
        cost: None,
        created,
        session_id: Some("cloud-session".into()),
        project: Some("/w/cloud".into()),
        ..Default::default()
    };
    let engine = crate::pricing::PricingEngine::bundled();
    crate::pricing::apply_estimated_pricing(std::slice::from_mut(&mut usage), &engine);
    assert_eq!(
        usage.cost_status,
        CostStatus::Quota,
        "fixture must go through the real normalisation"
    );
    usage
}

/// A buffer with one line of text at a known column, styled as given.
fn styled_buffer(text: &str, at: u16, style: Style) -> Buffer {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 3));
    buffer.set_string(at, 1, text, style);
    buffer
}

/// A Claude Code row on a Pro/Max plan, as the collector stamps it, normalised by the real
/// pricing pass so these tests exercise the seam rather than a hand-set enum.
fn subscription_usage(tokens: u64, created: i64) -> Usage {
    let mut usage = Usage {
        provider: "anthropic".into(),
        model: "claude-sonnet-4-5-20250929".into(),
        category: Category::Paid,
        cost_status: CostStatus::Unavailable,
        billing: crate::model::Billing::Subscription,
        requests: 1,
        input: tokens,
        output: tokens / 4,
        cost: None,
        created,
        session_id: Some("plan-session".into()),
        project: Some("/w/plan".into()),
        ..Default::default()
    };
    let engine = crate::pricing::PricingEngine::bundled();
    crate::pricing::apply_estimated_pricing(std::slice::from_mut(&mut usage), &engine);
    assert_eq!(
        usage.cost_status,
        CostStatus::Quota,
        "fixture must go through pricing"
    );
    assert!(usage.api_equivalent_cost.is_some_and(|c| c > 0.0));
    usage
}

fn render_breakdown(app: &App, w: u16, h: u16) -> String {
    render_panel(w, h, |frame, area| {
        crate::ui::panels::breakdown::draw_breakdown(frame, area, app)
    })
}

fn render_metrics(app: &App, w: u16, h: u16) -> String {
    render_panel(w, h, |frame, area| {
        crate::ui::panels::metrics::draw_metrics(frame, area, app)
    })
}

/// The fixture records, evaluated ten minutes after they were written.
fn fixture_limits(stale: bool) -> crate::omarchy::LimitsReport {
    let dir = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/omarchy"
    ));
    let now = 1_787_479_800 + if stale { 3 * 3600 } else { 0 };
    crate::omarchy::load_limits(&dir, now, crate::omarchy::STALE_AFTER_SECS)
}

fn render_limits(app: &App, w: u16, h: u16) -> String {
    render_panel(w, h, |frame, area| {
        crate::ui::panels::limits::draw_limits(frame, area, app)
    })
}

/// The foreground colour of the cell holding the first character of `needle`, or `None` when
/// the text is not on screen. Used where the colour *is* the assertion.
fn colour_of(
    w: u16,
    h: u16,
    draw: impl FnOnce(&mut ratatui::Frame, ratatui::layout::Rect),
    needle: &str,
) -> Option<ratatui::style::Color> {
    use ratatui::{backend::TestBackend, Terminal};
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend");
    terminal
        .draw(|frame| draw(frame, frame.area()))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
    let index = text.find(needle)?;
    let column = text[..index].chars().count();
    buffer.content().get(column).and_then(|cell| cell.fg.into())
}
