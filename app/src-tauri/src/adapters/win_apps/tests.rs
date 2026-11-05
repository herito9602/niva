//! The app index, the fuzzy match and `open_app`, over fakes. Nothing here
//! starts a program or runs PowerShell (the real-Windows tests are `#[ignore]`d
//! at the end of this file).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::aliases::alias_terms;
use super::*;

fn entry(name: &str, id: &str) -> AppEntry {
    AppEntry {
        name: name.into(),
        id: id.into(),
    }
}

fn menu() -> Vec<AppEntry> {
    vec![
        entry("Notepad", "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
        entry("Notepad++", r"C:\Program Files\Notepad++\notepad++.exe"),
        entry(
            "Calculator",
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
        ),
        entry("Spotify", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
        entry(
            "Visual Studio Code",
            r"C:\Users\u\AppData\Local\Programs\Code\Code.exe",
        ),
        entry("Microsoft Edge", "MSEdge"),
        entry(
            "Word",
            r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE",
        ),
        entry(
            "WordPad",
            r"C:\Program Files\Windows NT\Accessories\wordpad.exe",
        ),
        entry("Uninstall Spotify", r"C:\Spotify\uninstall.exe"),
        entry("Calculadora", "Microsoft.WindowsCalculator.es!App"),
        entry("Bloc de notas", "Microsoft.WindowsNotepad.es!App"),
    ]
}

fn found(query: &str) -> Option<String> {
    find_app(&menu(), query).map(|e| e.name.clone())
}

// --- parsing Get-StartApps output ---------------------------------------------

#[test]
fn an_array_of_entries_is_parsed() {
    let json = r#"[{"Name":"Notepad","AppID":"A!App"},{"Name":"Calculator","AppID":"B!App"}]"#;
    assert_eq!(
        parse_start_apps(json).unwrap(),
        vec![entry("Notepad", "A!App"), entry("Calculator", "B!App")]
    );
}

#[test]
fn a_single_object_is_one_entry() {
    // ConvertTo-Json does not wrap a one-item list in an array.
    let json = r#"{"Name":"Notepad","AppID":"A!App"}"#;
    assert_eq!(
        parse_start_apps(json).unwrap(),
        vec![entry("Notepad", "A!App")]
    );
}

#[test]
fn a_byte_order_mark_and_accents_survive() {
    let json = "\u{feff}[{\"Name\":\"Cámara\",\"AppID\":\"C!App\"}]";
    assert_eq!(
        parse_start_apps(json).unwrap(),
        vec![entry("Cámara", "C!App")]
    );
}

#[test]
fn unusable_entries_are_dropped() {
    let json = r#"[{"Name":"","AppID":"A"},{"Name":"NoId","AppID":""},
        {"Name":"Ctl","AppID":"bad\u0007id"},{"Name":"Null","AppID":null},{"Name":"Ok","AppID":"ok!App"}]"#;
    assert_eq!(parse_start_apps(json).unwrap(), vec![entry("Ok", "ok!App")]);
}

#[test]
fn garbage_or_an_empty_list_is_an_error_so_the_index_is_not_ready() {
    assert!(parse_start_apps("").is_err());
    assert!(parse_start_apps("not json").is_err());
    assert!(parse_start_apps("[]").is_err());
    assert!(parse_start_apps(r#"[{"Name":"","AppID":""}]"#).is_err());
}

/// `Get-StartApps | ConvertTo-Json` for these (name, AppID) pairs.
fn start_apps_json(pairs: &[(&str, &str)]) -> String {
    serde_json::Value::Array(
        pairs
            .iter()
            .map(|(name, id)| serde_json::json!({ "Name": name, "AppID": id }))
            .collect(),
    )
    .to_string()
}

/// Entries as this PC's `Get-StartApps` really reports them (read-only look at
/// the 286 entries, PR 2d-a review): packaged apps (`Family!App`), desktop
/// AUMIDs with no `!` (`Chrome`, `MSEdge`, `com.squirrel.*`), `.exe` targets
/// under a known-folder GUID or a drive path, and everything that is not an app.
const LAUNCHABLE: &[(&str, &str)] = &[
    ("Notepad", "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
    (
        "Calculator",
        "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
    ),
    ("Spotify", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
    ("Google Chrome", "Chrome"),
    ("Microsoft Edge", "MSEdge"),
    ("Firefox", "308046B0AF4A39CB"),
    ("Discord", "com.squirrel.Discord.Discord"),
    ("Word", "Microsoft.Office.WINWORD.EXE.15"),
    ("Explorador de archivos", "Microsoft.Windows.Explorer"),
    ("Panel de control", "Microsoft.Windows.ControlPanel"),
    (
        "Notepad++",
        r"{6D809377-6AF0-444B-8957-A3773F02200E}\Notepad++\notepad++.exe",
    ),
    (
        "HWiNFO 64",
        r"{6D809377-6AF0-444B-8957-A3773F02200E}\HWiNFO64\HWiNFO64.EXE",
    ),
    (
        "Ollama",
        r"C:\Users\u\AppData\Local\Programs\Ollama\ollama app.exe",
    ),
    (
        "Editor del Registro",
        r"{F38BF404-1D43-42F2-9305-67DE0B28FC23}\regedit.exe",
    ),
];

const NOT_LAUNCHABLE: &[(&str, &str)] = &[
    // scripts, URLs, documents, folders, settings files
    (
        "Log KMSpico",
        r"{6D809377-6AF0-444B-8957-A3773F02200E}\KMSpico\scripts\Log.cmd",
    ),
    ("Setup helper", r"C:\Tools\setup.bat"),
    ("Profile", r"C:\Tools\profile.ps1"),
    ("Get Help", "https://java.com/help"),
    ("Site", "http://www.soapui.org"),
    (
        "4GMIFI",
        r"{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}\4GMIFI\4GMIFI.url",
    ),
    (
        "7-Zip Help",
        r"{6D809377-6AF0-444B-8957-A3773F02200E}\7-Zip\7-zip.chm",
    ),
    (
        "HWiNFO Manual",
        r"{6D809377-6AF0-444B-8957-A3773F02200E}\HWiNFO64\HWiNFO Manual.pdf",
    ),
    (
        "Edit CPU-Z Config File",
        r"{6D809377-6AF0-444B-8957-A3773F02200E}\CPUID\CPU-Z\cpuz.ini",
    ),
    (
        "Git Release Notes",
        r"{6D809377-6AF0-444B-8957-A3773F02200E}\Git\ReleaseNotes.html",
    ),
    (
        "Release Notes",
        r"{6D809377-6AF0-444B-8957-A3773F02200E}\VideoLAN\VLC\NEWS.txt",
    ),
    (
        "Servicios",
        r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\services.msc",
    ),
    (
        "Samples",
        r"{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}\MSI Afterburner\SDK\Samples",
    ),
    ("Este equipo", "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"),
    (
        "anyPDF",
        r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu\Programs",
    ),
    // shortcuts with arguments / unknown target (no AUMID of their own)
    (
        "Git CMD",
        "Microsoft.AutoGenerated.{29591C0F-7C09-B655-C0EB-8544FCA7D1DE}",
    ),
    (
        "VLC media player - reset preferences and cache files",
        "Microsoft.AutoGenerated.{51325390-AE6A-68FC-A315-0950CC83A166}",
    ),
    // uninstallers, by name (any word) or by AppID
    (
        "Uninstall",
        r"{6D809377-6AF0-444B-8957-A3773F02200E}\DBeaver\Uninstall.exe",
    ),
    (
        "Desinstalar",
        r"{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}\LagoFast\Uninstall.exe",
    ),
    (
        "Remove Spotify (uninstall)",
        "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Remove",
    ),
    (
        "Uninstall Driver",
        r"{6D809377-6AF0-444B-8957-A3773F02200E}\MediaTek\SP Driver\unins000.exe",
    ),
    (
        "Driver cleanup",
        r"{6D809377-6AF0-444B-8957-A3773F02200E}\MediaTek\SP Driver\unins000.exe",
    ),
    ("Cleaner", r"C:\Program Files\Cleaner\uninstall.exe"),
    // maintenance entries by a word of the name
    ("Reset Preferences", "Some.App_abc!Reset"),
    ("Reparar Office", "Some.Office_abc!Repair"),
    // not a bare identifier / not a known path shape
    ("Spaced", "not an id"),
    ("Remote", r"\\server\share\tool.exe"),
    ("Relative", r"..\tool.exe"),
    ("Dotted", r"C:\Tools\..\Windows\tool.exe"),
    ("Odd scheme", "file:///C:/Windows/notepad.exe"),
];

#[test]
fn only_launchable_apps_survive_the_real_start_menu_shapes() {
    let mut all: Vec<(&str, &str)> = LAUNCHABLE.to_vec();
    all.extend_from_slice(NOT_LAUNCHABLE);
    let kept = parse_start_apps(&start_apps_json(&all)).unwrap();
    let names: Vec<&str> = kept.iter().map(|e| e.name.as_str()).collect();
    for (name, id) in LAUNCHABLE {
        assert!(names.contains(name), "{name} ({id}) must be kept");
    }
    for (name, id) in NOT_LAUNCHABLE {
        assert!(!names.contains(name), "{name} ({id}) must be dropped");
    }
}

#[test]
fn a_menu_with_no_launchable_entry_is_an_error() {
    assert!(parse_start_apps(&start_apps_json(NOT_LAUNCHABLE)).is_err());
}

#[test]
fn the_index_itself_refuses_what_the_parser_would_drop() {
    // Defence in depth: a source other than PowerShell cannot slip a script in.
    let index = AppIndex::new();
    let mixed = vec![
        entry("Real", "Real.App_abc!App"),
        entry("Script", r"C:\Tools\run.cmd"),
    ];
    assert_eq!(
        index.refresh_from(&FakeSource::with(vec![Ok(mixed)])),
        Ok(1)
    );
    let only_script = vec![entry("Script", r"C:\Tools\run.cmd")];
    let index = AppIndex::new();
    assert!(
        index
            .refresh_from(&FakeSource::with(vec![Ok(only_script)]))
            .is_err()
    );
    assert!(!index.is_ready());
}

// --- normalising ---------------------------------------------------------------

#[test]
fn normalizing_ignores_case_accents_and_punctuation() {
    assert_eq!(normalize("  Cámara   De-Fotos! "), "camara de fotos");
    assert_eq!(normalize("Notepad++"), "notepad");
    assert_eq!(normalize("ÑANDÚ"), "nandu");
    assert_eq!(normalize("***"), "");
}

// --- the fuzzy match -----------------------------------------------------------

#[test]
fn an_exact_name_wins_whatever_the_case() {
    assert_eq!(found("notepad").as_deref(), Some("Notepad"));
    assert_eq!(found("SPOTIFY").as_deref(), Some("Spotify"));
    assert_eq!(
        found("word").as_deref(),
        Some("Word"),
        "exact beats WordPad"
    );
}

#[test]
fn a_prefix_picks_the_shortest_name() {
    assert_eq!(found("note").as_deref(), Some("Notepad"));
    assert_eq!(found("spot").as_deref(), Some("Spotify"));
}

#[test]
fn a_word_inside_a_longer_name_matches() {
    assert_eq!(found("studio code").as_deref(), Some("Visual Studio Code"));
    assert_eq!(found("edge").as_deref(), Some("Microsoft Edge"));
}

#[test]
fn a_query_that_wraps_a_name_matches_it() {
    assert_eq!(found("spotify app").as_deref(), Some("Spotify"));
    assert_eq!(
        found("microsoft edge browser").as_deref(),
        Some("Microsoft Edge")
    );
}

#[test]
fn accents_do_not_matter() {
    assert_eq!(found("Bloc de nótas").as_deref(), Some("Bloc de notas"));
}

#[test]
fn a_small_typo_still_finds_the_app() {
    assert_eq!(found("calculater").as_deref(), Some("Calculator"));
    assert_eq!(found("spotfy").as_deref(), Some("Spotify"));
}

#[test]
fn nothing_close_finds_nothing() {
    assert_eq!(found("zzzzqq"), None);
    assert_eq!(found("photoshop"), None);
}

#[test]
fn a_query_too_short_or_empty_finds_nothing() {
    assert_eq!(found(""), None);
    assert_eq!(found("   "), None);
    assert_eq!(found("a"), None);
    assert_eq!(found("***"), None);
}

#[test]
fn a_maintenance_entry_is_never_picked_not_even_when_asked_for() {
    assert_eq!(found("spotify").as_deref(), Some("Spotify"));
    let only_uninstaller = vec![entry("Uninstall Spotify", "u")];
    assert!(find_app(&only_uninstaller, "spotify").is_none());
    // Asking for the uninstaller must not open the app either: open_app has no
    // confirmation, so "uninstall spotify" is not a request it can serve.
    assert_eq!(found("uninstall spotify"), None);
    assert_eq!(found("desinstalar spotify"), None);
}

#[test]
fn a_very_short_query_matches_only_a_whole_name_or_a_whole_word() {
    let menu = vec![
        entry("Logitech G Hub", "Logitech.GHub_abc!App"),
        entry("Log Viewer Pro", r"C:\Tools\logviewer.exe"),
        entry("Notepad", "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
        entry(
            "VLC media player",
            r"{6D809377-6AF0-444B-8957-A3773F02200E}\VideoLAN\VLC\vlc.exe",
        ),
        entry("Google Chrome", "Chrome"),
        entry("Microsoft Edge", "MSEdge"),
        entry("Spotify", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
    ];
    let pick = |q: &str| find_app(&menu, q).map(|e| e.name.clone());
    // "log" is a prefix of Logitech but not a word of it: no guess.
    assert_ne!(pick("log").as_deref(), Some("Logitech G Hub"));
    assert_eq!(pick("lo"), None, "two letters never prefix-match");
    assert_eq!(
        pick("not"),
        None,
        "three letters do not prefix-match Notepad"
    );
    // Short but a whole word of the name: fine.
    assert_eq!(pick("vlc").as_deref(), Some("VLC media player"));
    // From four letters the usual rules apply.
    assert_eq!(pick("edge").as_deref(), Some("Microsoft Edge"));
    assert_eq!(pick("chrome").as_deref(), Some("Google Chrome"));
    assert_eq!(pick("spot").as_deref(), Some("Spotify"));
    assert_eq!(pick("note").as_deref(), Some("Notepad"));
}

#[test]
fn the_choice_does_not_depend_on_the_order_of_the_menu() {
    let mut reversed = menu();
    reversed.reverse();
    for query in ["note", "word", "spotify app", "calculater"] {
        assert_eq!(
            find_app(&menu(), query).map(|e| &e.id),
            find_app(&reversed, query).map(|e| &e.id),
            "{query}"
        );
    }
}

// --- the launch target ---------------------------------------------------------

#[test]
fn the_target_is_the_apps_folder_path_of_the_indexed_id() {
    assert_eq!(
        launch_target(&entry(
            "Notepad",
            "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"
        )),
        r"shell:AppsFolder\Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"
    );
}

// --- the index and open_app ----------------------------------------------------

struct FakeSource(Mutex<Vec<Result<Vec<AppEntry>, String>>>);

impl FakeSource {
    fn with(answers: Vec<Result<Vec<AppEntry>, String>>) -> FakeSource {
        FakeSource(Mutex::new(answers))
    }
}

impl AppSource for FakeSource {
    fn list(&self) -> Result<Vec<AppEntry>, String> {
        let mut answers = self.0.lock().unwrap();
        if answers.len() > 1 {
            answers.remove(0)
        } else {
            answers[0].clone()
        }
    }
}

#[derive(Default)]
struct FakeLauncher {
    targets: Mutex<Vec<String>>,
    fail: Mutex<Option<String>>,
}

impl Launcher for FakeLauncher {
    fn launch(&self, target: &str) -> Result<(), String> {
        self.targets.lock().unwrap().push(target.to_string());
        match self.fail.lock().unwrap().clone() {
            Some(why) => Err(why),
            None => Ok(()),
        }
    }
}

fn ready_apps() -> (Apps, Arc<FakeLauncher>, Arc<AppIndex>) {
    let index = Arc::new(AppIndex::new());
    index
        .refresh_from(&FakeSource::with(vec![Ok(menu())]))
        .unwrap();
    let launcher = Arc::new(FakeLauncher::default());
    (Apps::new(index.clone(), launcher.clone()), launcher, index)
}

#[test]
fn before_the_index_is_built_open_is_not_ready_and_starts_nothing() {
    let index = Arc::new(AppIndex::new());
    let launcher = Arc::new(FakeLauncher::default());
    let apps = Apps::new(index.clone(), launcher.clone());
    assert!(!index.is_ready());
    assert_eq!(apps.open("notepad"), Err(PcError::NotReady));
    assert!(launcher.targets.lock().unwrap().is_empty());
}

#[test]
fn a_failed_first_build_leaves_the_index_not_ready() {
    let index = AppIndex::new();
    let source = FakeSource::with(vec![Err("powershell failed".into())]);
    assert_eq!(index.refresh_from(&source), Err("powershell failed".into()));
    assert!(!index.is_ready());
}

#[test]
fn a_failed_rebuild_keeps_the_previous_index() {
    let (apps, launcher, index) = ready_apps();
    assert!(
        index
            .refresh_from(&FakeSource::with(vec![Err("boom".into())]))
            .is_err()
    );
    assert!(index.is_ready());
    assert_eq!(apps.open("notepad"), Ok("Notepad".to_string()));
    assert_eq!(launcher.targets.lock().unwrap().len(), 1);
}

#[test]
fn a_match_starts_only_the_indexed_target() {
    let (apps, launcher, _) = ready_apps();
    assert_eq!(apps.open("spotify"), Ok("Spotify".to_string()));
    assert_eq!(
        *launcher.targets.lock().unwrap(),
        vec![r"shell:AppsFolder\SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string()]
    );
}

#[test]
fn the_models_text_never_reaches_the_launcher() {
    // Model output is data: shell syntax in it can only influence WHICH indexed
    // app is picked, never what is run. Each hostile string below is known to
    // reach the fuzzy match (the expected pick is asserted), so the loop cannot
    // pass by finding nothing.
    let hostile: [(&str, Option<&str>); 7] = [
        (
            "notepad & calc.exe",
            Some("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
        ),
        (
            "notepad\"; Start-Process calc; \"",
            Some("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
        ),
        (
            "spotify | del /q *",
            Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
        ),
        (
            "notepad $(calc)",
            Some("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
        ),
        (
            "%SystemRoot%\\notepad.exe",
            Some("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
        ),
        (r"C:\Windows\System32\cmd.exe /c calc", None),
        ("; rm -rf /", None),
    ];
    let mut reached_the_launcher = 0;
    for (text, expected_id) in hostile {
        let (apps, launcher, _) = ready_apps();
        match expected_id {
            Some(id) => {
                assert!(
                    apps.open(text).is_ok(),
                    "{text:?} should reach the fuzzy match"
                );
                reached_the_launcher += 1;
                assert_eq!(
                    *launcher.targets.lock().unwrap(),
                    vec![format!(r"shell:AppsFolder\{id}")],
                    "{text:?}"
                );
            }
            None => {
                assert_eq!(apps.open(text), Err(PcError::NotFound), "{text:?}");
                assert!(launcher.targets.lock().unwrap().is_empty(), "{text:?}");
            }
        }
    }
    assert_eq!(reached_the_launcher, 5, "the fuzzy path was exercised");
}

#[test]
fn a_miss_is_not_found_starts_nothing_and_asks_for_a_refresh() {
    let (apps, launcher, index) = ready_apps();
    assert_eq!(apps.open("photoshop"), Err(PcError::NotFound));
    assert!(launcher.targets.lock().unwrap().is_empty());
    assert!(
        index.refresh.wait(Duration::from_millis(0)),
        "a miss asks the background refresher to look again"
    );
    assert!(
        !index.refresh.wait(Duration::from_millis(0)),
        "consumed once"
    );
}

#[test]
fn a_hit_does_not_ask_for_a_refresh() {
    let (apps, _, index) = ready_apps();
    assert_eq!(apps.open("notepad"), Ok("Notepad".to_string()));
    assert!(!index.refresh.wait(Duration::from_millis(0)));
}

#[test]
fn a_launcher_failure_is_reported_not_hidden() {
    let (apps, launcher, _) = ready_apps();
    *launcher.fail.lock().unwrap() = Some("access denied".into());
    assert_eq!(
        apps.open("notepad"),
        Err(PcError::Failed("access denied".into()))
    );
}

#[test]
fn a_slow_launch_does_not_hold_the_index() {
    // The launcher is the slow part (a COM call). While it runs, the index must
    // stay usable: a held lock would stall every other command and the
    // background refresh. (Not waiting for a window is by construction: the
    // port has nothing to wait on.)
    struct ProbingLauncher(Arc<AppIndex>, AtomicUsize);
    impl Launcher for ProbingLauncher {
        fn launch(&self, _target: &str) -> Result<(), String> {
            self.1.fetch_add(1, Ordering::SeqCst);
            let index = self.0.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(index.is_ready());
            });
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(true) => Ok(()),
                other => Err(format!("index unusable during a launch: {other:?}")),
            }
        }
    }
    let index = Arc::new(AppIndex::new());
    index
        .refresh_from(&FakeSource::with(vec![Ok(menu())]))
        .unwrap();
    let launcher = Arc::new(ProbingLauncher(index.clone(), AtomicUsize::new(0)));
    let apps = Apps::new(index, launcher.clone());
    assert_eq!(apps.open("calculator"), Ok("Calculator".to_string()));
    assert_eq!(launcher.1.load(Ordering::SeqCst), 1);
}

// --- the refresh signal --------------------------------------------------------

#[test]
fn waiting_times_out_without_a_request() {
    let signal = RefreshSignal::default();
    assert!(!signal.wait(Duration::from_millis(20)));
}

#[test]
fn a_request_wakes_a_waiter_and_is_consumed() {
    let signal = Arc::new(RefreshSignal::default());
    let waiter = {
        let signal = signal.clone();
        std::thread::spawn(move || signal.wait(Duration::from_secs(10)))
    };
    std::thread::sleep(Duration::from_millis(20));
    signal.request();
    assert!(waiter.join().unwrap());
    assert!(!signal.wait(Duration::from_millis(0)));
}

#[test]
fn a_request_made_before_waiting_is_not_lost() {
    let signal = RefreshSignal::default();
    signal.request();
    signal.request();
    assert!(signal.wait(Duration::from_millis(0)));
    assert!(
        !signal.wait(Duration::from_millis(0)),
        "requests do not pile up"
    );
}

// --- the refresh schedule and the PowerShell path --------------------------------

#[test]
fn the_retry_after_a_failed_build_backs_off_up_to_a_cap() {
    let mut previous = None;
    let mut seen = Vec::new();
    for _ in 0..8 {
        let next = next_retry_delay(previous);
        seen.push(next.as_secs());
        previous = Some(next);
    }
    assert_eq!(seen, [30, 60, 120, 240, 480, 600, 600, 600]);
}

#[test]
fn a_successful_build_resets_the_backoff_and_waits_the_normal_refresh() {
    let index = AppIndex::new();
    let mut backoff = None;
    let failing = FakeSource::with(vec![Err("boom".into())]);
    let waits: Vec<u64> = (0..4)
        .map(|_| refresh_step(&index, &failing, &mut backoff).as_secs())
        .collect();
    assert_eq!(
        waits,
        [30, 60, 120, 240],
        "a dead PowerShell is not respawned every 30 s"
    );
    assert!(!index.is_ready());

    let working = FakeSource::with(vec![Ok(menu())]);
    assert_eq!(refresh_step(&index, &working, &mut backoff), REFRESH_EVERY);
    assert!(index.is_ready());
    assert_eq!(
        backoff, None,
        "the next failure starts again at the short delay"
    );
    assert_eq!(refresh_step(&index, &failing, &mut backoff).as_secs(), 30);
}

#[test]
fn powershell_is_found_only_through_the_system_root() {
    use std::path::PathBuf;
    assert_eq!(
        powershell_path(Some(r"C:\Windows".into())),
        Some(PathBuf::from(
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
        ))
    );
    // Fail closed: no bare "powershell.exe" resolved through PATH or the
    // current directory when SystemRoot is missing or empty.
    assert_eq!(powershell_path(None), None);
    assert_eq!(powershell_path(Some("".into())), None);
}

// --- Spanish and English names of built-in apps ----------------------------------

fn pick<'a>(menu: &'a [AppEntry], query: &str) -> Option<&'a str> {
    find_app(menu, query).map(|e| e.name.as_str())
}

const SETTINGS_ID: &str = "windows.immersivecontrolpanel_cw5n1h2txyewy!Microsoft.Windows.Settings";

fn english_menu() -> Vec<AppEntry> {
    vec![
        entry(
            "Calculator",
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
        ),
        entry("Notepad", "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
        entry("Settings", SETTINGS_ID),
    ]
}

fn spanish_menu() -> Vec<AppEntry> {
    vec![
        entry("Calculadora", "Microsoft.WindowsCalculator.es!App"),
        entry("Bloc de notas", "Microsoft.WindowsNotepad.es!App"),
        entry("Configuración", SETTINGS_ID),
    ]
}

fn mixed_menu() -> Vec<AppEntry> {
    vec![
        entry("WSL Settings", r"C:\Program Files\WSL\wslsettings.exe"),
        entry("Configuración", SETTINGS_ID),
        entry(
            "Configuración de cámara Logitech",
            r"C:\Program Files\Logitech\camera.exe",
        ),
        entry("Camera", "Microsoft.WindowsCamera_8wekyb3d8bbwe!App"),
        entry("Logitech G Hub", r"C:\Program Files\Logitech\ghub.exe"),
        entry("Notepad", "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"),
        entry("Notepad++", r"C:\Program Files\Notepad++\notepad++.exe"),
        entry(
            "Calculator",
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
        ),
    ]
}

#[test]
fn a_spanish_name_finds_the_english_entry() {
    let menu = english_menu();
    assert_eq!(pick(&menu, "calculadora"), Some("Calculator"));
    assert_eq!(pick(&menu, "bloc de notas"), Some("Notepad"));
    assert_eq!(pick(&menu, "configuración"), Some("Settings"));
}

#[test]
fn an_english_name_finds_the_spanish_entry() {
    let menu = spanish_menu();
    assert_eq!(pick(&menu, "calculator"), Some("Calculadora"));
    assert_eq!(pick(&menu, "notepad"), Some("Bloc de notas"));
    assert_eq!(pick(&menu, "settings"), Some("Configuración"));
}

#[test]
fn an_article_or_an_app_word_around_the_name_is_ignored() {
    let menu = english_menu();
    assert_eq!(pick(&menu, "la calculadora"), Some("Calculator"));
    assert_eq!(pick(&menu, "calculadora app"), Some("Calculator"));
    assert_eq!(pick(&menu, "mi calculadora aplicacion"), Some("Calculator"));
    assert_eq!(pick(&spanish_menu(), "the calculator"), Some("Calculadora"));
}

#[test]
fn settings_prefers_the_windows_app_over_a_longer_name_with_the_word() {
    let menu = mixed_menu();
    assert_eq!(pick(&menu, "settings"), Some("Configuración"));
    assert_eq!(pick(&menu, "ajustes"), Some("Configuración"));
    assert_eq!(pick(&menu, "configuracion"), Some("Configuración"));
}

#[test]
fn an_alias_needs_the_whole_query_not_a_word_of_it() {
    let menu = mixed_menu();
    assert_eq!(
        pick(&menu, "camara logitech"),
        Some("Configuración de cámara Logitech")
    );
    assert_eq!(pick(&menu, "camara"), Some("Camera"));
    assert!(alias_terms("camara logitech").is_empty());
}

#[test]
fn an_alias_only_reaches_a_whole_name() {
    // Never a prefix or typo match through the other language's word.
    let menu = vec![
        entry("Photoshop", "Adobe.Photoshop"),
        entry("Clockify", "Coing.Clockify"),
        entry("Mailbird", "Mailbird.Mailbird"),
    ];
    assert_eq!(pick(&menu, "fotos"), None);
    assert_eq!(pick(&menu, "reloj"), None);
    assert_eq!(pick(&menu, "correo"), None);
}

#[test]
fn a_name_that_differs_by_a_symbol_loses_to_the_real_translation() {
    let menu = vec![
        entry(
            "Bloc de notas",
            "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App",
        ),
        entry("Notepad++", r"C:\Program Files\Notepad++\notepad++.exe"),
    ];
    assert_eq!(pick(&menu, "notepad"), Some("Bloc de notas"));
    assert_eq!(pick(&menu, "notepad++"), Some("Notepad++"));
}

#[test]
fn a_name_with_a_symbol_is_still_found_when_nothing_else_fits() {
    let menu = vec![entry(
        "Notepad++",
        r"C:\Program Files\Notepad++\notepad++.exe",
    )];
    assert_eq!(pick(&menu, "notepad"), Some("Notepad++"));
}

#[test]
fn notepad_plus_plus_is_not_confused_with_notepad() {
    let menu = mixed_menu();
    assert_eq!(pick(&menu, "notepad++"), Some("Notepad++"));
    assert_eq!(pick(&menu, " Notepad++ "), Some("Notepad++"));
    assert_eq!(pick(&menu, "notepad"), Some("Notepad"));
}

#[test]
fn the_name_asked_for_wins_when_both_languages_are_in_the_menu() {
    // The shared test menu holds Calculator and Calculadora: each name still
    // opens itself.
    assert_eq!(found("calculadora").as_deref(), Some("Calculadora"));
    assert_eq!(found("calculator").as_deref(), Some("Calculator"));
}

#[test]
fn an_uninstaller_is_never_reached_through_an_alias() {
    let menu = vec![
        entry("Desinstalar Calculadora", r"C:\Tools\unins000.exe"),
        entry("Uninstall Notepad", r"C:\Tools\uninstall.exe"),
        entry("Spotify", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
    ];
    assert_eq!(pick(&menu, "calculator"), None);
    assert_eq!(pick(&menu, "calculadora"), None);
    assert_eq!(pick(&menu, "bloc de notas"), None);
    assert_eq!(pick(&menu, "notepad"), None);
}

#[test]
fn an_unknown_query_has_no_aliases() {
    assert!(alias_terms("spotify").is_empty());
    assert!(alias_terms("").is_empty());
    assert!(alias_terms("app").is_empty());
    assert!(alias_terms("my").is_empty());
}

#[test]
fn every_alias_term_is_normalized_and_belongs_to_one_group() {
    let mut seen = std::collections::HashSet::new();
    for group in aliases::GROUPS {
        assert!(group.len() >= 2, "{group:?} has nothing to alias");
        for term in *group {
            assert_eq!(*term, normalize(term), "{term:?} is not normalized");
            assert!(seen.insert(*term), "{term:?} is in two groups");
        }
    }
}

// --- process list --------------------------------------------------------------

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[test]
fn a_process_of_the_users_session_is_marked_as_such() {
    let info = process_info(&wide("Spotify.exe"), Some(1), Some(1));
    assert_eq!(info.exe_name, "Spotify.exe");
    assert!(info.in_user_session);
}

#[test]
fn another_or_an_unknown_session_is_never_the_users() {
    assert!(!process_info(&wide("svchost.exe"), Some(0), Some(1)).in_user_session);
    assert!(!process_info(&wide("a.exe"), None, Some(1)).in_user_session);
    assert!(!process_info(&wide("a.exe"), Some(1), None).in_user_session);
    assert!(!process_info(&wide("a.exe"), None, None).in_user_session);
}

#[test]
fn the_name_stops_at_the_first_nul() {
    let mut buffer = wide("notepad.exe");
    buffer.extend([0, 0, u16::from(b'x')]);
    assert_eq!(
        process_info(&buffer, Some(1), Some(1)).exe_name,
        "notepad.exe"
    );
}

#[test]
fn a_name_that_is_not_valid_utf16_is_kept_lossily() {
    let info = process_info(
        &[u16::from(b'a'), 0xD800, u16::from(b'b')],
        Some(1),
        Some(1),
    );
    assert!(info.exe_name.starts_with('a') && info.exe_name.ends_with('b'));
}

// --- the real Windows adapter ----------------------------------------------------

#[cfg(windows)]
mod windows_live {
    use super::*;
    use crate::adapters::win_apps::WinApps;

    /// Read-only: lists the processes of this machine.
    #[test]
    fn the_snapshot_lists_this_test_process_in_the_users_session() {
        let apps = WinApps::without_index();
        let processes = apps.running_processes();
        let me = std::env::current_exe().unwrap();
        let me = me.file_name().unwrap().to_string_lossy().to_lowercase();
        let mine = processes
            .iter()
            .find(|p| p.exe_name.to_lowercase() == me)
            .unwrap_or_else(|| panic!("{me} is not in the snapshot"));
        assert!(mine.in_user_session);
    }

    /// Runs PowerShell (read-only: lists the Start menu). Ignored so a normal
    /// `cargo test` does not spawn it. Run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "runs PowerShell Get-StartApps (about 4 s)"]
    fn the_real_start_menu_has_apps_with_names_and_ids() {
        let entries = crate::adapters::win_apps::PowerShellSource.list().unwrap();
        assert!(!entries.is_empty());
        assert!(
            entries
                .iter()
                .all(|e| !e.name.is_empty() && is_launchable_id(&e.id) && !is_maintenance(e)),
            "the index holds only launchable apps"
        );
        // Informational (run with --nocapture): what the fuzzy match picks here.
        for query in [
            "notepad",
            "bloc de notas",
            "calculadora",
            "calculator",
            "spotify",
            "chrome",
            "edge",
        ] {
            let pick = find_app(&entries, query).map(|e| e.name.as_str());
            eprintln!("{} apps; {query:?} -> {pick:?}", entries.len());
        }
    }

    /// Opens a real app. Ignored: it changes the desktop (starts Notepad).
    #[test]
    #[ignore = "starts Notepad on the real desktop"]
    fn the_real_adapter_opens_notepad() {
        let apps = WinApps::start();
        for _ in 0..40 {
            if apps.is_ready() {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let opened = apps.open("notepad").expect("Notepad opens");
        eprintln!("opened {opened:?}");
    }
}
