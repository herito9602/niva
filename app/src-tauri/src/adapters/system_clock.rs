//! The system clock behind the core's [`Clock`] port, with the local date and
//! time the system prompt states (design "Data flow"). Time zone resolution of
//! reminders (DST, wall-clock rules) is slice 4's `reminders.rs`; this only
//! reads the OS's local wall clock.

use std::time::SystemTime;

use niva_core::ports::{Clock, LocalDateTime};

pub struct SystemClock;

/// A reading in range, or `None` (a field the OS gave out of range is treated as
/// "the time zone is unknown", and the prompt then has no time line).
pub fn from_parts(
    year: u16,
    month: u16,
    day: u16,
    hour: u16,
    minute: u16,
) -> Option<LocalDateTime> {
    let valid = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && year >= 1970;
    valid.then_some(LocalDateTime {
        year,
        month: month as u8,
        day: day as u8,
        hour: hour as u8,
        minute: minute as u8,
    })
}

#[cfg(windows)]
fn read_local() -> Option<LocalDateTime> {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    let mut now = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    // SAFETY: `GetLocalTime` only writes the SYSTEMTIME it is given.
    unsafe { GetLocalTime(&mut now) };
    from_parts(now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute)
}

#[cfg(not(windows))]
fn read_local() -> Option<LocalDateTime> {
    None
}

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn local_now(&self) -> Option<LocalDateTime> {
        read_local()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_in_range_is_kept() {
        assert_eq!(
            from_parts(2026, 11, 4, 14, 5),
            Some(LocalDateTime {
                year: 2026,
                month: 11,
                day: 4,
                hour: 14,
                minute: 5
            })
        );
    }

    #[test]
    fn an_out_of_range_field_means_unknown() {
        for parts in [
            (2026, 0, 4, 14, 5),
            (2026, 13, 4, 14, 5),
            (2026, 11, 0, 14, 5),
            (2026, 11, 32, 14, 5),
            (2026, 11, 4, 24, 5),
            (2026, 11, 4, 14, 60),
            (1601, 11, 4, 14, 5),
        ] {
            assert_eq!(
                from_parts(parts.0, parts.1, parts.2, parts.3, parts.4),
                None
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_clock_knows_the_local_time() {
        let now = SystemClock.local_now().expect("GetLocalTime");
        assert!(now.year >= 2024, "{now:?}");
        assert!((1..=12).contains(&now.month));
    }

    #[test]
    fn the_instant_is_the_system_instant() {
        let before = SystemTime::now();
        let now = SystemClock.now();
        assert!(now >= before);
    }
}
