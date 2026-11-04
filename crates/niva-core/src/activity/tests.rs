//! Input watch tests: baseline, grace period, fail closed.

use std::time::Duration;

use super::*;
use crate::ports::testing::FakeActivity;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn nothing_happening_is_quiet_for_the_whole_countdown() {
    let source = FakeActivity::new(500);
    let mut watch = InputWatch::start(&source).unwrap();
    for step in 0..=100 {
        assert_eq!(watch.poll(ms(step * 100), &source), Watch::Quiet, "{step}");
    }
}

#[test]
fn input_after_the_grace_period_is_reported() {
    let source = FakeActivity::new(500);
    let mut watch = InputWatch::start(&source).unwrap();
    assert_eq!(watch.poll(ms(1_500), &source), Watch::Quiet);
    source.touch();
    assert_eq!(watch.poll(ms(1_600), &source), Watch::Input);
}

#[test]
fn mouse_jitter_right_after_the_accept_click_is_ignored() {
    // The click that accepted is already in the baseline; the hand moving
    // afterwards touches the stamp several times during the grace period.
    let source = FakeActivity::new(500);
    let mut watch = InputWatch::start(&source).unwrap();
    for step in 1..10 {
        source.touch();
        assert_eq!(
            watch.poll(ms(step * 100), &source),
            Watch::Quiet,
            "jitter at {} ms",
            step * 100
        );
    }
    // Once the jitter stops, the baseline is the last jittered value: quiet...
    assert_eq!(watch.poll(ms(1_000), &source), Watch::Quiet);
    assert_eq!(watch.poll(ms(1_100), &source), Watch::Quiet);
    // ...and a real key press afterwards cancels.
    source.touch();
    assert_eq!(watch.poll(ms(1_200), &source), Watch::Input);
}

#[test]
fn the_grace_period_is_one_second_and_ends_exactly_there() {
    assert_eq!(GRACE, Duration::from_secs(1));
    let source = FakeActivity::new(1);
    let mut watch = InputWatch::start(&source).unwrap();
    source.touch();
    assert_eq!(watch.poll(GRACE - ms(1), &source), Watch::Quiet);
    source.touch();
    assert_eq!(watch.poll(GRACE, &source), Watch::Input);
}

#[test]
fn input_at_the_very_start_is_taken_into_the_baseline_not_reported() {
    let source = FakeActivity::new(10);
    source.touch(); // before `start`: the accepting input
    let mut watch = InputWatch::start(&source).unwrap();
    assert_eq!(watch.poll(ms(100), &source), Watch::Quiet);
    assert_eq!(watch.poll(ms(2_000), &source), Watch::Quiet);
}

#[test]
fn an_unreadable_source_cannot_start_a_watch() {
    let source = FakeActivity::new(1);
    source.set_unreadable(true);
    assert!(InputWatch::start(&source).is_err());
}

#[test]
fn a_source_that_fails_while_watching_is_unreadable_in_and_after_the_grace() {
    let source = FakeActivity::new(1);
    let mut watch = InputWatch::start(&source).unwrap();
    source.set_unreadable(true);
    assert_eq!(watch.poll(ms(100), &source), Watch::Unreadable);
    assert_eq!(watch.poll(ms(5_000), &source), Watch::Unreadable);
    // It never reads "quiet" when it cannot read: that would let the PC sleep.
    source.set_unreadable(false);
    assert_eq!(watch.poll(ms(5_100), &source), Watch::Quiet);
}

#[test]
fn polling_is_every_100_ms() {
    assert_eq!(POLL_INTERVAL, ms(100));
}
