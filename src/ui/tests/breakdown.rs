//! The per-model breakdown, including how subscription work is presented.

use super::*;

#[test]
fn an_all_subscription_breakdown_says_on_quota_and_shows_the_counterfactual() {
    // Before: "EST. PAID COST $0.0000" for a month of Max-plan work. The tile read as free.
    let now = crate::utils::now();
    let mut app = test_app(
        (0..5)
            .map(|i| subscription_usage(20_000, now - i))
            .collect(),
    );
    app.recompute();

    let rendered = render_breakdown(&app, 60, 12);
    assert!(rendered.contains("on quota"), "{rendered}");
    assert!(
        rendered.contains("API-RATE EQUIV.") && rendered.contains("≈ $"),
        "the list-rate figure must survive as a labelled counterfactual:\n{rendered}"
    );
    assert!(
        !rendered.contains("$0.0000"),
        "never render plan-billed work as zero dollars:\n{rendered}"
    );
}

#[test]
fn a_priced_breakdown_still_shows_dollars_and_no_counterfactual_line() {
    // The anti-test: the fix must not replace real dollars with "on quota" when any exist.
    let mut app = test_app(vec![usage(None, None, Some(2.5), 100)]);
    app.recompute();
    let rendered = render_breakdown(&app, 60, 12);
    assert!(rendered.contains("$2.5000"), "{rendered}");
    assert!(!rendered.contains("API-RATE EQUIV."), "{rendered}");
}

#[test]
fn the_paid_tile_does_not_show_zero_dollars_for_subscription_work() {
    let now = crate::utils::now();
    let mut app = test_app(
        (0..3)
            .map(|i| subscription_usage(20_000, now - i))
            .collect(),
    );
    app.recompute();

    let rendered = render_metrics(&app, 130, 7);
    assert!(rendered.contains("on quota"), "{rendered}");
    assert!(!rendered.contains("$0.0000"), "{rendered}");
}
