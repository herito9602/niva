//! `set_volume` for Windows (spec `pc-actions`: set, raise/lower, mute/unmute,
//! clamping, truthful results).
//!
//! The decision is pure: [`plan`] turns a [`VolumeChange`] and the current
//! state into the writes to make, and [`verify`] judges the state read back
//! after them. [`apply`] runs both over an [`Endpoint`], so it is tested with a
//! fake device. The real endpoint is the default render device's
//! `IAudioEndpointVolume` (Core Audio), reached with a COM bracket per call:
//! the pool's worker threads have no COM of their own.

use niva_core::ports::{PcError, VolumeResult};
use niva_core::tools::VolumeChange;

#[cfg(test)]
mod tests;

/// The master volume as the device reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioState {
    /// 0-100.
    pub level: u8,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    /// There is no audio output device to control.
    NoDevice,
    Failed(String),
}

/// The writes a change needs, and whether its target was cut to 0-100.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub level: Option<u8>,
    pub mute: Option<bool>,
    pub clamped: bool,
}

/// What a device may differ from the requested level and still count as set:
/// many devices move in steps coarser than 1%.
pub const LEVEL_TOLERANCE: u8 = 2;

/// The writes `change` needs from the `current` state. Raising the sound of a
/// muted device also unmutes it, so "volume 30" is audible (like moving the
/// Windows slider); lowering it, or setting 0, leaves the mute alone.
pub fn plan(change: VolumeChange, current: AudioState) -> Plan {
    match change {
        VolumeChange::Set(wanted) => {
            let level = wanted.min(100);
            Plan {
                level: Some(level),
                mute: (level > 0 && current.muted).then_some(false),
                clamped: wanted > 100,
            }
        }
        VolumeChange::Delta(delta) => {
            let raw = i16::from(current.level) + i16::from(delta);
            let level = raw.clamp(0, 100);
            Plan {
                level: Some(level as u8),
                mute: (delta > 0 && current.muted).then_some(false),
                clamped: raw != level,
            }
        }
        VolumeChange::Mute(muted) => Plan {
            level: None,
            mute: Some(muted),
            clamped: false,
        },
    }
}

/// Judges the state read back after the writes of `plan`. The result reports
/// what the device says, not what was asked.
pub fn verify(plan: &Plan, after: AudioState) -> Result<VolumeResult, String> {
    if let Some(target) = plan.level
        && after.level.abs_diff(target) > LEVEL_TOLERANCE
    {
        return Err(format!(
            "the volume is {} after asking for {target}",
            after.level
        ));
    }
    if let Some(muted) = plan.mute
        && after.muted != muted
    {
        return Err(format!(
            "the device is {} after asking for {}",
            if after.muted { "muted" } else { "not muted" },
            if muted { "mute" } else { "unmute" },
        ));
    }
    Ok(VolumeResult {
        level: after.level,
        muted: after.muted,
        clamped: plan.clamped,
    })
}

/// A device that can be read and written (Core Audio on Windows, a fake in
/// tests).
pub trait Endpoint {
    fn state(&self) -> Result<AudioState, AudioError>;
    fn set_level(&self, level: u8) -> Result<(), AudioError>;
    fn set_mute(&self, muted: bool) -> Result<(), AudioError>;
}

fn pc_error(error: AudioError) -> PcError {
    match error {
        AudioError::NoDevice => PcError::Failed("no audio output device".into()),
        AudioError::Failed(why) => PcError::Failed(why),
    }
}

/// Plans, writes, reads back and verifies. A write that fails stops the plan;
/// a write the device did not take is a failure, never a success.
pub fn apply(endpoint: &dyn Endpoint, change: VolumeChange) -> Result<VolumeResult, PcError> {
    let before = endpoint.state().map_err(pc_error)?;
    let plan = plan(change, before);
    if let Some(level) = plan.level {
        endpoint.set_level(level).map_err(pc_error)?;
    }
    if let Some(muted) = plan.mute {
        endpoint.set_mute(muted).map_err(pc_error)?;
    }
    let after = endpoint.state().map_err(pc_error)?;
    verify(&plan, after).map_err(PcError::Failed)
}

/// The default render device through Core Audio. Every call brackets its own
/// COM initialisation and device lookup, so nothing is kept between calls or
/// shared between threads (the device can change between two commands).
#[cfg(windows)]
pub struct CoreAudio;

/// `E_NOTFOUND`: no default endpoint of that kind.
#[cfg(windows)]
const E_NOTFOUND: i32 = 0x8007_0490_u32 as i32;

#[cfg(windows)]
fn with_volume<T>(
    work: impl FnOnce(
        &windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume,
    ) -> windows::core::Result<T>,
) -> Result<T, AudioError> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};

    // Declared first, dropped last: every COM pointer below is released before
    // `CoUninitialize` runs.
    let com = super::com::ComGuard::multithreaded();
    if !com.is_usable() {
        return Err(AudioError::Failed("COM could not be initialised".into()));
    }
    let failed = |error: windows::core::Error| {
        if error.code().0 == E_NOTFOUND {
            AudioError::NoDevice
        } else {
            AudioError::Failed(error.to_string())
        }
    };
    // SAFETY: COM is initialised on this thread (above) and the interfaces are
    // used only inside this function.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(failed)?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(failed)?;
        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).map_err(failed)?;
        work(&volume).map_err(failed)
    }
}

#[cfg(windows)]
impl Endpoint for CoreAudio {
    fn state(&self) -> Result<AudioState, AudioError> {
        with_volume(|volume| {
            // SAFETY: plain getters on a live interface.
            let (scalar, muted) = unsafe {
                (
                    volume.GetMasterVolumeLevelScalar()?,
                    volume.GetMute()?.as_bool(),
                )
            };
            Ok(AudioState {
                level: (scalar * 100.0).round().clamp(0.0, 100.0) as u8,
                muted,
            })
        })
    }

    fn set_level(&self, level: u8) -> Result<(), AudioError> {
        with_volume(|volume| {
            // SAFETY: a null event context is allowed.
            unsafe {
                volume
                    .SetMasterVolumeLevelScalar(f32::from(level.min(100)) / 100.0, std::ptr::null())
            }
        })
    }

    fn set_mute(&self, muted: bool) -> Result<(), AudioError> {
        with_volume(|volume| {
            // SAFETY: a null event context is allowed.
            unsafe { volume.SetMute(muted, std::ptr::null()) }
        })
    }
}
