//! Adapters: the host side of the core's ports.
//!
//! PR 2c adds the Ollama LLM provider and the local-clock reading; the sleep
//! countdown adds the Windows input-activity reading (`windows_activity`); PR 2d-a
//! adds the Windows adapters of the SAFE actions (`win_apps`, `win_volume`,
//! assembled as `windows_pc`, with `com` for the worker threads) and the
//! Ollama keeper. PR 2d-b adds the real `close_app` (`win_close`) and power (`win_power`). Task 3.4 adds the voice sidecar adapter (`voice_sidecar`) and the `rodio` audio cues (`audio_cue`). The module is public
//! so `niva-eval` (feature `eval`) can drive the same provider the app does.

pub mod audio_cue;
#[cfg(windows)]
pub mod com;
pub mod ollama;
pub mod search;
pub mod system_clock;
pub mod voice_sidecar;
#[cfg(windows)]
pub mod win_apps;
#[cfg(windows)]
pub mod win_close;
#[cfg(windows)]
pub mod win_power;
#[cfg(windows)]
pub mod win_volume;
#[cfg(windows)]
pub mod windows_activity;
#[cfg(windows)]
pub mod windows_pc;
