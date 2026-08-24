//! Pricing coverage: the share of billable requests that actually carry a price, and
//! the quota-billed volume that is deliberately not counted as a gap.

use super::*;

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

#[test]
fn the_header_shows_how_much_of_the_spend_is_actually_priced() {
    // Provenance is the project's differentiator and it lived in an internal enum. A reader
    // could take a total at face value without learning it covered two thirds of the requests.
    use ratatui::{backend::TestBackend, Terminal};

    let render = |app: &App| -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 3)).expect("backend");
        terminal
            .draw(|frame| crate::ui::panels::header::draw_header(frame, frame.area(), app))
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    };

    let mut partial = test_app(vec![
        usage_at("2026-08-01", 1000, Some(1.0)),
        usage_at("2026-08-01", 1000, Some(1.0)),
        usage_at("2026-08-01", 1000, None),
    ]);
    partial.recompute();
    assert!(
        render(&partial).contains("67% priced"),
        "{}",
        render(&partial)
    );

    let mut complete = test_app(vec![usage_at("2026-08-01", 1000, Some(1.0))]);
    complete.recompute();
    let rendered = render(&complete);
    assert!(rendered.contains("all priced"), "{rendered}");
    assert!(
        !rendered.contains('%'),
        "a fully priced range should not shout a percentage:\n{rendered}"
    );
}

#[test]
fn quota_billed_usage_does_not_reduce_pricing_coverage() {
    // Measured against real data: the header read "71.6% priced" when every unpriced request was
    // Ollama Cloud usage the tool deliberately refuses to price. A correct refusal to invent a
    // number was being reported as a failure to produce one.
    let now = crate::utils::now();
    let mut usages: Vec<Usage> = (0..7).map(|_| usage(None, None, Some(1.0), 100)).collect();
    usages.extend((0..3).map(|i| quota_usage(100, now - i)));

    let c = coverage(&usages);
    assert_eq!(
        c.pct(),
        Some(100.0),
        "quota-billed work is not a pricing gap; every priceable request here was priced"
    );
    assert_eq!(
        c.quota_requests, 3,
        "and it must still be counted, or the volume silently disappears"
    );
}

#[test]
fn a_billable_model_with_no_rate_still_counts_against_coverage() {
    // The anti-test for the one above: the fix must not make coverage unconditionally 100% by
    // swallowing the real case the figure exists to report.
    let mut unpriceable = usage(None, None, None, 100);
    unpriceable.model = "a-model-no-table-has".into();
    let usages = vec![usage(None, None, Some(1.0), 100), unpriceable];

    let c = coverage(&usages);
    assert_eq!(c.pct(), Some(50.0), "a genuine missing rate is still a gap");
    assert_eq!(c.quota_requests, 0);
}

#[test]
fn the_header_discloses_quota_volume_rather_than_dropping_it() {
    // "all priced" while thousands of requests sit outside the ratio is true and unhelpful.
    use ratatui::{backend::TestBackend, Terminal};
    let now = crate::utils::now();
    let mut usages = vec![usage(None, None, Some(1.0), 100)];
    usages.extend((0..3).map(|i| quota_usage(100, now - i)));
    let mut app = test_app(usages);
    app.recompute();

    let mut terminal = Terminal::new(TestBackend::new(120, 3)).expect("backend");
    terminal
        .draw(|frame| crate::ui::panels::header::draw_header(frame, frame.area(), &app))
        .expect("draw");
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();

    assert!(rendered.contains("all priced"), "{rendered}");
    assert!(
        rendered.contains("on quota"),
        "the denominator the percentage was taken over must stay visible:\n{rendered}"
    );
}

#[test]
fn subscription_rows_are_quota_volume_not_a_coverage_gap() {
    let now = crate::utils::now();
    let mut usages = vec![usage(None, None, Some(1.0), 100)];
    usages.extend((0..4).map(|i| subscription_usage(100, now - i)));
    let c = coverage(&usages);
    assert_eq!(c.pct(), Some(100.0), "{c:?}");
    assert_eq!(c.quota_requests, 4);
}
