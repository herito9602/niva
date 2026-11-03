//! Safety gate tests. Spec: `action-safety`. The scenarios the gate does not
//! own are covered elsewhere: timeout and "no answer" by the reducer
//! (`state_machine/tests_confirm.rs`), the whole-utterance yes/no matcher by
//! the assistant loop (PR 2b). The gate only turns an explicit accepted
//! answer into `Approved`.

use super::*;
use crate::ports::testing::{FakePc, PcCall};
use crate::ports::{CloseOutcome, PcActions, ProcessInfo};
use crate::tools::{ArgProblem, ArgValue, ToolCall, VolumeChange};

fn procs(names: &[&str]) -> Vec<ProcessInfo> {
    names
        .iter()
        .map(|name| ProcessInfo {
            exe_name: (*name).into(),
            in_user_session: true,
        })
        .collect()
}

fn tool(name: &str, key: &str, value: ArgValue) -> ToolCall {
    ToolCall {
        name: name.into(),
        args: [(key.to_string(), value)].into(),
    }
}

fn close_call(query: &str) -> ToolCall {
    tool("close_app", "name", ArgValue::Str(query.into()))
}

fn power_call(action: &str) -> ToolCall {
    tool("power_action", "action", ArgValue::Str(action.into()))
}

fn cid(request: u64) -> ConfirmationId {
    ConfirmationId { request, call: 0 }
}

fn click(id: u64) -> ConfirmationAnswer {
    ConfirmationAnswer {
        confirmation_id: cid(id),
        channel: Channel::Click,
        accepted: true,
    }
}

fn voice(id: u64) -> ConfirmationAnswer {
    ConfirmationAnswer {
        confirmation_id: cid(id),
        channel: Channel::Voice,
        accepted: true,
    }
}

fn risky(call: &ToolCall, processes: &[ProcessInfo], id: u64) -> RiskyRequest {
    match SafetyGate::new().validate(call, processes, id, 0) {
        Ok(Validated::Risky(request)) => request,
        other => panic!("expected a risky request, got {other:?}"),
    }
}

// Matcher: substring only, never fuzzy

#[test]
fn a_near_miss_name_matches_nothing() {
    // Spec "Near-miss name": "Notepad" and "Notion" running, "Notpad" asked.
    let running = procs(&["Notepad.exe", "Notion.exe"]);
    assert_eq!(
        resolve_close_target("Notpad", &running),
        Err(ToolError::NoMatch)
    );
}

#[test]
fn matching_is_a_case_insensitive_substring_of_the_process_stem() {
    let running = procs(&["Spotify.exe", "chrome.exe"]);
    assert_eq!(
        resolve_close_target("SPOTIFY", &running),
        Ok("Spotify.exe".into())
    );
    assert_eq!(
        resolve_close_target("poti", &running),
        Ok("Spotify.exe".into())
    );
    assert_eq!(
        resolve_close_target("Chrome", &running),
        Ok("chrome.exe".into())
    );
}

#[test]
fn whitespace_in_the_query_is_ignored() {
    let running = procs(&["GoogleChrome.exe"]);
    assert_eq!(
        resolve_close_target("google chrome", &running),
        Ok("GoogleChrome.exe".into())
    );
}

#[test]
fn a_query_may_carry_the_exe_suffix_of_a_candidate_name() {
    // The follow-up answer to "which one?" can repeat a candidate's file name.
    let running = procs(&["Spotify.exe", "chrome.exe"]);
    assert_eq!(
        resolve_close_target("Spotify.exe", &running),
        Ok("Spotify.exe".into())
    );
}

#[test]
fn the_exe_suffix_is_not_part_of_the_match() {
    // "exe" is in every name; it must not make everything match.
    let running = procs(&["Spotify.exe", "chrome.exe"]);
    assert_eq!(
        resolve_close_target("exe", &running),
        Err(ToolError::NoMatch)
    );
}

#[test]
fn a_fuzzy_neighbour_is_never_returned() {
    // The spike once mapped "Spotify" to FortiTray.exe by fuzzy matching.
    let running = procs(&["FortiTray.exe"]);
    assert_eq!(
        resolve_close_target("Spotify", &running),
        Err(ToolError::NoMatch)
    );
}

#[test]
fn a_query_shorter_than_three_characters_is_refused() {
    let running = procs(&["Spotify.exe"]);
    for query in ["", "s", "sp", " s p "] {
        assert_eq!(
            resolve_close_target(query, &running),
            Err(ToolError::QueryTooShort { min: 3 }),
            "{query:?}"
        );
    }
    assert_eq!(
        resolve_close_target("spo", &running),
        Ok("Spotify.exe".into())
    );
}

#[test]
fn the_minimum_counts_characters_not_bytes() {
    let running = procs(&["Ñandú.exe"]);
    assert_eq!(
        resolve_close_target("ñan", &running),
        Ok("Ñandú.exe".into())
    );
    assert_eq!(
        resolve_close_target("ñá", &running),
        Err(ToolError::QueryTooShort { min: 3 })
    );
}

// Matcher: ambiguity (spec Assumptions: ask which one, never close all)

#[test]
fn several_distinct_executables_are_returned_as_candidates() {
    let running = procs(&["Notion.exe", "Notepad.exe", "Spotify.exe"]);
    assert_eq!(
        resolve_close_target("not", &running),
        Err(ToolError::Ambiguous(vec![
            "Notepad.exe".into(),
            "Notion.exe".into()
        ]))
    );
}

#[test]
fn processes_of_the_same_executable_are_one_candidate() {
    let running = procs(&["chrome.exe", "chrome.exe", "Chrome.exe", "chrome.exe"]);
    assert_eq!(
        resolve_close_target("chrome", &running),
        Ok("chrome.exe".into())
    );
}

#[test]
fn an_exact_name_does_not_beat_a_longer_one() {
    // Spec: several matches -> ask, never pick one on its own.
    let running = procs(&["notepad.exe", "notepad++.exe"]);
    assert_eq!(
        resolve_close_target("notepad", &running),
        Err(ToolError::Ambiguous(vec![
            "notepad++.exe".into(),
            "notepad.exe".into()
        ]))
    );
}

// Matcher: protected list

const PROTECTED: [&str; 15] = [
    "ollama",
    "explorer",
    "dwm",
    "csrss",
    "winlogon",
    "wininit",
    "lsass",
    "services",
    "smss",
    "svchost",
    "sihost",
    "ctfmon",
    "SearchHost",
    "StartMenuExperienceHost",
    "ShellExperienceHost",
];

#[test]
fn every_protected_process_is_protected_in_any_case() {
    for stem in PROTECTED {
        assert!(is_protected(&format!("{stem}.exe")), "{stem}");
        assert!(
            is_protected(&format!("{}.EXE", stem.to_uppercase())),
            "{stem}"
        );
        assert!(is_protected(stem), "{stem} without suffix");
    }
}

#[test]
fn niva_and_its_sidecar_and_ollama_helpers_are_protected() {
    for name in [
        "niva.exe",
        "Niva.exe",
        "niva-voice.exe",
        "niva_voice.exe",
        "ollama app.exe",
        "ollama_llama_server.exe",
    ] {
        assert!(is_protected(name), "{name}");
    }
}

#[test]
fn ordinary_apps_are_not_protected() {
    for name in [
        "Spotify.exe",
        "chrome.exe",
        "notepad.exe",
        "explorer2.exe",
        "niv.exe",
        "olla.exe",
    ] {
        assert!(!is_protected(name), "{name}");
    }
}

#[test]
fn a_protected_process_is_never_matched_even_by_its_exact_name() {
    let running = procs(&[
        "explorer.exe",
        "svchost.exe",
        "dwm.exe",
        "ollama.exe",
        "niva.exe",
    ]);
    for query in ["explorer", "svchost", "dwm.exe", "OLLAMA", "niva"] {
        assert_eq!(
            resolve_close_target(query, &running),
            Err(ToolError::NoMatch),
            "{query}"
        );
    }
}

#[test]
fn a_protected_process_does_not_make_a_match_ambiguous() {
    // "host" is inside svchost (protected) and in ghostly (not).
    let running = procs(&["svchost.exe", "ghostly.exe", "SearchHost.exe"]);
    assert_eq!(
        resolve_close_target("host", &running),
        Ok("ghostly.exe".into())
    );
}

#[test]
fn processes_outside_the_user_session_are_refused() {
    let mut running = procs(&["Spotify.exe"]);
    running[0].in_user_session = false;
    assert_eq!(
        resolve_close_target("spotify", &running),
        Err(ToolError::NoMatch)
    );
}

// Validation through the gate

#[test]
fn a_safe_call_validates_without_confirmation() {
    // Spec "Safe action": "volume to 30" executes with no confirmation.
    let call = tool("set_volume", "level", ArgValue::Int(30));
    assert_eq!(
        SafetyGate::new().validate(&call, &[], 1, 0),
        Ok(Validated::Safe(SafeAction::SetVolume {
            change: VolumeChange::Set(30),
            clamped: false
        }))
    );
}

#[test]
fn close_app_is_risky_and_names_the_exact_executable() {
    let request = risky(&close_call("chrome"), &procs(&["chrome.exe"]), 9);
    assert_eq!(request.target(), "chrome.exe");
    assert_eq!(request.confirmation_id(), cid(9));
    assert!(
        !request.click_only(),
        "closing an app may be confirmed by voice"
    );
}

#[test]
fn power_restart_and_shutdown_are_click_only() {
    for action in ["restart", "shutdown"] {
        assert!(risky(&power_call(action), &[], 1).click_only(), "{action}");
    }
}

#[test]
fn power_lock_and_sleep_may_be_confirmed_by_voice() {
    for action in ["lock", "sleep"] {
        assert!(!risky(&power_call(action), &[], 1).click_only(), "{action}");
    }
}

#[test]
fn every_power_action_is_risky() {
    for kind in PowerKind::ALL {
        let request = risky(&power_call(kind.as_str()), &[], 1);
        assert_eq!(request.target(), kind.as_str());
    }
}

#[test]
fn invalid_arguments_yield_a_typed_error() {
    let gate = SafetyGate::new();
    assert_eq!(
        gate.validate(&power_call("hibernate"), &[], 1, 0),
        Err(ToolError::InvalidArgument {
            name: "action",
            problem: ArgProblem::NotOneOf
        })
    );
    assert_eq!(
        gate.validate(&close_call("sp"), &procs(&["Spotify.exe"]), 1, 0),
        Err(ToolError::QueryTooShort { min: 3 })
    );
    assert_eq!(
        gate.validate(&close_call("zoom"), &procs(&["Spotify.exe"]), 1, 0),
        Err(ToolError::NoMatch)
    );
}

#[test]
fn close_app_with_several_matches_returns_the_candidates_not_a_request() {
    let result = SafetyGate::new().validate(
        &close_call("not"),
        &procs(&["Notepad.exe", "Notion.exe"]),
        1,
        0,
    );
    assert_eq!(
        result,
        Err(ToolError::Ambiguous(vec![
            "Notepad.exe".into(),
            "Notion.exe".into()
        ]))
    );
}

// Approval

#[test]
fn an_explicit_click_approves_a_close() {
    // Spec "Confirmed".
    let request = risky(&close_call("chrome"), &procs(&["chrome.exe"]), 4);
    let Ok(Approval::Close(approved)) = SafetyGate::new().approve(request, click(4)) else {
        panic!("a click must approve");
    };
    assert_eq!(approved.action().exe(), "chrome.exe");
    assert_eq!(approved.confirmation_id(), cid(4));
}

#[test]
fn a_voice_yes_approves_a_close_lock_or_sleep() {
    let gate = SafetyGate::new();
    let close = risky(&close_call("chrome"), &procs(&["chrome.exe"]), 4);
    assert!(matches!(
        gate.approve(close, voice(4)),
        Ok(Approval::Close(_))
    ));
    for action in ["lock", "sleep"] {
        let request = risky(&power_call(action), &[], 5);
        assert!(
            matches!(gate.approve(request, voice(5)), Ok(Approval::Power(_))),
            "{action}"
        );
    }
}

#[test]
fn restart_and_shutdown_cannot_be_approved_by_voice() {
    let gate = SafetyGate::new();
    for action in ["restart", "shutdown"] {
        // A refused answer consumes the request; the click needs a new one.
        let by_voice = risky(&power_call(action), &[], 5);
        assert_eq!(
            gate.approve(by_voice, voice(5)),
            Err(ApprovalError::ClickRequired)
        );
        let by_click = risky(&power_call(action), &[], 5);
        assert!(
            matches!(gate.approve(by_click, click(5)), Ok(Approval::Power(_))),
            "{action}"
        );
    }
}

#[test]
fn a_denied_answer_does_not_approve() {
    // Spec "Denied" and "Unclear reply": anything not accepted is no approval.
    let request = risky(&close_call("chrome"), &procs(&["chrome.exe"]), 4);
    let answer = ConfirmationAnswer {
        accepted: false,
        ..click(4)
    };
    assert_eq!(
        SafetyGate::new().approve(request, answer),
        Err(ApprovalError::NotConfirmed)
    );
}

#[test]
fn an_answer_for_another_confirmation_does_not_approve() {
    let request = risky(&close_call("chrome"), &procs(&["chrome.exe"]), 4);
    assert_eq!(
        SafetyGate::new().approve(request, click(3)),
        Err(ApprovalError::WrongConfirmation)
    );
}

// Pinned pick (the follow-up answer to "which one?"): the generic path never
// takes an exact name as a shortcut, only `validate_pinned` does

#[test]
fn a_generic_close_with_an_exact_exe_name_is_still_ambiguous() {
    let running = procs(&["notepad.exe", "notepad++.exe", "Spotify.exe"]);
    let expected = Err(ToolError::Ambiguous(vec![
        "notepad++.exe".into(),
        "notepad.exe".into(),
    ]));
    assert_eq!(resolve_close_target("notepad.exe", &running), expected);
    assert_eq!(resolve_close_target("  NOTEPAD.EXE ", &running), expected);
    // ...and through the gate, which is what the model's call goes through.
    assert!(matches!(
        SafetyGate::new().validate(&close_call("notepad.exe"), &running, 1, 0),
        Err(ToolError::Ambiguous(_))
    ));
}

#[test]
fn a_pinned_exe_name_picks_that_executable_among_several_matches() {
    let running = procs(&["notepad.exe", "notepad++.exe", "Spotify.exe"]);
    let gate = SafetyGate::new();
    let request = gate
        .validate_pinned("notepad.exe", &running, 4, 0)
        .expect("an exact name picks");
    assert_eq!(request.target(), "notepad.exe");
    assert_eq!(request.confirmation_id(), cid(4));
    let request = gate
        .validate_pinned("  NOTEPAD++.EXE ", &running, 5, 2)
        .expect("case and spaces do not matter");
    assert_eq!(request.target(), "notepad++.exe");
}

#[test]
fn a_pinned_exe_name_never_widens_what_it_reaches() {
    let gate = SafetyGate::new();
    // Protected processes stay unreachable even when named exactly.
    assert_eq!(
        gate.validate_pinned(
            "explorer.exe",
            &procs(&["explorer.exe", "notepad.exe"]),
            1,
            0
        ),
        Err(ToolError::NoMatch)
    );
    // Out-of-session processes are refused.
    let other_session = vec![ProcessInfo {
        exe_name: "notepad.exe".into(),
        in_user_session: false,
    }];
    assert_eq!(
        gate.validate_pinned("notepad.exe", &other_session, 1, 0),
        Err(ToolError::NoMatch)
    );
    // Only a whole file name counts: no substring, no prefix, nothing empty.
    let running = procs(&["notepad.exe", "notepad++.exe"]);
    for wrong in ["note.exe", "notepad", "pad.exe", "", "  "] {
        assert_eq!(
            gate.validate_pinned(wrong, &running, 1, 0),
            Err(ToolError::NoMatch),
            "{wrong:?}"
        );
    }
}

#[test]
fn a_pinned_pick_survives_the_re_resolve_at_execution() {
    let pc = FakePc::with_processes(&["notepad.exe", "notepad++.exe"]);
    let gate = SafetyGate::new();
    let request = gate
        .validate_pinned("notepad.exe", &pc.running_processes(), 4, 0)
        .expect("pinned");
    let Ok(Approval::Close(approved)) = gate.approve(request, click(4)) else {
        panic!("a click must approve");
    };
    assert_eq!(gate.execute_close(approved, &pc), Ok(CloseOutcome::Closed));
    assert_eq!(
        pc.calls(),
        vec![PcCall::CloseApp {
            exe: "notepad.exe".into()
        }],
        "only the picked executable is closed"
    );
}

#[test]
fn a_pinned_pick_whose_app_is_gone_closes_nothing() {
    let gate = SafetyGate::new();
    let before = procs(&["notepad.exe", "notepad++.exe"]);
    let request = gate
        .validate_pinned("notepad.exe", &before, 4, 0)
        .expect("pinned");
    let Ok(Approval::Close(approved)) = gate.approve(request, click(4)) else {
        panic!("a click must approve");
    };
    let pc = FakePc::with_processes(&["notepad++.exe"]);
    assert_eq!(
        gate.execute_close(approved, &pc),
        Err(ExecError::NotRunning)
    );
    assert!(pc.calls().is_empty());
}

// Execution

#[test]
fn a_confirmed_close_runs_exactly_once_with_the_confirmed_executable() {
    let pc = FakePc::with_processes(&["chrome.exe", "chrome.exe", "Spotify.exe"]);
    let gate = SafetyGate::new();
    let request = risky(&close_call("chrome"), &pc.running_processes(), 4);
    let Ok(Approval::Close(approved)) = gate.approve(request, click(4)) else {
        panic!("a click must approve");
    };
    assert_eq!(gate.execute_close(approved, &pc), Ok(CloseOutcome::Closed));
    assert_eq!(
        pc.calls(),
        vec![PcCall::CloseApp {
            exe: "chrome.exe".into()
        }]
    );
}

#[test]
fn a_close_that_leaves_the_app_running_reports_it() {
    let pc = FakePc::with_processes(&["chrome.exe"]);
    pc.set_close_outcome(CloseOutcome::StillRunning);
    let gate = SafetyGate::new();
    let request = risky(&close_call("chrome"), &pc.running_processes(), 4);
    let Ok(Approval::Close(approved)) = gate.approve(request, click(4)) else {
        panic!("a click must approve");
    };
    assert_eq!(
        gate.execute_close(approved, &pc),
        Ok(CloseOutcome::StillRunning)
    );
}

#[test]
fn the_target_is_re_resolved_and_a_changed_executable_set_aborts() {
    let pc = FakePc::with_processes(&["Notepad.exe"]);
    let gate = SafetyGate::new();
    let request = risky(&close_call("note"), &pc.running_processes(), 4);
    let Ok(Approval::Close(approved)) = gate.approve(request, click(4)) else {
        panic!("a click must approve");
    };
    // Between the prompt and the click, another executable now matches.
    pc.set_processes(procs(&["Notepad.exe", "Notes.exe"]));
    assert_eq!(
        gate.execute_close(approved, &pc),
        Err(ExecError::TargetChanged)
    );
    assert!(pc.calls().is_empty(), "nothing may be closed");
}

#[test]
fn a_different_single_executable_also_aborts() {
    let pc = FakePc::with_processes(&["Notepad.exe"]);
    let gate = SafetyGate::new();
    let request = risky(&close_call("note"), &pc.running_processes(), 4);
    let Ok(Approval::Close(approved)) = gate.approve(request, click(4)) else {
        panic!("a click must approve");
    };
    pc.set_processes(procs(&["Notes.exe"]));
    assert_eq!(
        gate.execute_close(approved, &pc),
        Err(ExecError::TargetChanged)
    );
    assert!(pc.calls().is_empty());
}

#[test]
fn new_pids_of_the_same_executable_are_fine() {
    let pc = FakePc::with_processes(&["chrome.exe"]);
    let gate = SafetyGate::new();
    let request = risky(&close_call("chrome"), &pc.running_processes(), 4);
    let Ok(Approval::Close(approved)) = gate.approve(request, click(4)) else {
        panic!("a click must approve");
    };
    pc.set_processes(procs(&["chrome.exe", "chrome.exe", "chrome.exe"]));
    assert_eq!(gate.execute_close(approved, &pc), Ok(CloseOutcome::Closed));
}

#[test]
fn an_app_that_stopped_before_the_click_is_reported_not_running() {
    let pc = FakePc::with_processes(&["chrome.exe"]);
    let gate = SafetyGate::new();
    let request = risky(&close_call("chrome"), &pc.running_processes(), 4);
    let Ok(Approval::Close(approved)) = gate.approve(request, click(4)) else {
        panic!("a click must approve");
    };
    pc.set_processes(vec![]);
    assert_eq!(
        gate.execute_close(approved, &pc),
        Err(ExecError::NotRunning)
    );
    assert!(pc.calls().is_empty());
}

#[test]
fn a_protected_process_that_starts_matching_after_approval_is_still_refused() {
    // Re-resolution applies the protected list too.
    let pc = FakePc::with_processes(&["svchost2.exe"]);
    let gate = SafetyGate::new();
    let request = risky(&close_call("svc"), &pc.running_processes(), 4);
    let Ok(Approval::Close(approved)) = gate.approve(request, click(4)) else {
        panic!("a click must approve");
    };
    pc.set_processes(procs(&["svchost.exe"]));
    assert_eq!(
        gate.execute_close(approved, &pc),
        Err(ExecError::NotRunning)
    );
    assert!(pc.calls().is_empty());
}

#[test]
fn a_confirmed_power_action_runs_exactly_the_confirmed_kind() {
    let pc = FakePc::with_processes(&[]);
    let gate = SafetyGate::new();
    let request = risky(&power_call("restart"), &[], 6);
    let Ok(Approval::Power(approved)) = gate.approve(request, click(6)) else {
        panic!("a click must approve a restart");
    };
    assert_eq!(gate.execute_power(approved, &pc), Ok(()));
    assert_eq!(pc.calls(), vec![PcCall::Power(PowerKind::Restart)]);
}

// Model bypass and "risky never runs without Approved"

#[test]
fn a_model_claiming_confirmation_still_only_gets_a_pending_request() {
    // Spec "Model bypass attempt". The call carrying a "confirmed" flag is an
    // unknown argument; the plain call is only a request. Neither touches the PC.
    let pc = FakePc::with_processes(&["chrome.exe"]);
    let gate = SafetyGate::new();

    let mut forged = close_call("chrome");
    forged.args.insert("confirmed".into(), ArgValue::Bool(true));
    assert_eq!(
        gate.validate(&forged, &pc.running_processes(), 1, 0),
        Err(ToolError::UnknownArgument("confirmed".into()))
    );

    let plain = gate.validate(&close_call("chrome"), &pc.running_processes(), 1, 0);
    assert!(matches!(plain, Ok(Validated::Risky(_))));
    assert!(pc.calls().is_empty(), "validation never executes");
}

#[test]
fn only_a_matching_accepted_answer_ever_reaches_the_risky_port_methods() {
    // The type-level guarantee is in the compile_fail doc tests of this
    // module. Here: every refused path leaves the PC untouched.
    let pc = FakePc::with_processes(&["chrome.exe"]);
    let gate = SafetyGate::new();
    let request = || risky(&close_call("chrome"), &pc.running_processes(), 4);
    let denied = ConfirmationAnswer {
        accepted: false,
        ..click(4)
    };
    assert!(gate.approve(request(), denied).is_err());
    assert!(gate.approve(request(), click(99)).is_err());
    let power = risky(&power_call("shutdown"), &[], 4);
    assert!(gate.approve(power, voice(4)).is_err());
    assert!(pc.calls().is_empty());
}

// Protected list: fail closed on the adapter-supplied name

#[test]
fn paths_trailing_junk_and_case_do_not_hide_a_protected_process() {
    for name in [
        r"C:\Windows\explorer.exe",
        "C:/Windows/System32/svchost.exe",
        r"\\?\C:\Windows\System32\Dwm.exe",
        "explorer.exe ",
        " explorer.exe",
        "explorer.exe.",
        "explorer.exe...",
        "EXPLORER.EXE",
        "ExPlOrEr.ExE",
        "Explorer",
    ] {
        assert!(is_protected(name), "{name:?}");
    }
}

#[test]
fn a_protected_process_with_a_decorated_name_cannot_resolve_as_a_target() {
    let running = procs(&[
        r"C:\Windows\explorer.exe",
        "C:/Windows/System32/svchost.exe",
        "explorer.exe ",
        "explorer.exe.",
        "EXPLORER.EXE",
    ]);
    for query in ["explorer", "svchost", "EXPLORER.EXE", "plorer"] {
        assert_eq!(
            resolve_close_target(query, &running),
            Err(ToolError::NoMatch),
            "{query}"
        );
    }
}

#[test]
fn an_odd_name_is_treated_as_protected() {
    for name in [
        "",
        "   ",
        ".",
        "...",
        ".exe",
        "dir\\",
        "bad\u{0}name.exe",
        "tab\tname.exe",
        "new\nline.exe",
        "co:lon.exe",
        "wild*card.exe",
        "q?.exe",
        "pipe|name.exe",
    ] {
        assert!(is_protected(name), "{name:?}");
    }
    let running = procs(&["", "bad\u{0}name.exe", "co:lon.exe"]);
    assert_eq!(
        resolve_close_target("name", &running),
        Err(ToolError::NoMatch)
    );
}

#[test]
fn a_path_does_not_change_the_candidate_name_shown_to_the_user() {
    let running = procs(&[r"C:\Apps\Spotify\Spotify.exe"]);
    assert_eq!(
        resolve_close_target("spotify", &running),
        Ok("Spotify.exe".into())
    );
    // A directory name is not part of the match.
    assert_eq!(
        resolve_close_target("apps", &running),
        Err(ToolError::NoMatch)
    );
}

#[test]
fn the_widened_list_protects_uwp_host_and_the_webview() {
    for name in [
        "ApplicationFrameHost.exe",
        "applicationframehost.exe",
        "msedgewebview2.exe",
        "MSEdgeWebView2.exe",
    ] {
        assert!(is_protected(name), "{name}");
    }
    let running = procs(&["ApplicationFrameHost.exe", "msedgewebview2.exe"]);
    assert_eq!(
        resolve_close_target("frame", &running),
        Err(ToolError::NoMatch)
    );
    assert_eq!(
        resolve_close_target("webview", &running),
        Err(ToolError::NoMatch)
    );
}

#[test]
fn every_niva_or_ollama_prefixed_executable_is_protected_with_or_without_separator() {
    for name in [
        "NivaVoice.exe",
        "nivaldo.exe",
        "niva.exe",
        "niva-eval.exe",
        "OllamaApp.exe",
        "ollama.exe",
        "ollama_llama_server.exe",
    ] {
        assert!(is_protected(name), "{name}");
    }
}

#[test]
fn candidates_that_differ_only_by_non_ascii_case_are_one_executable() {
    let running = procs(&["Ñandú.exe", "ñandú.exe"]);
    assert_eq!(
        resolve_close_target("ñandú", &running),
        Ok("Ñandú.exe".into())
    );
}

#[test]
fn a_non_ascii_case_variant_still_counts_as_the_confirmed_executable() {
    let pc = FakePc::with_processes(&["Ñandú.exe"]);
    let gate = SafetyGate::new();
    let request = risky(&close_call("ñandú"), &pc.running_processes(), 4);
    let Ok(Approval::Close(approved)) = gate.approve(request, click(4)) else {
        panic!("a click must approve");
    };
    pc.set_processes(procs(&["ÑANDÚ.exe"]));
    assert_eq!(gate.execute_close(approved, &pc), Ok(CloseOutcome::Closed));
}

// One confirmation, one execution; one id per risky call

#[test]
fn two_risky_calls_of_one_response_never_share_a_confirmation_id() {
    let gate = SafetyGate::new();
    let running = procs(&["chrome.exe", "Spotify.exe"]);
    let Ok(Validated::Risky(first)) = gate.validate(&close_call("chrome"), &running, 7, 0) else {
        panic!("risky");
    };
    let Ok(Validated::Risky(second)) = gate.validate(&close_call("spotify"), &running, 7, 1) else {
        panic!("risky");
    };
    assert_ne!(first.confirmation_id(), second.confirmation_id());
    // The answer to the first confirmation cannot approve the second call.
    let answer_to_first = ConfirmationAnswer {
        confirmation_id: first.confirmation_id(),
        channel: Channel::Click,
        accepted: true,
    };
    assert_eq!(
        gate.approve(second, answer_to_first),
        Err(ApprovalError::WrongConfirmation)
    );
    assert!(gate.approve(first, answer_to_first).is_ok());
}
