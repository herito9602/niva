//! The real `power` for Windows (PR 2d-b; spec `pc-actions`, `action-safety`).
//!
//! What each action does, and why:
//!
//! - **lock**: `LockWorkStation`.
//! - **sleep**: only on a Modern Standby PC (`AoAc`, "always on, always
//!   connected"), through `PostMessage(HWND_BROADCAST, WM_SYSCOMMAND,
//!   SC_MONITORPOWER, 2)`, which returns at once. The classic
//!   `SetSuspendState(false, false, false)` HUNG the owner's PC in a real test
//!   (a forced shutdown was needed), while the broadcast put it into Modern
//!   Standby (Kernel-Power event 506, reason SC_MONITORPOWER, woke by a key). A
//!   PC without Modern Standby (S3) has no tested path, so sleep there is a
//!   truthful "not available yet" and NO call is made: an untested path does not
//!   ship. A sleep still only reaches here from `release_held`, after the
//!   countdown.
//! - **restart / shutdown**: `SeShutdownPrivilege` is enabled and checked, then
//!   `ExitWindowsEx` with a reboot or power-off flag and NO force flag, so apps
//!   with unsaved work can object and Windows says so. A failure of either step
//!   is reported, never ignored. They are click-only, as before.
//!
//! The decisions are in [`apply`], over a small trait ([`PowerSystem`]) so they
//! are tested with fakes. The Windows calls are in the `cfg(windows)` part at the
//! end; their real effects are only in `#[ignore]`d tests, never run by `cargo
//! test`.

use niva_core::ports::PcError;
use niva_core::tools::PowerKind;

#[cfg(test)]
mod tests;

/// What the power actions need from the OS. Windows in production, a fake in
/// tests.
pub trait PowerSystem: Send + Sync {
    fn lock_workstation(&self) -> Result<(), String>;
    /// `Some(true)` on a Modern Standby PC, `Some(false)` on one without it,
    /// `None` when the OS could not say.
    fn modern_standby(&self) -> Option<bool>;
    /// Asks Windows to enter Modern Standby. Returns at once, it never blocks.
    fn start_modern_standby(&self) -> Result<(), String>;
    fn enable_shutdown_privilege(&self) -> Result<(), String>;
    /// Takes the privilege away again (best effort, after the exit call).
    fn disable_shutdown_privilege(&self) -> Result<(), String>;
    /// Starts a restart or a power-off, without forcing apps to close.
    fn exit_windows(&self, restart: bool) -> Result<(), String>;
}

fn failed(why: String) -> PcError {
    PcError::Failed(why)
}

/// Runs one approved power action.
pub fn apply(system: &dyn PowerSystem, kind: PowerKind) -> Result<(), PcError> {
    match kind {
        PowerKind::Lock => system.lock_workstation().map_err(failed),
        PowerKind::Sleep => match system.modern_standby() {
            Some(true) => system.start_modern_standby().map_err(failed),
            // No Modern Standby, or unknown: no tested path, so no call.
            Some(false) | None => Err(PcError::NotSupported),
        },
        PowerKind::Restart | PowerKind::Shutdown => {
            system.enable_shutdown_privilege().map_err(failed)?;
            let exit = system
                .exit_windows(kind == PowerKind::Restart)
                .map_err(failed);
            // Whatever the exit call said, the privilege does not stay enabled in
            // this process. A failure to drop it cannot change the answer.
            let _ = system.disable_shutdown_privilege();
            exit
        }
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

/// The real power system.
#[cfg(windows)]
pub struct RealPower;

#[cfg(windows)]
fn last_error(what: &str) -> String {
    // SAFETY: reads the calling thread's last error.
    format!("{what} failed (error {})", unsafe {
        windows_sys::Win32::Foundation::GetLastError()
    })
}

#[cfg(windows)]
impl PowerSystem for RealPower {
    fn lock_workstation(&self) -> Result<(), String> {
        use windows_sys::Win32::System::Shutdown::LockWorkStation;

        // SAFETY: no arguments.
        if unsafe { LockWorkStation() } != 0 {
            Ok(())
        } else {
            Err(last_error("LockWorkStation"))
        }
    }

    fn modern_standby(&self) -> Option<bool> {
        use windows_sys::Win32::System::Power::{
            CallNtPowerInformation, SYSTEM_POWER_CAPABILITIES, SystemPowerCapabilities,
        };

        // SAFETY: a plain-data struct, zeroed is a valid empty value.
        let mut capabilities: SYSTEM_POWER_CAPABILITIES = unsafe { std::mem::zeroed() };
        // SAFETY: the output buffer is `capabilities`, and its size is passed.
        let status = unsafe {
            CallNtPowerInformation(
                SystemPowerCapabilities,
                std::ptr::null(),
                0,
                (&mut capabilities as *mut SYSTEM_POWER_CAPABILITIES).cast(),
                std::mem::size_of::<SYSTEM_POWER_CAPABILITIES>() as u32,
            )
        };
        // STATUS_SUCCESS is 0; any other status means the answer is unknown.
        (status == 0).then_some(capabilities.AoAc)
    }

    fn start_modern_standby(&self) -> Result<(), String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_BROADCAST, PostMessageW, SC_MONITORPOWER, WM_SYSCOMMAND,
        };

        // lParam 2: the display is powering off. On a Modern Standby PC this
        // enters standby (verified on the owner's PC); the post returns at once.
        // SAFETY: a plain post, no pointer in any parameter.
        let posted =
            unsafe { PostMessageW(HWND_BROADCAST, WM_SYSCOMMAND, SC_MONITORPOWER as usize, 2) };
        if posted != 0 {
            Ok(())
        } else {
            Err(last_error("PostMessageW"))
        }
    }

    fn enable_shutdown_privilege(&self) -> Result<(), String> {
        set_shutdown_privilege(true)
    }

    fn disable_shutdown_privilege(&self) -> Result<(), String> {
        set_shutdown_privilege(false)
    }

    fn exit_windows(&self, restart: bool) -> Result<(), String> {
        use windows_sys::Win32::System::Shutdown::{
            EWX_POWEROFF, EWX_REBOOT, ExitWindowsEx, SHTDN_REASON_FLAG_PLANNED,
            SHTDN_REASON_MAJOR_OTHER, SHTDN_REASON_MINOR_OTHER,
        };

        // No force flag: an app with unsaved work may object, and Windows then
        // tells the user which one.
        let how = if restart { EWX_REBOOT } else { EWX_POWEROFF };
        let reason =
            SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_MINOR_OTHER | SHTDN_REASON_FLAG_PLANNED;
        // SAFETY: plain flags, no pointer.
        if unsafe { ExitWindowsEx(how, reason) } != 0 {
            Ok(())
        } else {
            Err(last_error("ExitWindowsEx"))
        }
    }
}

/// Enables or disables `SeShutdownPrivilege` on this process's token and checks
/// the result.
#[cfg(windows)]
fn set_shutdown_privilege(enabled: bool) -> Result<(), String> {
    {
        use windows_sys::Win32::Foundation::{
            CloseHandle, ERROR_SUCCESS, GetLastError, HANDLE, SetLastError,
        };
        use windows_sys::Win32::Security::{
            AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
            SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        let name: Vec<u16> = "SeShutdownPrivilege\0".encode_utf16().collect();
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: `token` is a valid out pointer; the pseudo handle of the
        // current process needs no closing.
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )
        };
        if opened == 0 {
            return Err(last_error("OpenProcessToken"));
        }
        // SAFETY: plain-data struct; `name` is NUL-terminated and outlives the call.
        let mut privileges: TOKEN_PRIVILEGES = unsafe { std::mem::zeroed() };
        privileges.PrivilegeCount = 1;
        let looked_up = unsafe {
            LookupPrivilegeValueW(
                std::ptr::null(),
                name.as_ptr(),
                &mut privileges.Privileges[0].Luid,
            )
        };
        let result = if looked_up == 0 {
            Err(last_error("LookupPrivilegeValueW"))
        } else {
            privileges.Privileges[0] = LUID_AND_ATTRIBUTES {
                Luid: privileges.Privileges[0].Luid,
                Attributes: if enabled { SE_PRIVILEGE_ENABLED } else { 0 },
            };
            // SAFETY: `token` is open with TOKEN_ADJUST_PRIVILEGES and
            // `privileges` is initialised. AdjustTokenPrivileges reports "not
            // all assigned" through the last error, even when it returns true,
            // so the error is cleared first and checked after.
            unsafe { SetLastError(ERROR_SUCCESS) };
            let adjusted = unsafe {
                AdjustTokenPrivileges(
                    token,
                    0,
                    &privileges,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            // SAFETY: reads the calling thread's last error.
            let error = unsafe { GetLastError() };
            if adjusted == 0 || error != ERROR_SUCCESS {
                Err(format!(
                    "the shutdown privilege could not be enabled (error {error})"
                ))
            } else {
                Ok(())
            }
        };
        // SAFETY: `token` was opened above and is closed once.
        unsafe { CloseHandle(token) };
        result
    }
}

#[cfg(all(test, windows))]
mod real_tests {
    //! Against the real OS. The first test only READS; the others have real
    //! effects on the PC and are `#[ignore]`d: `cargo test` never runs them, and
    //! nobody should run them unattended.

    use super::*;

    #[test]
    #[ignore = "reads the real power capabilities; run by hand"]
    fn the_real_pc_reports_whether_it_has_modern_standby() {
        let reported = RealPower.modern_standby();
        eprintln!("modern standby (AoAc): {reported:?}");
        assert!(reported.is_some(), "CallNtPowerInformation failed");
    }

    #[test]
    #[ignore = "LOCKS the PC"]
    fn really_locks_the_workstation() {
        RealPower.lock_workstation().unwrap();
    }

    #[test]
    #[ignore = "PUTS THE PC TO SLEEP (modern standby)"]
    fn really_enters_modern_standby() {
        assert_eq!(RealPower.modern_standby(), Some(true));
        RealPower.start_modern_standby().unwrap();
    }

    #[test]
    #[ignore = "enables SeShutdownPrivilege on this process (no shutdown)"]
    fn really_enables_the_shutdown_privilege() {
        RealPower.enable_shutdown_privilege().unwrap();
    }

    #[test]
    #[ignore = "RESTARTS THE PC"]
    fn really_restarts() {
        RealPower.enable_shutdown_privilege().unwrap();
        RealPower.exit_windows(true).unwrap();
    }

    #[test]
    #[ignore = "SHUTS THE PC DOWN"]
    fn really_shuts_down() {
        RealPower.enable_shutdown_privilege().unwrap();
        RealPower.exit_windows(false).unwrap();
    }
}
