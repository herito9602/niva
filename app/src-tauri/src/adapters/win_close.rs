//! The real `close_app` for Windows (PR 2d-b; spec `pc-actions` "Close app").
//!
//! The safety gate already decided WHAT may be closed: an exe the user confirmed,
//! not protected, in the user's session. This module only asks that app's
//! windows to close, politely: `WM_CLOSE` posted to a window, exactly what the
//! close button does. Never `TerminateProcess`, never a forced close; an app
//! with unsaved work shows its own dialog and the reply says the app did not
//! close.
//!
//! - Desktop apps: the visible top-level windows owned by a process of the exe.
//! - UWP apps (`CalculatorApp.exe`, Settings, Photos) own no top-level window.
//!   Their window is a frame (`ApplicationFrameWindow`) owned by
//!   `ApplicationFrameHost.exe`. The frame is matched to the app by its
//!   AppUserModelID: the frame's `PKEY_AppUserModel_ID` property must be
//!   non-empty and equal the AUMID of a target process
//!   (`GetApplicationUserModelId`). This also works when the app is minimized
//!   and suspended, when the frame has no `CoreWindow` child any more. A
//!   process with no AUMID has no frame route. `applicationframehost` stays
//!   protected: it is never a target, only the owner of the frames.
//! - Right before each post the window's owner is read again (and a frame's
//!   AUMID re-checked): a handle that was reused by another process is skipped.
//! - The wait is for the WINDOWS to go away (up to 3 s). Then a desktop app's
//!   process is checked too, within the same budget: if it is still running the
//!   outcome is `KeepsRunning`, not `Closed`. A UWP app whose frame is gone
//!   counts as closed (Windows suspends or ends its process).
//!
//! The decisions are pure and run over a small trait ([`WindowSystem`]) so they
//! are tested with fakes. The Windows calls are in the `cfg(windows)` part at the
//! end.

use std::time::Duration;

use niva_core::ports::{CloseOutcome, PcError};
use niva_core::safety::is_protected;

use super::win_apps::ProcessEntry;

#[cfg(test)]
mod tests;

/// How long the windows (and then the process) get to go away.
pub const CLOSE_WAIT: Duration = Duration::from_secs(3);
/// How often the wait looks again.
pub const CLOSE_POLL: Duration = Duration::from_millis(100);

/// The process that owns the frame window of every UWP app.
const FRAME_HOST: &str = "applicationframehost.exe";

/// A window handle. `isize` so the logic does not depend on the Windows types.
pub type Hwnd = isize;

/// A top-level window as the window enumeration reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopWindow {
    pub handle: Hwnd,
    /// The process that owns the window.
    pub pid: u32,
    pub visible: bool,
}

/// What closing needs from the system. Windows in production, a fake in tests.
pub trait WindowSystem: Send + Sync {
    /// The process list. A failed snapshot is an error, never an empty list.
    fn processes(&self) -> Result<Vec<ProcessEntry>, PcError>;
    fn top_windows(&self) -> Vec<TopWindow>;
    /// The AppUserModelID of a process (`None` for a plain desktop process).
    fn process_aumid(&self, pid: u32) -> Option<String>;
    /// The AppUserModelID property of a window (`None` when it has none).
    fn window_aumid(&self, window: Hwnd) -> Option<String>;
    /// The process that owns `window` right now (`None` if it is gone).
    fn window_pid(&self, window: Hwnd) -> Option<u32>;
    /// Posts `WM_CLOSE` to `window`. Returns at once.
    fn post_close(&self, window: Hwnd) -> Result<(), String>;
    fn pause(&self, how_long: Duration);
}

/// The file name of an adapter-supplied exe name, lower case.
fn file_name(exe: &str) -> String {
    exe.rsplit(['\\', '/'])
        .next()
        .unwrap_or(exe)
        .trim()
        .to_lowercase()
}

/// The processes whose windows are looked for.
struct Target {
    /// The processes of the exe, in the user's session.
    pids: Vec<u32>,
    /// The `ApplicationFrameHost` processes of the user's session: the only
    /// owners of a frame window that counts.
    frame_hosts: Vec<u32>,
    /// The non-empty AppUserModelIDs of the target processes.
    aumids: Vec<String>,
}

fn target(system: &dyn WindowSystem, processes: &[ProcessEntry], exe: &str) -> Target {
    let wanted = file_name(exe);
    let pids_named = |name: &str| -> Vec<u32> {
        processes
            .iter()
            .filter(|p| p.in_user_session && file_name(&p.exe_name) == name)
            .map(|p| p.pid)
            .collect()
    };
    let pids = pids_named(&wanted);
    let mut aumids: Vec<String> = Vec::new();
    for pid in &pids {
        if let Some(aumid) = system.process_aumid(*pid).filter(|a| !a.is_empty())
            && !aumids.contains(&aumid)
        {
            aumids.push(aumid);
        }
    }
    Target {
        pids,
        frame_hosts: pids_named(FRAME_HOST),
        aumids,
    }
}

/// Whether a window owned by `pid` is one of the target's: it belongs to a
/// target process, or it is a frame of ApplicationFrameHost whose AUMID is
/// non-empty and equals the AUMID of a target process.
fn belongs(system: &dyn WindowSystem, target: &Target, pid: u32, window: Hwnd) -> bool {
    target.pids.contains(&pid)
        || (target.frame_hosts.contains(&pid)
            && !target.aumids.is_empty()
            && system
                .window_aumid(window)
                .is_some_and(|aumid| !aumid.is_empty() && target.aumids.contains(&aumid)))
}

/// The visible windows to ask, as enumerated.
fn windows_of(system: &dyn WindowSystem, target: &Target) -> Vec<Hwnd> {
    let mut found = Vec::new();
    for window in system.top_windows() {
        if window.visible
            && !found.contains(&window.handle)
            && belongs(system, target, window.pid, window.handle)
        {
            found.push(window.handle);
        }
    }
    found
}

/// Whether any process of the target is still in the list.
fn still_running(system: &dyn WindowSystem, target: &Target) -> Result<bool, PcError> {
    Ok(system
        .processes()?
        .iter()
        .any(|p| target.pids.contains(&p.pid)))
}

/// Asks every window of `exe` to close and waits up to `wait` for them to go.
///
/// - `Closed`: every window is gone (and a desktop app's process too).
/// - `KeepsRunning`: the windows are gone but a desktop process is not.
/// - `StillRunning`: a window is still there after the wait (a dialog).
/// - `NoWindow`: the app has no window to ask (or is not running); nothing was
///   posted.
/// - An error when the process list cannot be read or no close could be posted.
pub fn close_exe(
    system: &dyn WindowSystem,
    exe: &str,
    wait: Duration,
) -> Result<CloseOutcome, PcError> {
    // Defence in depth: the gate never approves a protected process.
    if is_protected(exe) {
        return Err(PcError::Failed("that process is protected".into()));
    }
    let processes = system.processes()?;
    let target = target(system, &processes, exe);
    if target.pids.is_empty() {
        return Ok(CloseOutcome::NoWindow);
    }
    let windows = windows_of(system, &target);
    if windows.is_empty() {
        return Ok(CloseOutcome::NoWindow);
    }
    let mut failure = None;
    let mut posted = 0;
    for window in &windows {
        // The handle may have been reused since the enumeration: read the owner
        // again and skip a window that is no longer the target's.
        let owner = system.window_pid(*window);
        if !owner.is_some_and(|pid| belongs(system, &target, pid, *window)) {
            continue;
        }
        match system.post_close(*window) {
            Ok(()) => posted += 1,
            Err(why) => failure = Some(why),
        }
    }
    if posted == 0 {
        return match failure {
            Some(why) => Err(PcError::Failed(format!(
                "could not ask the app to close: {why}"
            ))),
            None => Ok(CloseOutcome::NoWindow),
        };
    }
    let packaged = !target.aumids.is_empty();
    let mut waited = Duration::ZERO;
    let mut windows_gone = false;
    while waited < wait {
        system.pause(CLOSE_POLL);
        waited += CLOSE_POLL;
        if !windows_gone {
            windows_gone = windows_of(system, &target).is_empty();
        }
        if windows_gone && (packaged || !still_running(system, &target)?) {
            return Ok(CloseOutcome::Closed);
        }
    }
    Ok(if windows_gone {
        CloseOutcome::KeepsRunning
    } else {
        CloseOutcome::StillRunning
    })
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

/// The real window system.
#[cfg(windows)]
pub struct RealWindows;

#[cfg(windows)]
impl WindowSystem for RealWindows {
    fn processes(&self) -> Result<Vec<ProcessEntry>, PcError> {
        super::win_apps::snapshot()
    }

    fn top_windows(&self) -> Vec<TopWindow> {
        use windows_sys::Win32::Foundation::{HWND, LPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
        };

        unsafe extern "system" fn collect(handle: HWND, found: LPARAM) -> i32 {
            // SAFETY: `found` is the address of the `Vec` `top_windows` passes
            // below, alive for the whole `EnumWindows` call.
            let found = unsafe { &mut *(found as *mut Vec<TopWindow>) };
            let mut pid = 0u32;
            // SAFETY: `pid` is a valid out pointer; `handle` came from Windows.
            unsafe { GetWindowThreadProcessId(handle, &mut pid) };
            found.push(TopWindow {
                handle: handle as Hwnd,
                pid,
                // SAFETY: plain query on a handle Windows just gave us.
                visible: unsafe { IsWindowVisible(handle) } != 0,
            });
            1
        }

        let mut found: Vec<TopWindow> = Vec::new();
        // SAFETY: the callback only touches `found`, which outlives the call.
        unsafe { EnumWindows(Some(collect), &mut found as *mut Vec<TopWindow> as LPARAM) };
        found
    }

    fn process_aumid(&self, pid: u32) -> Option<String> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        // SAFETY: a plain open of a process handle for a limited query.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return None;
        }
        let mut length = 256u32;
        let mut buffer = [0u16; 256];
        // SAFETY: `buffer` holds `length` UTF-16 units and both outlive the call.
        let status =
            unsafe { GetApplicationUserModelId(process, &mut length, buffer.as_mut_ptr()) };
        // SAFETY: `process` was opened above and is closed once.
        unsafe { CloseHandle(process) };
        // 0 is success; a plain desktop process answers APPMODEL_ERROR_NO_APPLICATION.
        if status != 0 || length == 0 {
            return None;
        }
        let text = String::from_utf16_lossy(&buffer[..(length as usize - 1).min(buffer.len())]);
        (!text.is_empty()).then_some(text)
    }

    fn window_aumid(&self, window: Hwnd) -> Option<String> {
        use windows::Win32::Foundation::{HWND, PROPERTYKEY};
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
        use windows::Win32::UI::Shell::PropertiesSystem::{
            IPropertyStore, SHGetPropertyStoreForWindow,
        };
        use windows::core::GUID;

        // PKEY_AppUserModel_ID: {9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, 5.
        const KEY: PROPERTYKEY = PROPERTYKEY {
            fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
            pid: 5,
        };
        // One COM bracket per call, like every other adapter.
        let com = super::com::ComGuard::apartment();
        if !com.is_usable() {
            return None;
        }
        // SAFETY: the handle is a plain value Windows gave us; a stale one makes
        // the call fail, which is `None`.
        let store: IPropertyStore =
            unsafe { SHGetPropertyStoreForWindow(HWND(window as *mut _)) }.ok()?;
        // SAFETY: `KEY` outlives the call. The returned PROPVARIANT clears itself
        // when dropped at the end of this function.
        let value = unsafe { store.GetValue(&KEY) }.ok()?;
        // SAFETY: `value` is a valid PROPVARIANT; the allocated string is read
        // once and freed with CoTaskMemFree.
        let text = unsafe {
            let wide = PropVariantToStringAlloc(&value).ok()?;
            let text = wide.to_string().ok();
            CoTaskMemFree(Some(wide.0 as *const _));
            text
        }?;
        (!text.is_empty()).then_some(text)
    }

    fn window_pid(&self, window: Hwnd) -> Option<u32> {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

        let mut pid = 0u32;
        // SAFETY: `pid` is a valid out pointer; a dead handle returns 0.
        let thread = unsafe { GetWindowThreadProcessId(window as HWND, &mut pid) };
        (thread != 0 && pid != 0).then_some(pid)
    }

    fn post_close(&self, window: Hwnd) -> Result<(), String> {
        use windows_sys::Win32::Foundation::{GetLastError, HWND};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

        // SAFETY: a plain post of WM_CLOSE (no pointer in either parameter).
        let posted = unsafe { PostMessageW(window as HWND, WM_CLOSE, 0, 0) };
        if posted != 0 {
            Ok(())
        } else {
            // SAFETY: reads the calling thread's last error.
            Err(format!("PostMessageW failed (error {})", unsafe {
                GetLastError()
            }))
        }
    }

    fn pause(&self, how_long: Duration) {
        std::thread::sleep(how_long);
    }
}

/// Real windows, real wait: what [`WinApps`](super::win_apps) closes with.
#[cfg(windows)]
pub fn close(exe: &str) -> Result<CloseOutcome, PcError> {
    close_exe(&RealWindows, exe, CLOSE_WAIT)
}

#[cfg(all(test, windows))]
mod real_tests {
    //! Read-only looks at the real system. Nothing here posts a message.

    use super::*;

    #[test]
    #[ignore = "reads the real window list; run by hand"]
    fn the_real_window_list_has_windows_and_the_snapshot_has_this_process() {
        let system = RealWindows;
        let windows = system.top_windows();
        assert!(
            windows.iter().any(|w| w.visible),
            "{} windows",
            windows.len()
        );
        let processes = system.processes().expect("the snapshot is readable");
        let own = std::process::id();
        assert!(processes.iter().any(|p| p.pid == own && p.in_user_session));
    }

    fn class_of(handle: Hwnd) -> String {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW;

        let mut buffer = [0u16; 128];
        // SAFETY: the buffer length is passed.
        let length = unsafe { GetClassNameW(handle as HWND, buffer.as_mut_ptr(), 128) };
        String::from_utf16_lossy(&buffer[..length.max(0) as usize])
    }

    #[test]
    #[ignore = "reads the real window list; run by hand"]
    fn every_open_application_frame_window_has_an_app_user_model_id() {
        let system = RealWindows;
        let processes = system.processes().expect("the snapshot is readable");
        let hosts: Vec<u32> = processes
            .iter()
            .filter(|p| file_name(&p.exe_name) == FRAME_HOST)
            .map(|p| p.pid)
            .collect();
        assert!(!hosts.is_empty(), "ApplicationFrameHost runs on a desktop");
        let mut frames = Vec::new();
        let mut without = Vec::new();
        for window in system.top_windows() {
            if window.visible
                && hosts.contains(&window.pid)
                && class_of(window.handle) == "ApplicationFrameWindow"
            {
                let aumid = system.window_aumid(window.handle);
                frames.push((window.handle, aumid.clone()));
                if aumid.is_none() {
                    without.push(window.handle);
                }
            }
        }
        eprintln!("open frames (handle, aumid): {frames:?}");
        assert!(without.is_empty(), "frames with no AUMID: {without:?}");
    }

    #[test]
    #[ignore = "reads the real process list; run by hand"]
    fn a_plain_desktop_process_has_no_aumid() {
        assert_eq!(RealWindows.process_aumid(std::process::id()), None);
    }
}
