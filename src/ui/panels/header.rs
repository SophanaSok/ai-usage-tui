//! Top bar: range, last refresh, and collector health.
//!
//! Adding a panel: create a sibling module here, add a `Panel` variant in `app.rs`, a `Binding`
//! in `keys.rs` (with a footer `hint`), and a match arm in `draw`. See CONTRIBUTING.md.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::model::{CYAN, RED, YELLOW};
use crate::ui::app::App;
use crate::ui::theme::MUTED;
use crate::utils::format_count;

pub fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let title = Paragraph::new(header_line(app, area.width))
        .style(Style::default().bg(Color::Rgb(10, 18, 24)));
    frame.render_widget(title, area);
}

/// The header, with its subtitle dropped when the line would not otherwise fit.
///
/// `LIVE PROVIDER MONITOR` is decoration — the badge to its left already says what this is — and
/// it is the only thing here a reader loses nothing by. Everything after it is not decoration:
/// the collector status is last, and a `Paragraph` truncates in silence, so before the update
/// notice existed an 80-column terminal fitted the line exactly and afterwards it did not. A
/// monitor that has gone quiet looking exactly like a monitor with nothing to report is the one
/// failure this bar exists to prevent, so the subtitle yields to it. Measured, not thresholded,
/// for the reason the footer measures: a width written down here is a fact about the header's
/// own contents, and it goes stale the next time they change.
pub(crate) fn header_line<'a>(app: &App, width: u16) -> Line<'a> {
    let full = Line::from(spans(app, true));
    if full.width() <= usize::from(width) {
        return full;
    }
    Line::from(spans(app, false))
}

fn spans<'a>(app: &App, subtitle: bool) -> Vec<Span<'a>> {
    let mut spans = vec![Span::styled(
        " AI USAGE ",
        Style::default()
            .fg(Color::Black)
            .bg(CYAN)
            .add_modifier(Modifier::BOLD),
    )];
    if subtitle {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "LIVE PROVIDER MONITOR",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(update_span(app));
    spans.push(Span::raw("   "));
    spans.push(coverage_span(app));
    spans.push(limits_span(app));
    spans.push(Span::raw("   "));
    spans.push(Span::styled(
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
    ));
    spans
}

/// A newer release, when an opted-in check found one.
///
/// This is the only place a user who never runs `--doctor` learns that a release exists, which
/// is what it is for. It says a version and nothing else: what to *do* about it depends on how
/// this copy was installed, and `--doctor` is where that answer already lives — naming an
/// upgrade command here would mean guessing the channel from a header.
fn update_span<'a>(app: &App) -> Span<'a> {
    match &app.update_notice {
        Some(notice) => Span::styled(
            format!("  {notice}"),
            Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
        ),
        None => Span::raw(""),
    }
}

/// The fullest subscription window Omarchy reports, when one is fresh. The bar's own glyph
/// does this for Omarchy users; here it sits beside the cost figure the window constrains.
fn limits_span<'a>(app: &App) -> Span<'a> {
    match app.limits().binding_window() {
        Some((snapshot, window)) => {
            let text = format!(
                "   {} {} {:.0}%",
                snapshot.agent,
                window
                    .label
                    .to_lowercase()
                    .split(' ')
                    .next()
                    .unwrap_or("window"),
                window.percent_used()
            );
            if window.is_alarming() {
                Span::styled(text, Style::default().fg(RED).add_modifier(Modifier::BOLD))
            } else {
                Span::styled(text, Style::default().fg(MUTED))
            }
        }
        None => Span::raw(""),
    }
}

/// How much of the visible spend actually carries a known price.
///
/// Cost provenance is what this project does that the alternatives do not, and until now it
/// lived entirely in an internal enum and one panel's title — a reader could take a total at
/// face value without ever learning it covered two thirds of their requests. It belongs where
/// the total is.
///
/// Below 100% it is yellow, because that is the case worth noticing.
fn coverage_span<'a>(app: &App) -> Span<'a> {
    let coverage = app.coverage();
    // "all priced" while thousands of quota-billed requests sit outside the ratio is technically
    // true and unhelpful. A rate is only readable next to what it was taken over.
    let quota = match coverage.quota_requests {
        0 => String::new(),
        n => format!(" · {} on quota", format_count(n)),
    };
    match coverage.pct() {
        Some(pct) if pct >= 99.95 => {
            Span::styled(format!("all priced{quota}"), Style::default().fg(MUTED))
        }
        Some(pct) => Span::styled(
            format!("{pct:.0}% priced{quota}"),
            Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
        ),
        // Nothing billable in range, but there may still be quota work to disclose.
        None if coverage.quota_requests > 0 => Span::styled(
            quota.trim_start_matches(" · ").to_string(),
            Style::default().fg(MUTED),
        ),
        None => Span::raw(""),
    }
}
