//! `close_exe` over a fake window system: which windows are asked to close,
//! what is never touched, and what the wait reports.

use std::sync::Mutex;
use std::time::Duration;

use niva_core::ports::{CloseOutcome, PcError};

use super::*;
use crate::adapters::win_apps::ProcessEntry;

fn process(pid: u32, exe: &str) -> ProcessEntry {
    ProcessEntry {
        pid,
        exe_name: exe.into(),
        in_user_session: true,
    }
}

fn window(handle: Hwnd, pid: u32) -> TopWindow {
    TopWindow {
        handle,
        pid,
        visible: true,
    }
}

/// What `post_close` does with a window.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OnClose {
    /// The window goes away (the app closes it).
    Vanishes,
    /// The window stays (an unsaved-changes dialog is open).
    Stays,
    /// The window goes away only after this many pauses.
    After(usize),
    /// `PostMessage` itself fails.
    Refuses,
}

struct FakeSystem {
    processes: Mutex<Result<Vec<ProcessEntry>, PcError>>,
    windows: Mutex<Vec<TopWindow>>,
    /// Process id -> its AppUserModelID.
    process_aumids: Vec<(u32, String)>,
    /// Frame handle -> its AppUserModelID property.
    frame_aumids: Vec<(Hwnd, String)>,
    on_close: OnClose,
    /// Windows that ignore the close whatever `on_close` says.
    stay: Vec<Hwnd>,
    /// Desktop processes keep running after their last window is gone.
    lingering: bool,
    /// `window_pid` answers this instead of the listed owner (a reused handle).
    reassigned: Mutex<Vec<(Hwnd, u32)>>,
    posted: Mutex<Vec<Hwnd>>,
    /// Windows that were asked to close and after how many pauses they leave.
    leaving: Mutex<Vec<(Hwnd, usize)>>,
    paused: Mutex<Vec<Duration>>,
}

impl FakeSystem {
    fn new(processes: Vec<ProcessEntry>, windows: Vec<TopWindow>) -> FakeSystem {
        FakeSystem {
            processes: Mutex::new(Ok(processes)),
            windows: Mutex::new(windows),
            process_aumids: Vec::new(),
            frame_aumids: Vec::new(),
            on_close: OnClose::Vanishes,
            stay: Vec::new(),
            lingering: false,
            reassigned: Mutex::default(),
            posted: Mutex::default(),
            leaving: Mutex::default(),
            paused: Mutex::default(),
        }
    }

    fn aumids(mut self, process: Vec<(u32, &str)>, frames: Vec<(Hwnd, &str)>) -> FakeSystem {
        self.process_aumids = process.into_iter().map(|(p, a)| (p, a.into())).collect();
        self.frame_aumids = frames.into_iter().map(|(h, a)| (h, a.into())).collect();
        self
    }

    fn on_close(mut self, on_close: OnClose) -> FakeSystem {
        self.on_close = on_close;
        self
    }

    fn staying(mut self, stay: Vec<Hwnd>) -> FakeSystem {
        self.stay = stay;
        self
    }

    fn lingering(mut self) -> FakeSystem {
        self.lingering = true;
        self
    }

    fn posted(&self) -> Vec<Hwnd> {
        let mut posted = self.posted.lock().unwrap().clone();
        posted.sort_unstable();
        posted
    }

    fn pauses(&self) -> usize {
        self.paused.lock().unwrap().len()
    }

    fn waited(&self) -> Duration {
        self.paused.lock().unwrap().iter().sum()
    }
}

impl WindowSystem for FakeSystem {
    fn processes(&self) -> Result<Vec<ProcessEntry>, PcError> {
        self.processes.lock().unwrap().clone()
    }

    fn top_windows(&self) -> Vec<TopWindow> {
        self.windows.lock().unwrap().clone()
    }

    fn process_aumid(&self, pid: u32) -> Option<String> {
        self.process_aumids
            .iter()
            .find(|(p, _)| *p == pid)
            .map(|(_, a)| a.clone())
    }

    fn window_aumid(&self, window: Hwnd) -> Option<String> {
        self.frame_aumids
            .iter()
            .find(|(h, _)| *h == window)
            .map(|(_, a)| a.clone())
    }

    fn window_pid(&self, window: Hwnd) -> Option<u32> {
        if let Some((_, pid)) = self
            .reassigned
            .lock()
            .unwrap()
            .iter()
            .find(|(h, _)| *h == window)
        {
            return Some(*pid);
        }
        self.windows
            .lock()
            .unwrap()
            .iter()
            .find(|w| w.handle == window)
            .map(|w| w.pid)
    }

    fn post_close(&self, handle: Hwnd) -> Result<(), String> {
        self.posted.lock().unwrap().push(handle);
        if self.stay.contains(&handle) {
            return Ok(());
        }
        match self.on_close {
            OnClose::Refuses => return Err("access denied".into()),
            OnClose::Vanishes => self.leaving.lock().unwrap().push((handle, 0)),
            OnClose::After(pauses) => self.leaving.lock().unwrap().push((handle, pauses)),
            OnClose::Stays => {}
        }
        Ok(())
    }

    fn pause(&self, how_long: Duration) {
        self.paused.lock().unwrap().push(how_long);
        let pauses = self.paused.lock().unwrap().len();
        let mut windows = self.windows.lock().unwrap();
        for (handle, after) in self.leaving.lock().unwrap().iter() {
            if pauses >= *after {
                windows.retain(|w| w.handle != *handle);
            }
        }
        // A desktop process exits with its last window, unless it lingers.
        if !self.lingering {
            let alive: Vec<u32> = windows
                .iter()
                .filter(|w| w.visible)
                .map(|w| w.pid)
                .collect();
            if let Ok(processes) = self.processes.lock().unwrap().as_mut() {
                processes.retain(|p| {
                    alive.contains(&p.pid)
                        || p.exe_name.eq_ignore_ascii_case(FRAME_HOST)
                        || self.process_aumids.iter().any(|(pid, _)| *pid == p.pid)
                });
            }
        }
    }
}

const WAIT: Duration = Duration::from_secs(3);

fn close(system: &FakeSystem, exe: &str) -> Result<CloseOutcome, PcError> {
    close_exe(system, exe, WAIT)
}

// --- desktop apps -------------------------------------------------------------

#[test]
fn every_visible_top_level_window_of_the_app_is_asked_to_close_and_nothing_else() {
    let mut hidden = window(12, 10);
    hidden.visible = false;
    let system = FakeSystem::new(
        vec![process(10, "Spotify.exe"), process(20, "notepad.exe")],
        vec![
            window(11, 10),
            hidden,
            window(13, 10),
            window(21, 20), // another app
        ],
    );
    assert_eq!(close(&system, "Spotify.exe"), Ok(CloseOutcome::Closed));
    assert_eq!(system.posted(), [11, 13]);
}

#[test]
fn an_app_with_several_processes_has_all_their_windows_closed() {
    let system = FakeSystem::new(
        vec![
            process(10, "chrome.exe"),
            process(11, "chrome.exe"),
            process(12, "chrome.exe"),
        ],
        vec![window(1, 10), window(2, 12), window(3, 99)],
    );
    assert_eq!(close(&system, "chrome.exe"), Ok(CloseOutcome::Closed));
    assert_eq!(system.posted(), [1, 2]);
}

#[test]
fn the_exe_name_is_matched_whole_and_without_regard_to_case() {
    let system = FakeSystem::new(
        vec![
            process(10, "Spotify.exe"),
            process(20, "SpotifyWebHelper.exe"),
            process(30, "notspotify.exe"),
        ],
        vec![window(1, 10), window(2, 20), window(3, 30)],
    );
    assert_eq!(close(&system, "spotify.EXE"), Ok(CloseOutcome::Closed));
    assert_eq!(system.posted(), [1]);
}

#[test]
fn a_process_outside_the_users_session_is_never_touched() {
    let mut other = process(20, "Spotify.exe");
    other.in_user_session = false;
    let system = FakeSystem::new(
        vec![process(10, "Spotify.exe"), other],
        vec![window(1, 10), window(2, 20)],
    );
    assert_eq!(close(&system, "Spotify.exe"), Ok(CloseOutcome::Closed));
    assert_eq!(system.posted(), [1]);
}

// --- UWP apps, matched by AppUserModelID ---------------------------------------

const CALC: &str = "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App";
const SETTINGS: &str =
    "windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel";

/// Calculator is `CalculatorApp.exe` (pid 100) with no top-level window of its
/// own. Its window is a frame owned by ApplicationFrameHost (pid 50) whose
/// AUMID property equals the AUMID of pid 100.
fn uwp_system() -> FakeSystem {
    FakeSystem::new(
        vec![
            process(100, "CalculatorApp.exe"),
            process(200, "SystemSettings.exe"),
            process(50, "ApplicationFrameHost.exe"),
            process(60, "explorer.exe"),
        ],
        vec![
            window(501, 50), // the Calculator frame
            window(502, 50), // the frame of another app
            window(503, 50), // a frame with no AUMID
            window(601, 60), // the shell
        ],
    )
    .aumids(
        vec![(100, CALC), (200, SETTINGS)],
        vec![(501, CALC), (502, SETTINGS)],
    )
}

#[test]
fn a_uwp_app_is_closed_through_its_frame_and_only_that_frame() {
    let system = uwp_system();
    assert_eq!(
        close(&system, "CalculatorApp.exe"),
        Ok(CloseOutcome::Closed)
    );
    assert_eq!(system.posted(), [501]);
}

#[test]
fn a_frame_of_another_app_or_without_an_aumid_is_never_touched() {
    let system = uwp_system();
    assert_eq!(
        close(&system, "SystemSettings.exe"),
        Ok(CloseOutcome::Closed)
    );
    assert_eq!(system.posted(), [502], "not 501, not the bare frame 503");
}

#[test]
fn a_minimized_app_whose_process_is_suspended_is_closed_through_its_frame() {
    // The frame stays, there is no CoreWindow to look for: the AUMID still
    // matches. Closing the frame is enough (the process may end on its own).
    let system = uwp_system().on_close(OnClose::After(4));
    assert_eq!(
        close(&system, "CalculatorApp.exe"),
        Ok(CloseOutcome::Closed)
    );
    assert_eq!(system.posted(), [501]);
}

#[test]
fn an_empty_aumid_never_matches() {
    let system = FakeSystem::new(
        vec![
            process(100, "CalculatorApp.exe"),
            process(50, "ApplicationFrameHost.exe"),
        ],
        vec![window(501, 50)],
    )
    .aumids(vec![(100, "")], vec![(501, "")]);
    assert_eq!(
        close(&system, "CalculatorApp.exe"),
        Ok(CloseOutcome::NoWindow)
    );
    assert!(system.posted().is_empty());
}

#[test]
fn a_process_with_no_aumid_has_no_frame_route() {
    let system = FakeSystem::new(
        vec![
            process(300, "BackgroundApp.exe"),
            process(50, "ApplicationFrameHost.exe"),
        ],
        vec![window(501, 50)],
    )
    .aumids(vec![], vec![(501, CALC)]);
    assert_eq!(
        close(&system, "BackgroundApp.exe"),
        Ok(CloseOutcome::NoWindow)
    );
    assert!(system.posted().is_empty());
}

#[test]
fn only_a_window_of_application_frame_host_counts_as_a_frame() {
    let system = FakeSystem::new(
        vec![
            process(100, "CalculatorApp.exe"),
            process(77, "other.exe"),
            process(50, "ApplicationFrameHost.exe"),
        ],
        vec![window(701, 77)],
    )
    .aumids(vec![(100, CALC)], vec![(701, CALC)]);
    assert_eq!(
        close(&system, "CalculatorApp.exe"),
        Ok(CloseOutcome::NoWindow)
    );
    assert!(system.posted().is_empty());
}

#[test]
fn a_frame_host_of_another_session_is_not_a_route() {
    let mut host = process(50, "ApplicationFrameHost.exe");
    host.in_user_session = false;
    let system = FakeSystem::new(
        vec![process(100, "CalculatorApp.exe"), host],
        vec![window(501, 50)],
    )
    .aumids(vec![(100, CALC)], vec![(501, CALC)]);
    assert_eq!(
        close(&system, "CalculatorApp.exe"),
        Ok(CloseOutcome::NoWindow)
    );
    assert!(system.posted().is_empty());
}

#[test]
fn application_frame_host_is_never_a_target_itself() {
    let system = uwp_system();
    for exe in [
        "ApplicationFrameHost.exe",
        "applicationframehost",
        r"C:\Windows\System32\ApplicationFrameHost.exe",
    ] {
        assert!(
            matches!(close(&system, exe), Err(PcError::Failed(_))),
            "{exe}"
        );
    }
    assert!(system.posted().is_empty());
}

// --- a handle reused between the listing and the post ---------------------------

#[test]
fn a_window_that_changed_owner_before_the_post_is_skipped() {
    let system = FakeSystem::new(
        vec![process(10, "Spotify.exe"), process(20, "notepad.exe")],
        vec![window(1, 10), window(2, 10)],
    );
    system.reassigned.lock().unwrap().push((2, 20));
    let result = close(&system, "Spotify.exe");
    assert_eq!(system.posted(), [1], "window 2 now belongs to notepad");
    assert!(result.is_ok());
}

#[test]
fn a_window_that_is_gone_before_the_post_is_skipped_and_nothing_is_posted() {
    let system = FakeSystem::new(vec![process(10, "Spotify.exe")], vec![window(1, 10)]);
    system.reassigned.lock().unwrap().clear();
    // The owner lookup finds no window: simulate by removing it after listing.
    struct Gone<'a>(&'a FakeSystem);
    impl WindowSystem for Gone<'_> {
        fn processes(&self) -> Result<Vec<ProcessEntry>, PcError> {
            self.0.processes()
        }
        fn top_windows(&self) -> Vec<TopWindow> {
            self.0.top_windows()
        }
        fn process_aumid(&self, pid: u32) -> Option<String> {
            self.0.process_aumid(pid)
        }
        fn window_aumid(&self, w: Hwnd) -> Option<String> {
            self.0.window_aumid(w)
        }
        fn window_pid(&self, _: Hwnd) -> Option<u32> {
            None
        }
        fn post_close(&self, w: Hwnd) -> Result<(), String> {
            self.0.post_close(w)
        }
        fn pause(&self, d: Duration) {
            self.0.pause(d)
        }
    }
    assert_eq!(
        close_exe(&Gone(&system), "Spotify.exe", WAIT),
        Ok(CloseOutcome::NoWindow)
    );
    assert!(system.posted().is_empty());
}

#[test]
fn a_frame_whose_aumid_changed_before_the_post_is_skipped() {
    // The re-check for a frame uses the AUMID again: a reused handle whose
    // frame now belongs to another app is not closed.
    let system = uwp_system();
    // Same handle, now owned by a window that is not a frame of the host.
    system.reassigned.lock().unwrap().push((501, 60));
    assert_eq!(
        close(&system, "CalculatorApp.exe"),
        Ok(CloseOutcome::NoWindow)
    );
    assert!(system.posted().is_empty());
}

// --- protected processes and unreadable snapshots ------------------------------

#[test]
fn a_protected_process_is_refused_before_anything_is_listed_or_posted() {
    let system = FakeSystem::new(
        vec![process(60, "explorer.exe"), process(70, "niva.exe")],
        vec![window(601, 60), window(701, 70)],
    );
    for exe in ["explorer.exe", "Niva.exe", "ollama.exe", "dwm.exe"] {
        assert!(
            matches!(close(&system, exe), Err(PcError::Failed(_))),
            "{exe}"
        );
    }
    assert!(system.posted().is_empty());
    assert_eq!(system.pauses(), 0);
}

#[test]
fn an_unreadable_process_list_is_an_error_and_posts_nothing() {
    let system = FakeSystem::new(Vec::new(), vec![window(1, 10)]);
    *system.processes.lock().unwrap() = Err(PcError::Failed("snapshot failed".into()));
    assert_eq!(
        close(&system, "Spotify.exe"),
        Err(PcError::Failed("snapshot failed".into()))
    );
    assert!(system.posted().is_empty());
}

// --- no window, wait, still running ---------------------------------------------

#[test]
fn an_app_that_is_not_running_or_has_no_window_reports_no_window() {
    let system = FakeSystem::new(vec![process(20, "notepad.exe")], vec![window(2, 20)]);
    assert_eq!(close(&system, "Spotify.exe"), Ok(CloseOutcome::NoWindow));
    let system = FakeSystem::new(vec![process(10, "Spotify.exe")], Vec::new());
    assert_eq!(close(&system, "Spotify.exe"), Ok(CloseOutcome::NoWindow));
    let mut hidden = window(1, 10);
    hidden.visible = false;
    let system = FakeSystem::new(vec![process(10, "Spotify.exe")], vec![hidden]);
    assert_eq!(close(&system, "Spotify.exe"), Ok(CloseOutcome::NoWindow));
    assert!(system.posted().is_empty());
    assert_eq!(system.pauses(), 0, "nothing to wait for");
}

#[test]
fn windows_that_stay_for_the_whole_wait_report_still_running_after_three_seconds() {
    let system = FakeSystem::new(vec![process(10, "notepad.exe")], vec![window(1, 10)])
        .on_close(OnClose::Stays);
    assert_eq!(
        close(&system, "notepad.exe"),
        Ok(CloseOutcome::StillRunning)
    );
    assert_eq!(system.waited(), WAIT, "waits the whole time, not more");
    assert_eq!(
        system.posted(),
        [1],
        "the close was asked once, not repeated"
    );
}

#[test]
fn the_wait_ends_as_soon_as_the_windows_and_the_process_are_gone() {
    let system = FakeSystem::new(vec![process(10, "notepad.exe")], vec![window(1, 10)])
        .on_close(OnClose::After(5));
    assert_eq!(close(&system, "notepad.exe"), Ok(CloseOutcome::Closed));
    assert_eq!(system.pauses(), 5);
    assert!(system.waited() < WAIT);
}

#[test]
fn a_window_that_closes_only_after_the_wait_is_still_running() {
    let system = FakeSystem::new(vec![process(10, "notepad.exe")], vec![window(1, 10)])
        .on_close(OnClose::After(1000));
    assert_eq!(
        close(&system, "notepad.exe"),
        Ok(CloseOutcome::StillRunning)
    );
}

#[test]
fn one_window_closing_and_another_staying_is_still_running() {
    let system = FakeSystem::new(
        vec![process(10, "notepad.exe")],
        vec![window(1, 10), window(2, 10)],
    )
    .staying(vec![2]);
    assert_eq!(
        close(&system, "notepad.exe"),
        Ok(CloseOutcome::StillRunning)
    );
    assert_eq!(system.posted(), [1, 2]);
}

// --- the window went, the process did not ---------------------------------------

#[test]
fn a_desktop_process_that_outlives_its_window_is_not_said_to_be_closed() {
    let system = FakeSystem::new(vec![process(10, "Spotify.exe")], vec![window(1, 10)]).lingering();
    assert_eq!(
        close(&system, "Spotify.exe"),
        Ok(CloseOutcome::KeepsRunning)
    );
    assert_eq!(
        system.waited(),
        WAIT,
        "it waited the whole budget for the process"
    );
}

#[test]
fn a_uwp_app_whose_frame_is_gone_counts_as_closed_even_if_the_process_stays() {
    let system = uwp_system().lingering();
    assert_eq!(
        close(&system, "CalculatorApp.exe"),
        Ok(CloseOutcome::Closed)
    );
}

#[test]
fn an_unreadable_process_list_while_checking_the_process_is_an_error() {
    // Windows gone, then the snapshot fails: no claim either way.
    struct Flaky(FakeSystem, Mutex<usize>);
    impl WindowSystem for Flaky {
        fn processes(&self) -> Result<Vec<ProcessEntry>, PcError> {
            let mut calls = self.1.lock().unwrap();
            *calls += 1;
            if *calls > 1 {
                Err(PcError::Failed("snapshot failed".into()))
            } else {
                self.0.processes()
            }
        }
        fn top_windows(&self) -> Vec<TopWindow> {
            self.0.top_windows()
        }
        fn process_aumid(&self, pid: u32) -> Option<String> {
            self.0.process_aumid(pid)
        }
        fn window_aumid(&self, w: Hwnd) -> Option<String> {
            self.0.window_aumid(w)
        }
        fn window_pid(&self, w: Hwnd) -> Option<u32> {
            self.0.window_pid(w)
        }
        fn post_close(&self, w: Hwnd) -> Result<(), String> {
            self.0.post_close(w)
        }
        fn pause(&self, d: Duration) {
            self.0.pause(d)
        }
    }
    let system = Flaky(
        FakeSystem::new(vec![process(10, "Spotify.exe")], vec![window(1, 10)]).lingering(),
        Mutex::new(0),
    );
    assert_eq!(
        close_exe(&system, "Spotify.exe", WAIT),
        Err(PcError::Failed("snapshot failed".into()))
    );
}

// --- failures of the post itself -----------------------------------------------

#[test]
fn when_every_post_fails_the_close_is_an_error_not_a_wait() {
    let system = FakeSystem::new(vec![process(10, "notepad.exe")], vec![window(1, 10)])
        .on_close(OnClose::Refuses);
    let result = close(&system, "notepad.exe");
    assert!(matches!(result, Err(PcError::Failed(_))), "{result:?}");
    assert_eq!(
        system.pauses(),
        0,
        "no 3 s wait for a close that was never sent"
    );
}

#[test]
fn the_default_wait_is_three_seconds_in_tenth_of_a_second_steps() {
    assert_eq!(CLOSE_WAIT, Duration::from_secs(3));
    assert_eq!(CLOSE_POLL, Duration::from_millis(100));
}

#[test]
fn the_windows_part_no_longer_looks_for_a_core_window() {
    let code: String = include_str!("../win_close.rs")
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!code.contains(&["Core", "Window"].concat()));
    assert!(code.contains("GetApplicationUserModelId"));
    assert!(code.contains("SHGetPropertyStoreForWindow"));
}
