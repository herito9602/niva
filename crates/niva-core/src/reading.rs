//! How long a final reply stays on screen so it can be read.
//!
//! Pure functions; the host's effect runner feeds them the length of the
//! reply and the time elapsed since it was shown. The reducer's `Settle` timer
//! (start at `speak_done`) keeps its meaning: it is the quiet time after the
//! speech. This module only stretches it when the message is long, so the
//! island is hidden at
//!
//! ```text
//! max(speech end + settle, shown_at + reading_time(text))
//! ```
//!
//! Confirmation prompts and follow-up questions do not use it: they stay until
//! answered or until their own timeout.

use std::time::Duration;

/// Shortest time any final reply stays up.
pub const READING_FLOOR: Duration = Duration::from_millis(2_500);
/// Longest hold. A message that needs more scrolls past it anyway.
pub const READING_CAP: Duration = Duration::from_millis(12_000);
/// Fixed part: eyes land on the island and the appear animation plays.
const READING_BASE: Duration = Duration::from_millis(1_500);
/// About 180 words per minute.
const READING_PER_CHAR: Duration = Duration::from_millis(55);

/// Minimum visible time of a reply of `chars` characters:
/// `clamp(1500 ms + 55 ms per character, 2500 ms, 12000 ms)`.
pub fn reading_time(chars: usize) -> Duration {
    let chars = u32::try_from(chars).unwrap_or(u32::MAX);
    let wanted = READING_PER_CHAR
        .checked_mul(chars)
        .and_then(|per_char| per_char.checked_add(READING_BASE))
        .unwrap_or(READING_CAP);
    wanted.clamp(READING_FLOOR, READING_CAP)
}

/// How long to wait from the moment the speech ended (`since_shown` after the
/// reply appeared) before hiding: the usual `settle`, or whatever is left of
/// the reading time if that is longer.
pub fn settle_delay(settle: Duration, reading: Duration, since_shown: Duration) -> Duration {
    settle.max(reading.saturating_sub(since_shown))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn short_messages_get_the_floor() {
        assert_eq!(reading_time(0), ms(2_500));
        assert_eq!(reading_time(4), ms(2_500));
        assert_eq!(reading_time(18), ms(2_500));
    }

    #[test]
    fn the_time_grows_with_the_length() {
        assert_eq!(reading_time(40), ms(1_500 + 40 * 55));
        assert_eq!(reading_time(100), ms(7_000));
        assert!(reading_time(60) < reading_time(61));
    }

    #[test]
    fn a_400_char_message_gets_the_cap() {
        assert_eq!(reading_time(400), ms(12_000));
    }

    #[test]
    fn the_cap_holds_for_any_length() {
        assert_eq!(reading_time(191), ms(12_000));
        assert_eq!(reading_time(100_000), ms(12_000));
        assert_eq!(reading_time(usize::MAX), ms(12_000));
    }

    #[test]
    fn the_floor_never_exceeds_the_cap_and_is_monotonic() {
        let mut previous = Duration::ZERO;
        for chars in 0..500 {
            let t = reading_time(chars);
            assert!(t >= READING_FLOOR && t <= READING_CAP);
            assert!(t >= previous);
            previous = t;
        }
    }

    #[test]
    fn settle_delay_is_the_usual_settle_for_short_replies() {
        let settle = ms(3_000);
        // 2.5 s of reading is already over by the time the 3 s settle would end.
        assert_eq!(settle_delay(settle, ms(2_500), ms(0)), ms(3_000));
        assert_eq!(settle_delay(settle, reading_time(10), ms(100)), ms(3_000));
    }

    #[test]
    fn settle_delay_waits_for_the_reading_time_of_long_replies() {
        let settle = ms(3_000);
        let reading = reading_time(400);
        // Speech ended at once (no TTS): the whole reading time is left.
        assert_eq!(settle_delay(settle, reading, ms(0)), ms(12_000));
        // Speech took 5 s: 7 s are left.
        assert_eq!(settle_delay(settle, reading, ms(5_000)), ms(7_000));
    }

    #[test]
    fn settle_delay_falls_back_to_the_settle_once_the_reading_time_has_passed() {
        let settle = ms(3_000);
        let reading = reading_time(400);
        // Speech ended exactly when the reading time did: the settle still applies.
        assert_eq!(settle_delay(settle, reading, ms(12_000)), ms(3_000));
        // Speech outlasted it: same.
        assert_eq!(settle_delay(settle, reading, ms(30_000)), ms(3_000));
    }
}
