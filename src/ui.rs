use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table},
    Frame, Terminal,
};

use crate::cli::Cli;
use crate::collector::background::CollectorHandle;
use crate::budget::{Alert, AlertLevel, BudgetEngine};
use crate::model::{Category, CostStatus, Range, Totals, Usage, CYAN, YELLOW, RED};
use crate::utils::{format_clock, format_count, journal_path};

const MUTED: Color = Color::Rgb(125, 145, 160);
const PANEL: Color = Color::Rgb(18, 28, 37);

pub struct App {
    pub range: Range,
    pub usages: Vec<Usage>,
    pub selected: usize,
    pub status: String,
    pub last_refresh: String,
    pub pulse: u64,
    pub refresh_interval: Duration,
    pub refreshed_at: Instant,
    pub db_path: Option<PathBuf>,
    pub journal_path: PathBuf,
    pub provider_filter: Option<String>,
    pub model_filter: Option<String>,
    pub collector: Option<CollectorHandle>,
    pub show_budgets: bool,
    pub budget_engine: BudgetEngine,
    pub alerts: Vec<Alert>,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db_path: Option<PathBuf>,
        journal_path: PathBuf,
        range: Range,
        refresh_interval: Duration,
        provider_filter: Option<String>,
        model_filter: Option<String>,
        collector: Option<CollectorHandle>,
        budget_engine: BudgetEngine,
    ) -> Self {
        let mut app = Self {
            range,
            usages: Vec::new(),
            selected: 0,
            status: String::new(),
            last_refresh: String::from("never"),
            pulse: 0,
            refresh_interval,
            refreshed_at: Instant::now(),
            db_path,
            journal_path,
            provider_filter,
            model_filter,
            collector,
            show_budgets: false,
            budget_engine,
            alerts: Vec::new(),
        };
        app.refresh();
        app
    }
    pub fn refresh(&mut self) {
        if let Some(ref collector) = self.collector {
            self.usages = collector.snapshot();
            self.status = collector.status();
        } else {
            match crate::collector::load_usage(self.db_path.as_deref(), &self.journal_path) {
                Ok((usages, source)) => {
                    self.usages = usages;
                    self.status = source;
                }
                Err(error) => {
                    self.usages.clear();
                    self.status = format!("OpenCode unavailable: {}", error);
                }
            }
        }
        self.last_refresh = format_clock();
        self.refreshed_at = Instant::now();
        self.selected = self.selected.min(self.rows().len().saturating_sub(1));
        if !self.budget_engine.is_empty() {
            self.alerts = self.budget_engine.check(&self.usages);
        }
    }
    pub fn refresh_if_due(&mut self) {
        if self.refreshed_at.elapsed() >= self.refresh_interval {
            self.refresh();
        }
    }
    pub fn filtered(&self) -> Vec<Usage> {
        self.usages
            .iter()
            .filter(|u| u.created >= self.range.cutoff() || self.range == Range::All)
            .filter(|u| {
                self.provider_filter
                    .as_ref()
                    .map(|provider| u.provider.eq_ignore_ascii_case(provider))
                    .unwrap_or(true)
            })
            .filter(|u| {
                self.model_filter
                    .as_ref()
                    .map(|model| u.model.eq_ignore_ascii_case(model))
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }
    pub fn rows(&self) -> Vec<Usage> {
        let mut grouped = BTreeMap::<(String, String, Category, CostStatus), Usage>::new();
        for u in self.filtered() {
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
        let mut rows: Vec<_> = grouped.into_values().collect();
        rows.sort_by_key(|u| Reverse(u.total_tokens()));
        rows
    }
    pub fn totals(&self) -> Totals {
        self.filtered().iter().fold(Totals::default(), |mut t, u| {
            t.add(u);
            t
        })
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
) -> Result<()> {
    let journal = cli
        .journal_path
        .clone()
        .or_else(journal_path)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let mut app = App::new(
        cli.db_path.clone(),
        journal,
        cli.range,
        cli.refresh_interval,
        cli.provider_filter.clone(),
        cli.model_filter.clone(),
        collector,
        budget_engine,
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
                    KeyCode::Char('b') => app.show_budgets = !app.show_budgets,
                    KeyCode::Char('1') => app.range = Range::Today,
                    KeyCode::Char('2') => app.range = Range::Week,
                    KeyCode::Char('3') => app.range = Range::Month,
                    KeyCode::Char('4') => app.range = Range::All,
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
    if app.show_budgets {
        draw_budgets(frame, body[1], app);
    } else {
        draw_models(frame, body[1], app);
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
            Style::default().fg(MUTED),
        ),
    ]))
    .style(Style::default().bg(Color::Rgb(10, 18, 24)));
    frame.render_widget(title, area);
}

fn draw_metrics(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.totals();
    let categories = [
        Category::Local,
        Category::Free,
        Category::Paid,
        Category::Cloud,
        Category::Unknown,
    ];
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
    for (i, category) in categories.iter().enumerate() {
        let mut cat = Totals::default();
        for u in app.filtered().iter().filter(|u| u.category == *category) {
            cat.add(u);
        }
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
    frame.render_widget(
        Table::new(table_rows, widths)
            .header(header)
            .column_spacing(1)
            .block(panel("MODEL ACTIVITY", CYAN)),
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
                    Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} {}/${:.2}/${:.2} ({}%)  ", alert.scope.label(), period_str, alert.spend, alert.limit, alert.pct as u64),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CostStatus, Range};
    use crate::utils::now;

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
        let app = App {
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
            last_refresh: String::new(),
            pulse: 0,
            refresh_interval: Duration::from_secs(30),
            refreshed_at: Instant::now(),
            db_path: None,
            journal_path: PathBuf::from("/tmp/unused-journal.db"),
            provider_filter: None,
            model_filter: None,
            collector: None,
            show_budgets: false,
            budget_engine: BudgetEngine::empty(),
            alerts: Vec::new(),
        };
        assert_eq!(app.rows().len(), 2);
    }
}
