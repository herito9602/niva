//! Test doubles shared by the host's tests (compiled for tests only).

use std::sync::Mutex;

use niva_core::ports::{ActivityError, InputActivity};

/// An activity source the test moves by hand: `touch` is the user giving
/// input, `set_unreadable` is `GetLastInputInfo` failing.
#[derive(Default)]
pub struct FakeActivity {
    stamp: Mutex<u64>,
    unreadable: Mutex<bool>,
    reads: Mutex<usize>,
}

impl FakeActivity {
    pub fn touch(&self) {
        *self.stamp.lock().unwrap() += 1;
    }

    pub fn set_unreadable(&self, value: bool) {
        *self.unreadable.lock().unwrap() = value;
    }

    /// How many times the source was read.
    pub fn reads(&self) -> usize {
        *self.reads.lock().unwrap()
    }
}

impl InputActivity for FakeActivity {
    fn input_stamp(&self) -> Result<u64, ActivityError> {
        *self.reads.lock().unwrap() += 1;
        if *self.unreadable.lock().unwrap() {
            return Err(ActivityError);
        }
        Ok(*self.stamp.lock().unwrap())
    }
}
