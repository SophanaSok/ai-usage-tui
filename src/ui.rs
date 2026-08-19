use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, TableState},
    Frame, Terminal,
};

use crate::budget::{Alert, AlertDispatcher, AlertLevel, BudgetEngine};
use crate::cli::Cli;
use crate::collector::background::CollectorHandle;
use crate::model::{
    Category, CostStatus, ProjectTotals, Range, RoutingAggregates, Totals, Usage, CYAN, RED, YELLOW,
};
use crate::utils::{format_clock, format_count, journal_path};

const MUTED: Color = Color::Rgb(125, 145, 160);
const PANEL: Color = Color::Rgb(18, 28, 37);

pub struct App {
    pub range: Range,
    pub usages: Vec<Usage>,
    pub selected: usize,
    pub status: String,
    /// Whether a collector is failing, restarting, dead, or stale. A monitor that goes quiet
    /// looks exactly like a monitor with nothing to report, so this is rendered, not logged.
    pub degraded: bool,
    pub last_refresh: String,
    pub pulse: u64,
    pub refresh_interval: Duration,
    pub refreshed_at: Instant,
    pub db_path: Option<PathBuf>,
    pub journal_path: PathBuf,
    pub claude_dir: Option<PathBuf>,
    pub provider_filter: Option<String>,
    pub model_filter: Option<String>,
    pub collector: Option<CollectorHandle>,
    /// Which view occupies the right-hand pane. This was two independent booleans, so
    /// "budgets on" and "routing on" could both be true and one silently won.
    pub panel: Panel,
    pub budget_engine: BudgetEngine,
    pub alerts: Vec<Alert>,
    /// Alerts are handed to a worker thread; the webhook POST is blocking and must never
    /// happen on the render path.
    alert_sink: Option<Sender<Vec<Alert>>>,
    /// Derived views, recomputed only when the data or the filters change.
    ///
    /// These were previously rebuilt inside `draw`, which cloned every `Usage` roughly eight
    /// times per frame at 4fps and re-read the routing table from SQLite on the render thread.
    view: DerivedView,
}

/// The right-hand pane's contents. Exactly one at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    #[default]
    Models,
    Budgets,
    Routing,
    Projects,
}

#[derive(Default)]
pub struct DerivedView {
    filtered: Vec<Usage>,
    rows: Vec<Usage>,
    totals: Totals,
    category_totals: Vec<(Category, Totals)>,
    routing: Vec<RoutingAggregates>,
    projects: Vec<ProjectTotals>,
    coverage: Coverage,
}

/// How much of the visible usage the pricing engine could actually price.
///
/// Provenance is the project's differentiator but it lived entirely in an internal enum: a
/// user could read a total without knowing it covered two thirds of their requests.
#[derive(Clone, Copy, Debug, Default)]
pub struct Coverage {
    pub priced_requests: u64,
    pub billable_requests: u64,
}

impl Coverage {
    /// `None` when nothing billable is in range — 100% of nothing is not a useful claim.
    pub fn pct(&self) -> Option<f64> {
        if self.billable_requests == 0 {
            return None;
        }
        Some((self.priced_requests as f64 / self.billable_requests as f64) * 100.0)
    }
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db_path: Option<PathBuf>,
        journal_path: PathBuf,
        claude_dir: Option<PathBuf>,
        range: Range,
        refresh_interval: Duration,
        provider_filter: Option<String>,
        model_filter: Option<String>,
        collector: Option<CollectorHandle>,
        budget_engine: BudgetEngine,
        alert_sink: Option<Sender<Vec<Alert>>>,
    ) -> Self {
        let mut app = Self {
            range,
            usages: Vec::new(),
            selected: 0,
            status: String::new(),
            degraded: false,
            last_refresh: String::from("never"),
            pulse: 0,
            refresh_interval,
            refreshed_at: Instant::now(),
            db_path,
            journal_path,
            claude_dir,
            provider_filter,
            model_filter,
            collector,
            panel: Panel::Models,
            budget_engine,
            alerts: Vec::new(),
            alert_sink,
            view: DerivedView::default(),
        };
        app.refresh();
        app
    }

    /// Rebuild every derived view from `usages`. Call after data or filters change.
    pub fn recompute(&mut self) {
        let cutoff = self.range.cutoff();
        let is_all = self.range == Range::All;
        let provider = self.provider_filter.as_deref();
        let model = self.model_filter.as_deref();

        self.view.filtered = self
            .usages
            .iter()
            .filter(|u| is_all || u.created >= cutoff)
            .filter(|u| provider.is_none_or(|p| u.provider.eq_ignore_ascii_case(p)))
            .filter(|u| model.is_none_or(|m| u.model.eq_ignore_ascii_case(m)))
            .cloned()
            .collect();

        self.view.totals = self
            .view
            .filtered
            .iter()
            .fold(Totals::default(), |mut t, u| {
                t.add(u);
                t
            });

        self.view.category_totals = [
            Category::Local,
            Category::Free,
            Category::Paid,
            Category::Cloud,
            Category::Unknown,
        ]
        .into_iter()
        .map(|category| {
            let totals = self
                .view
                .filtered
                .iter()
                .filter(|u| u.category == category)
                .fold(Totals::default(), |mut t, u| {
                    t.add(u);
                    t
                });
            (category, totals)
        })
        .collect();

        self.view.projects = project_totals(&self.view.filtered);
        self.view.coverage = coverage(&self.view.filtered);

        let mut grouped = BTreeMap::<(String, String, Category, CostStatus), Usage>::new();
        for u in &self.view.filtered {
            let key = (
                u.provider.clone(),
                u.model.clone(),
                u.category,
                u.cost_status,
            );
            let entry = grouped.entry(key).or_insert_with(|| Usage {
                provider: u.provider.clone(),
                model: u.model.clone(),
                category: u.category,
                cost_status: u.cost_status,
                ..Default::default()
            });
            entry.requests += u.requests;
            entry.input += u.input;
            entry.output += u.output;
            entry.reasoning += u.reasoning;
            entry.cache_read += u.cache_read;
            entry.cache_write += u.cache_write;
            if u.cost_status.is_billable() {
                if let Some(cost) = u.cost {
                    entry.cost = Some(entry.cost.unwrap_or(0.0) + cost);
                }
            }
        }
        self.view.rows = grouped.into_values().collect();
        self.view.rows.sort_by_key(|u| Reverse(u.total_tokens()));

        self.selected = self.selected.min(self.view.rows.len().saturating_sub(1));
    }

    pub fn projects(&self) -> &[ProjectTotals] {
        &self.view.projects
    }

    pub fn coverage(&self) -> Coverage {
        self.view.coverage
    }

    /// Toggle a panel on, or back to the model list if it is already showing.
    pub fn toggle_panel(&mut self, panel: Panel) {
        self.panel = if self.panel == panel {
            Panel::Models
        } else {
            panel
        };
    }

    /// Change the visible range and rebuild derived views.
    pub fn set_range(&mut self, range: Range) {
        self.range = range;
        self.recompute();
    }
    pub fn refresh(&mut self) {
        if let Some(ref collector) = self.collector {
            self.usages = collector.snapshot();
            self.status = collector.status();
            self.degraded = collector.is_degraded();
        } else {
            match crate::collector::load_usage(
                self.db_path.as_deref(),
                &self.journal_path,
                self.claude_dir.as_deref(),
            ) {
                Ok((usages, source)) => {
                    self.usages = usages;
                    self.status = source;
                    self.degraded = false;
                }
                Err(error) => {
                    self.usages.clear();
                    self.status = format!("OpenCode unavailable: {}", error);
                    self.degraded = true;
                }
            }
        }
        self.last_refresh = format_clock();
        self.refreshed_at = Instant::now();
        self.recompute();
        // Routing lives in SQLite; read it here, not inside `draw`.
        self.view.routing = match crate::collector::journal::load_routing(&self.journal_path) {
            Ok(events) => crate::routing::aggregate(&events),
            Err(error) => {
                // An empty routing panel used to be the rendering for both "no events yet"
                // and "the journal could not be read". Only one of those is fine.
                crate::logging::error("routing", &format!("journal read failed: {}", error));
                self.status = format!("{} | routing unavailable: {}", self.status, error);
                self.degraded = true;
                Vec::new()
            }
        };
        if !self.budget_engine.is_empty() {
            self.alerts = self.budget_engine.check(&self.usages);
            if let Some(sink) = &self.alert_sink {
                if self.alerts.iter().any(Alert::is_actionable) {
                    // Best-effort: a dead worker must not take the dashboard down with it.
                    let _ = sink.send(self.alerts.clone());
                }
            }
        }
    }
    pub fn refresh_if_due(&mut self) {
        if self.refreshed_at.elapsed() >= self.refresh_interval {
            self.refresh();
        }
    }
    pub fn filtered(&self) -> &[Usage] {
        &self.view.filtered
    }
    pub fn rows(&self) -> &[Usage] {
        &self.view.rows
    }
    pub fn totals(&self) -> &Totals {
        &self.view.totals
    }
    pub fn category_totals(&self) -> &[(Category, Totals)] {
        &self.view.category_totals
    }
    pub fn routing(&self) -> &[RoutingAggregates] {
        &self.view.routing
    }
}

pub fn cost_display(usage: &Usage) -> String {
    match usage.cost_status {
        CostStatus::Local => "LOCAL".into(),
        CostStatus::Free => "FREE".into(),
        CostStatus::ProviderReported => usage
            .cost
            .map(|cost| format!("${:.4} reported", cost))
            .unwrap_or_else(|| "REPORTED / NO COST".into()),
        CostStatus::Calculated => usage
            .cost
            .map(|cost| format!("${:.4} calculated", cost))
            .unwrap_or_else(|| "CALCULATED / NO COST".into()),
        CostStatus::Estimated => usage
            .cost
            .map(|cost| format!("${:.4} estimated", cost))
            .unwrap_or_else(|| "ESTIMATED / NO COST".into()),
        CostStatus::Unavailable => "UNKNOWN COST".into(),
    }
}

pub fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cli: &Cli,
    collector: Option<CollectorHandle>,
    budget_engine: BudgetEngine,
    mut dispatcher: AlertDispatcher,
) -> Result<()> {
    let journal = cli
        .journal_path
        .clone()
        .or_else(journal_path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not determine a home directory; pass an explicit path (see --help)"
            )
        })?;
    // The dispatcher owns a blocking HTTP client; give it its own thread and talk to it over
    // a channel. Dropping the sender when `run` returns ends the worker.
    let alert_sink = dispatcher.webhook_url.is_some().then(|| {
        let (tx, rx) = mpsc::channel::<Vec<Alert>>();
        std::thread::spawn(move || {
            while let Ok(alerts) = rx.recv() {
                let _ = dispatcher.dispatch(&alerts);
            }
        });
        tx
    });

    let mut app = App::new(
        cli.db_path.clone(),
        journal,
        cli.claude_dir.clone(),
        cli.range,
        cli.refresh_interval,
        cli.provider_filter.clone(),
        cli.model_filter.clone(),
        collector,
        budget_engine,
        alert_sink,
    );
    loop {
        app.refresh_if_due();
        terminal.draw(|frame| draw(frame, &app))?;
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('r') => app.refresh(),
                    KeyCode::Char('b') => app.toggle_panel(Panel::Budgets),
                    KeyCode::Char('t') => app.toggle_panel(Panel::Routing),
                    KeyCode::Char('p') => app.toggle_panel(Panel::Projects),
                    KeyCode::Char('1') => app.set_range(Range::Today),
                    KeyCode::Char('2') => app.set_range(Range::Week),
                    KeyCode::Char('3') => app.set_range(Range::Month),
                    KeyCode::Char('4') => app.set_range(Range::All),
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.selected = (app.selected + 1).min(app.rows().len().saturating_sub(1))
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.selected = app.selected.saturating_sub(1)
                    }
                    _ => {}
                }
            }
        }
        app.pulse = app.pulse.wrapping_add(1);
    }
    Ok(())
}

/// Shorten project paths to the fewest trailing segments that still tell them apart.
///
/// `project` holds the whole working directory so that `~/a/build` and `~/b/build` stay
/// distinct rows. Rendering the whole path would be unreadable and would put the user's home
/// directory on screen, so each label is trimmed to its last segment and lengthened only where
/// that would collide with another visible project.
pub fn project_labels(paths: &[String]) -> Vec<String> {
    fn segments(path: &str) -> Vec<&str> {
        path.split(['/', '\\']).filter(|s| !s.is_empty()).collect()
    }

    let split: Vec<Vec<&str>> = paths.iter().map(|p| segments(p)).collect();
    let deepest = split.iter().map(Vec::len).max().unwrap_or(0);

    let mut labels: Vec<String> = Vec::with_capacity(paths.len());
    for (index, parts) in split.iter().enumerate() {
        if parts.is_empty() {
            labels.push(paths[index].clone());
            continue;
        }
        let mut take = 1;
        while take < parts.len().min(deepest) {
            let candidate = &parts[parts.len() - take..];
            let collides = split.iter().enumerate().any(|(other, other_parts)| {
                other != index
                    && other_parts.len() >= take
                    && &other_parts[other_parts.len() - take..] == candidate
            });
            if !collides {
                break;
            }
            take += 1;
        }
        labels.push(parts[parts.len() - take..].join("/"));
    }
    labels
}

/// Roll usage up by project.
///
/// `project` and `session_id` have been populated by the Claude Code collector since it
/// landed, and nothing rendered them. Sorted by cost, then tokens: the question this view
/// answers is "where is the money going", and a project can burn tokens cheaply.
pub fn project_totals(usages: &[Usage]) -> Vec<ProjectTotals> {
    use std::collections::{BTreeMap, HashSet};

    struct Acc {
        totals: ProjectTotals,
        sessions: HashSet<String>,
        models: HashSet<String>,
    }

    let mut grouped: BTreeMap<String, Acc> = BTreeMap::new();
    for usage in usages {
        // Usage from a source that records no project still has to be accounted for
        // somewhere, or the per-project totals silently disagree with the headline total.
        let name = usage
            .project
            .clone()
            .unwrap_or_else(|| "(unattributed)".to_string());
        let acc = grouped.entry(name.clone()).or_insert_with(|| Acc {
            totals: ProjectTotals {
                project: name,
                ..Default::default()
            },
            sessions: HashSet::new(),
            models: HashSet::new(),
        });
        acc.totals.requests += usage.requests;
        acc.totals.tokens += usage.total_tokens();
        if usage.cost_status.needs_price() {
            match usage.cost.filter(|_| usage.cost_status.is_billable()) {
                Some(cost) => acc.totals.cost += cost,
                None => acc.totals.unpriced_requests += usage.requests,
            }
        }
        if let Some(session) = &usage.session_id {
            acc.sessions.insert(session.clone());
        }
        acc.models
            .insert(format!("{}/{}", usage.provider, usage.model));
    }

    let mut rows: Vec<ProjectTotals> = grouped
        .into_values()
        .map(|acc| ProjectTotals {
            sessions: acc.sessions.len(),
            models: acc.models.len(),
            ..acc.totals
        })
        .collect();
    rows.sort_by(|a, b| {
        b.cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.tokens.cmp(&a.tokens))
    });
    rows
}

/// What share of billable requests carry a known cost.
pub fn coverage(usages: &[Usage]) -> Coverage {
    let mut coverage = Coverage::default();
    for usage in usages {
        if !usage.cost_status.needs_price() {
            continue;
        }
        coverage.billable_requests += usage.requests;
        if usage.cost.is_some() && usage.cost_status.is_billable() {
            coverage.priced_requests += usage.requests;
        }
    }
    coverage
}

fn panel<'a>(title: &'a str, color: Color) -> Block<'a> {
    Block::default()
        .title(Span::styled(
            format!(" {} ", title),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(48, 72, 84)))
        .style(Style::default().bg(PANEL))
}

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let alert_banner_height = if app.alerts.iter().any(|a| a.is_actionable()) {
        1u16
    } else {
        0u16
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(alert_banner_height),
            Constraint::Length(7),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);
    draw_header(frame, chunks[0], app);
    if alert_banner_height > 0 {
        draw_alert_banner(frame, chunks[1], app);
    }
    draw_metrics(frame, chunks[2], app);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
        .split(chunks[3]);
    draw_breakdown(frame, body[0], app);
    match app.panel {
        Panel::Routing => draw_routing(frame, body[1], app),
        Panel::Budgets => draw_budgets(frame, body[1], app),
        Panel::Projects => draw_projects(frame, body[1], app),
        Panel::Models => draw_models(frame, body[1], app),
    }
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            " 1-4 ",
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ),
        Span::raw("range  "),
        Span::styled("r", Style::default().fg(CYAN).add_modifier(Modifier::BOLD)),
        Span::raw(" refresh  "),
        Span::styled("b", Style::default().fg(CYAN).add_modifier(Modifier::BOLD)),
        Span::raw(" budgets  "),
        Span::styled("t", Style::default().fg(CYAN).add_modifier(Modifier::BOLD)),
        Span::raw(" routing  "),
        Span::styled("p", Style::default().fg(CYAN).add_modifier(Modifier::BOLD)),
        Span::raw(" projects  "),
        Span::styled(
            "j/k",
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" navigate  "),
        Span::styled("q", Style::default().fg(CYAN).add_modifier(Modifier::BOLD)),
        Span::raw(" quit"),
    ]))
    .style(Style::default().fg(MUTED));
    frame.render_widget(footer, chunks[4]);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " AI USAGE ",
            Style::default()
                .fg(Color::Black)
                .bg(CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            "LIVE PROVIDER MONITOR",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("                                      "),
        Span::styled(
            format!(
                "{}  {}  {} ",
                app.range.label(),
                app.last_refresh,
                app.status
            ),
            if app.degraded {
                Style::default().fg(RED).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            },
        ),
    ]))
    .style(Style::default().bg(Color::Rgb(10, 18, 24)));
    frame.render_widget(title, area);
}

fn draw_metrics(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.totals();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(24),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(16),
        ])
        .split(area);
    let total = metric(
        "TOTAL TOKENS",
        format_count(t.tokens()),
        CYAN,
        format!("{} requests", t.requests),
    );
    frame.render_widget(total, cols[0]);
    for (i, (category, cat)) in app.category_totals().iter().enumerate() {
        let subtitle = if *category == Category::Paid {
            format!("${:.4}", cat.cost)
        } else {
            format!("{} tokens", format_count(cat.tokens()))
        };
        frame.render_widget(
            metric(
                category.label(),
                format_count(cat.tokens()),
                category.color(),
                subtitle,
            ),
            cols[i + 1],
        );
    }
}

fn metric<'a>(label: &'a str, value: String, color: Color, subtitle: String) -> Paragraph<'a> {
    Paragraph::new(vec![
        Line::from(Span::styled(
            value,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(subtitle, Style::default().fg(MUTED))),
    ])
    .block(panel(label, color))
    .style(Style::default().fg(Color::White))
}

fn draw_breakdown(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.totals();
    let rows = vec![
        ListItem::new(Line::from(vec![
            Span::styled("INPUT       ", Style::default().fg(MUTED)),
            Span::styled(format_count(t.input), Style::default().fg(Color::White)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("OUTPUT      ", Style::default().fg(MUTED)),
            Span::styled(format_count(t.output), Style::default().fg(Color::White)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("REASONING   ", Style::default().fg(MUTED)),
            Span::styled(format_count(t.reasoning), Style::default().fg(Color::White)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("CACHE READ  ", Style::default().fg(MUTED)),
            Span::styled(
                format_count(t.cache_read),
                Style::default().fg(Color::White),
            ),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("CACHE WRITE ", Style::default().fg(MUTED)),
            Span::styled(
                format_count(t.cache_write),
                Style::default().fg(Color::White),
            ),
        ])),
        ListItem::new(Line::from("")),
        ListItem::new(Line::from(vec![
            Span::styled("EST. PAID COST ", Style::default().fg(YELLOW)),
            Span::styled(
                format!("${:.4}", t.cost),
                Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
            ),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled("PRICING STATUS  ", Style::default().fg(MUTED)),
            Span::raw(if t.unknown_requests == 0 {
                "complete"
            } else {
                "partial / unknown"
            }),
        ])),
    ];
    frame.render_widget(List::new(rows).block(panel("TOKEN FLOW", CYAN)), area);
}

fn draw_models(frame: &mut Frame, area: Rect, app: &App) {
    let rows = app.rows();
    let table_rows = rows.iter().enumerate().map(|(index, u)| {
        let style = if index == app.selected {
            Style::default().bg(Color::Rgb(37, 57, 67))
        } else {
            Style::default()
        };
        Row::new(vec![
            Cell::from(format!("{} / {}", u.provider, u.model)),
            Cell::from(u.category.label()),
            Cell::from(format_count(u.total_tokens())),
            Cell::from(cost_display(u)),
            Cell::from(u.requests.to_string()),
        ])
        .style(style)
    });
    let header = Row::new(vec!["PROVIDER / MODEL", "CLASS", "TOKENS", "COST", "REQS"])
        .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Min(24),
        Constraint::Length(9),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Length(7),
    ];
    // A plain `render_widget` has no viewport offset, so a selection below the fold simply
    // vanished. `TableState` scrolls the viewport to keep it visible.
    let mut state = TableState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(
        Table::new(table_rows, widths)
            .header(header)
            .column_spacing(1)
            .block(panel("MODEL ACTIVITY", CYAN)),
        area,
        &mut state,
    );
}

fn draw_projects(frame: &mut Frame, area: Rect, app: &App) {
    let projects = app.projects();
    let paths: Vec<String> = projects.iter().map(|p| p.project.clone()).collect();
    let labels = project_labels(&paths);
    let rows = projects.iter().zip(labels).map(|(p, label)| {
        // A project with unpriced requests gets its cost shown as a floor, not a total. The
        // never-render-unknown-cost-as-zero invariant applies to partial sums too.
        let cost = if p.unpriced_requests > 0 {
            Span::styled(format!("≥ ${:.2}", p.cost), Style::default().fg(YELLOW))
        } else {
            Span::raw(format!("${:.2}", p.cost))
        };
        Row::new(vec![
            Cell::from(label),
            Cell::from(format_count(p.tokens)),
            Cell::from(cost),
            Cell::from(p.requests.to_string()),
            Cell::from(p.sessions.to_string()),
        ])
    });
    let header = Row::new(vec!["PROJECT", "TOKENS", "COST", "REQS", "SESS"])
        .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Min(24),
        Constraint::Length(11),
        Constraint::Length(12),
        Constraint::Length(7),
        Constraint::Length(6),
    ];
    let title = match app.coverage().pct() {
        Some(pct) => format!("PROJECT COST  ({:.0}% priced)", pct),
        None => "PROJECT COST".to_string(),
    };
    frame.render_widget(
        Table::new(rows, widths)
            .header(header)
            .column_spacing(1)
            .block(panel(&title, CYAN)),
        area,
    );
}

fn draw_alert_banner(frame: &mut Frame, area: Rect, app: &App) {
    let actionable: Vec<&Alert> = app.alerts.iter().filter(|a| a.is_actionable()).collect();
    if actionable.is_empty() {
        return;
    }
    let spans: Vec<Span> = actionable
        .iter()
        .flat_map(|alert| {
            let color = match alert.level {
                AlertLevel::Warn => YELLOW,
                AlertLevel::Critical | AlertLevel::Exceeded => RED,
                AlertLevel::Ok => MUTED,
            };
            let period_str = match alert.period {
                crate::budget::BudgetPeriod::Daily => "daily",
                crate::budget::BudgetPeriod::Monthly => "monthly",
            };
            vec![
                Span::styled(
                    format!(" {} ", alert.level.label()),
                    Style::default()
                        .fg(Color::Black)
                        .bg(color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "{} {}/${:.2}/${:.2} ({}%)  ",
                        alert.scope.label(),
                        period_str,
                        alert.spend,
                        alert.limit,
                        alert.pct as u64
                    ),
                    Style::default().fg(color),
                ),
            ]
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(10, 18, 24))),
        area,
    );
}

fn draw_budgets(frame: &mut Frame, area: Rect, app: &App) {
    let budgets = app.budget_engine.budgets();
    if budgets.is_empty() {
        frame.render_widget(
            Paragraph::new("No budgets configured.\nAdd [[budgets.entry]] to your config.toml.")
                .style(Style::default().fg(MUTED))
                .block(panel("BUDGETS", CYAN)),
            area,
        );
        return;
    }
    let alerts_map: std::collections::HashMap<_, _> = app
        .alerts
        .iter()
        .map(|a| ((a.scope.clone(), a.period), a))
        .collect();
    let table_rows = budgets.iter().map(|budget| {
        let alert = alerts_map.get(&(budget.scope.clone(), budget.period));
        let (spend, pct, level_str, color) = if let Some(alert) = alert {
            let c = match alert.level {
                AlertLevel::Warn => YELLOW,
                AlertLevel::Critical | AlertLevel::Exceeded => RED,
                AlertLevel::Ok => MUTED,
            };
            (alert.spend, alert.pct, alert.level.label(), c)
        } else {
            (0.0, 0.0, "OK", MUTED)
        };
        Row::new(vec![
            Cell::from(budget.scope.label().to_string()),
            Cell::from(format!("{:?}", budget.period).to_lowercase()),
            Cell::from(format!("${:.2}", spend)),
            Cell::from(format!("${:.2}", budget.limit)),
            Cell::from(format!("{}%", pct as u64)),
            Cell::from(level_str).style(Style::default().fg(color)),
        ])
    });
    let header = Row::new(vec!["SCOPE", "PERIOD", "SPEND", "LIMIT", "PCT", "STATUS"])
        .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(10),
    ];
    frame.render_widget(
        Table::new(table_rows, widths)
            .header(header)
            .column_spacing(1)
            .block(panel("BUDGETS", CYAN)),
        area,
    );
}

fn draw_routing(frame: &mut Frame, area: Rect, app: &App) {
    let aggregates = app.routing();
    if aggregates.is_empty() {
        frame.render_widget(
            Paragraph::new("No routing events recorded.\nUse --record-routing to capture.")
                .style(Style::default().fg(MUTED))
                .block(panel("ROUTING", CYAN)),
            area,
        );
        return;
    }
    let table_rows = aggregates.iter().map(|agg| {
        Row::new(vec![
            Cell::from(agg.agent.clone()),
            Cell::from(agg.model.clone()),
            Cell::from(format_count(agg.tokens)),
            Cell::from(format!("${:.4}", agg.cost)),
            Cell::from(format!("{:.0}%", crate::routing::retry_rate(agg))),
            Cell::from(format!("{:.0}%", crate::routing::defect_rate(agg))),
            Cell::from(agg.tasks.to_string()),
        ])
    });
    let header = Row::new(vec![
        "AGENT", "MODEL", "TOKENS", "COST", "RETRY%", "DEFECTS", "TASKS",
    ])
    .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Min(18),
        Constraint::Min(20),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(7),
    ];
    frame.render_widget(
        Table::new(table_rows, widths)
            .header(header)
            .column_spacing(1)
            .block(panel("ROUTING", CYAN)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CostStatus, Range};
    use crate::utils::now;

    fn usage(
        project: Option<&str>,
        session: Option<&str>,
        cost: Option<f64>,
        tokens: u64,
    ) -> Usage {
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
}
