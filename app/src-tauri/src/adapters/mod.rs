//! Adapters: the host side of the core's ports.
//!
//! PR 2c adds the Ollama LLM provider and the local-clock reading; the sleep
//! countdown adds the Windows input-activity reading (`windows_activity`); the Windows
//! adapters (`win_apps`, `win_volume`) arrive in PR 2d. The module is public so
//! `niva-eval` (feature `eval`) can drive the same provider the app does.

pub mod ollama;
pub mod system_clock;
#[cfg(windows)]
pub mod windows_activity;
