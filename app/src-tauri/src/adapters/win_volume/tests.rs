//! The volume decision, the read-back and `apply` over a fake device. No test
//! here touches the real volume; the two real-device tests are `#[ignore]`d.

use std::sync::Mutex;

use super::*;

fn state(level: u8, muted: bool) -> AudioState {
    AudioState { level, muted }
}

// --- plan ----------------------------------------------------------------------

#[test]
fn an_absolute_level_is_the_target() {
    let p = plan(VolumeChange::Set(30), state(60, false));
    assert_eq!(
        p,
        Plan {
            level: Some(30),
            mute: None,
            clamped: false
        }
    );
}

#[test]
fn an_absolute_level_above_the_range_is_cut_to_100_by_the_adapter_too() {
    // The core already clamps; the adapter does not trust that.
    let p = plan(VolumeChange::Set(150), state(60, false));
    assert_eq!(p.level, Some(100));
    assert!(p.clamped);
}

#[test]
fn a_delta_is_added_to_the_current_level() {
    assert_eq!(
        plan(VolumeChange::Delta(10), state(50, false)).level,
        Some(60)
    );
    assert_eq!(
        plan(VolumeChange::Delta(-20), state(50, false)).level,
        Some(30)
    );
    assert!(!plan(VolumeChange::Delta(10), state(50, false)).clamped);
}

#[test]
fn a_delta_past_either_end_is_clamped_and_says_so() {
    let up = plan(VolumeChange::Delta(10), state(95, false));
    assert_eq!((up.level, up.clamped), (Some(100), true));
    let down = plan(VolumeChange::Delta(-20), state(10, false));
    assert_eq!((down.level, down.clamped), (Some(0), true));
    let exact = plan(VolumeChange::Delta(5), state(95, false));
    assert_eq!((exact.level, exact.clamped), (Some(100), false));
}

#[test]
fn mute_and_unmute_leave_the_level_alone() {
    let mute = plan(VolumeChange::Mute(true), state(40, false));
    assert_eq!((mute.level, mute.mute), (None, Some(true)));
    let unmute = plan(VolumeChange::Mute(false), state(40, true));
    assert_eq!((unmute.level, unmute.mute), (None, Some(false)));
}

#[test]
fn raising_the_sound_of_a_muted_device_unmutes_it() {
    // "volume 30" must be audible, like moving the Windows slider.
    assert_eq!(
        plan(VolumeChange::Set(30), state(60, true)).mute,
        Some(false)
    );
    assert_eq!(
        plan(VolumeChange::Delta(10), state(60, true)).mute,
        Some(false)
    );
}

#[test]
fn lowering_or_zeroing_does_not_touch_the_mute() {
    assert_eq!(plan(VolumeChange::Set(0), state(60, true)).mute, None);
    assert_eq!(plan(VolumeChange::Delta(-10), state(60, true)).mute, None);
    assert_eq!(plan(VolumeChange::Set(30), state(60, false)).mute, None);
}

// --- verify --------------------------------------------------------------------

#[test]
fn the_result_reports_what_the_device_says() {
    let p = plan(VolumeChange::Set(30), state(60, false));
    // A device with coarse steps lands one off: still set, reported as read.
    assert_eq!(
        verify(&p, state(31, false)),
        Ok(VolumeResult {
            level: 31,
            muted: false,
            clamped: false
        })
    );
}

#[test]
fn a_level_too_far_from_the_target_is_a_failure_not_a_success() {
    let p = plan(VolumeChange::Set(30), state(60, false));
    assert!(verify(&p, state(60, false)).is_err());
    assert!(verify(&p, state(30 + LEVEL_TOLERANCE + 1, false)).is_err());
    assert!(verify(&p, state(30 + LEVEL_TOLERANCE, false)).is_ok());
}

#[test]
fn a_mute_the_device_did_not_take_is_a_failure() {
    let p = plan(VolumeChange::Mute(true), state(40, false));
    assert!(verify(&p, state(40, false)).is_err());
    assert_eq!(verify(&p, state(40, true)).map(|r| r.muted), Ok(true));
    let p = plan(VolumeChange::Mute(false), state(40, true));
    assert!(verify(&p, state(40, true)).is_err());
}

#[test]
fn clamping_is_carried_into_the_result() {
    let p = plan(VolumeChange::Delta(10), state(95, false));
    assert_eq!(verify(&p, state(100, false)).map(|r| r.clamped), Ok(true));
}

// --- apply, over a fake device ---------------------------------------------------

struct FakeDevice {
    now: Mutex<AudioState>,
    writes: Mutex<Vec<String>>,
    /// Ignore writes (a device that does not take them).
    deaf: bool,
    /// Round written levels to this step.
    step: u8,
    no_device: bool,
    fail_write: bool,
}

impl FakeDevice {
    fn at(level: u8, muted: bool) -> FakeDevice {
        FakeDevice {
            now: Mutex::new(state(level, muted)),
            writes: Mutex::default(),
            deaf: false,
            step: 1,
            no_device: false,
            fail_write: false,
        }
    }

    fn writes(&self) -> Vec<String> {
        self.writes.lock().unwrap().clone()
    }
}

impl Endpoint for FakeDevice {
    fn state(&self) -> Result<AudioState, AudioError> {
        if self.no_device {
            return Err(AudioError::NoDevice);
        }
        Ok(*self.now.lock().unwrap())
    }

    fn set_level(&self, level: u8) -> Result<(), AudioError> {
        self.writes.lock().unwrap().push(format!("level {level}"));
        if self.fail_write {
            return Err(AudioError::Failed("write refused".into()));
        }
        if !self.deaf {
            self.now.lock().unwrap().level = level / self.step * self.step;
        }
        Ok(())
    }

    fn set_mute(&self, muted: bool) -> Result<(), AudioError> {
        self.writes.lock().unwrap().push(format!("mute {muted}"));
        if self.fail_write {
            return Err(AudioError::Failed("write refused".into()));
        }
        if !self.deaf {
            self.now.lock().unwrap().muted = muted;
        }
        Ok(())
    }
}

#[test]
fn setting_a_level_writes_it_and_reports_the_read_back_state() {
    let device = FakeDevice::at(60, false);
    let result = apply(&device, VolumeChange::Set(30)).unwrap();
    assert_eq!(device.writes(), vec!["level 30"]);
    assert_eq!(
        result,
        VolumeResult {
            level: 30,
            muted: false,
            clamped: false
        }
    );
}

#[test]
fn a_delta_past_the_top_reports_the_clamp() {
    let device = FakeDevice::at(95, false);
    let result = apply(&device, VolumeChange::Delta(10)).unwrap();
    assert_eq!((result.level, result.clamped), (100, true));
}

#[test]
fn mute_and_unmute_write_only_the_mute() {
    let device = FakeDevice::at(40, false);
    assert!(apply(&device, VolumeChange::Mute(true)).unwrap().muted);
    assert!(!apply(&device, VolumeChange::Mute(false)).unwrap().muted);
    assert_eq!(device.writes(), vec!["mute true", "mute false"]);
}

#[test]
fn a_set_on_a_muted_device_unmutes_and_says_the_real_state() {
    let device = FakeDevice::at(60, true);
    let result = apply(&device, VolumeChange::Set(30)).unwrap();
    assert_eq!(device.writes(), vec!["level 30", "mute false"]);
    assert!(!result.muted);
}

#[test]
fn a_device_with_coarse_steps_reports_its_own_level() {
    let mut device = FakeDevice::at(60, false);
    device.step = 2;
    assert_eq!(apply(&device, VolumeChange::Set(31)).unwrap().level, 30);
}

#[test]
fn a_device_that_ignores_the_write_is_reported_as_a_failure() {
    let mut device = FakeDevice::at(60, false);
    device.deaf = true;
    assert!(matches!(
        apply(&device, VolumeChange::Set(30)),
        Err(PcError::Failed(_))
    ));
    assert!(matches!(
        apply(&device, VolumeChange::Mute(true)),
        Err(PcError::Failed(_))
    ));
}

#[test]
fn a_refused_write_is_a_failure_and_stops_the_plan() {
    let mut device = FakeDevice::at(60, true);
    device.fail_write = true;
    assert!(matches!(
        apply(&device, VolumeChange::Set(30)),
        Err(PcError::Failed(_))
    ));
    assert_eq!(
        device.writes(),
        vec!["level 30"],
        "no mute write after a failed level"
    );
}

#[test]
fn without_an_audio_device_nothing_is_written_and_the_error_is_truthful() {
    let mut device = FakeDevice::at(60, false);
    device.no_device = true;
    let error = apply(&device, VolumeChange::Set(30)).unwrap_err();
    assert_eq!(error, PcError::Failed("no audio output device".into()));
    assert!(device.writes().is_empty());
}

// --- the real device (ignored: needs hardware; the second changes the system) ----

#[cfg(windows)]
mod windows_live {
    use super::*;
    use crate::adapters::win_volume::CoreAudio;

    /// Read-only: reads the default output device.
    #[test]
    #[ignore = "needs an audio output device (read-only)"]
    fn the_default_device_reports_a_level_between_0_and_100() {
        let now = CoreAudio.state().expect("an audio device");
        assert!(now.level <= 100);
    }

    /// Writes the CURRENT level and mute back, so the owner hears nothing, but
    /// it does call the real setters. Ignored for that reason.
    #[test]
    #[ignore = "calls the real volume setters (with the current values)"]
    fn writing_the_current_state_back_succeeds_and_reads_the_same() {
        let before = CoreAudio.state().expect("an audio device");
        CoreAudio.set_level(before.level).expect("set level");
        CoreAudio.set_mute(before.muted).expect("set mute");
        let after = CoreAudio.state().unwrap();
        assert!(after.level.abs_diff(before.level) <= LEVEL_TOLERANCE);
        assert_eq!(after.muted, before.muted);
    }
}
