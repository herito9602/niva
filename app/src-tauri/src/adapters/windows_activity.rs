//! The Windows adapter of the core's [`InputActivity`] port: when did the user
//! last give keyboard or mouse input.
//!
//! It polls `GetLastInputInfo`. There is no keyboard hook and nothing here can
//! know WHICH key was pressed or where the mouse is: the only thing read is
//! the tick count of the last input of this session (owner decision
//! 2026-11-04). The stamp is only compared with an earlier one for equality.

use niva_core::ports::{ActivityError, InputActivity};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

pub struct WindowsActivity;

impl InputActivity for WindowsActivity {
    fn input_stamp(&self) -> Result<u64, ActivityError> {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: `info` is a valid LASTINPUTINFO whose `cbSize` is set, and
        // `GetLastInputInfo` only writes into it.
        let ok = unsafe { GetLastInputInfo(&mut info) };
        if ok == 0 {
            return Err(ActivityError);
        }
        Ok(u64::from(info.dwTime))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_reports_the_tick_of_its_last_input() {
        // Needs an interactive desktop session, like the app itself.
        let stamp = WindowsActivity.input_stamp();
        assert!(stamp.is_ok(), "GetLastInputInfo failed: {stamp:?}");
    }

    #[test]
    fn a_later_read_never_goes_back_in_time() {
        // The tick count only grows (until it wraps after 49 days), so the
        // stamp of a later read is never smaller than an earlier one.
        let first = WindowsActivity.input_stamp().unwrap();
        let second = WindowsActivity.input_stamp().unwrap();
        assert!(second >= first, "{first} then {second}");
    }
}
