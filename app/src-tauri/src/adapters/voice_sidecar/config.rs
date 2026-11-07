//! How the host finds the sidecar: Python, the `voice/` folder, the models.
//!
//! This PR's answer is environment variables with a development default (the
//! installer, slice 5, will set them or replace this; see `docs/BACKLOG.md`):
//!
//! | Variable | Meaning | Default |
//! |---|---|---|
//! | `NIVA_VOICE` | `off` (or `0`, `false`, `no`) keeps the app in text mode | on |
//! | `NIVA_VOICE_DIR` | the folder with `main.py` | `voice\` next to the app; in a debug build, the repository's `voice\` |
//! | `NIVA_VOICE_PYTHON` | a Python executable, run as `python main.py ...` | `uv run --offline --no-project python main.py ...` |
//! | `NIVA_WAKE_MODEL_DIR` | the openWakeWord `.onnx` folder | the sidecar's own default |
//! | `NIVA_PIPER_DIR` | the Piper voices folder | none (no offline voice) |
//! | `NIVA_WHISPER_MODEL` | Whisper model, name or path, first rung of the ladder | `small`, then `base` |
//! | `NIVA_MIC_DEVICE` | microphone index or name for the sidecar (`--mic-device`; one value, spaces allowed) | the system default |
//! | `NIVA_VOICE_ARGS` | extra sidecar options, split on spaces (`--no-edge`, `--allow-cpu`, `--verbose`) | none |
//!
//! The sidecar reads `NIVA_WAKE_MODEL_DIR`, `NIVA_PIPER_DIR` and
//! `NIVA_WHISPER_MODEL` itself; the host passes them as options as well, so the
//! command line says what the process runs with.
//!
//! No `voice/` folder, no `uv` or Python: that is not an error of the app. The
//! host stays in text mode and the tray's "Restart voice" tries again.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::policy::DEFAULT_LADDER;

/// The reason `launch_spec` gives when `NIVA_VOICE=off`; the launcher turns it
/// into its own kind of failure so the tray can say "turned off".
pub const TURNED_OFF: &str = "voice is turned off (NIVA_VOICE=off)";

/// A command line, a folder and extra environment, ready to spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceConfig {
    enabled: bool,
    /// The folder with `main.py`, or why there is none.
    voice_dir: Result<PathBuf, String>,
    python: Option<String>,
    wake_model_dir: Option<String>,
    piper_dir: Option<String>,
    whisper: Option<String>,
    mic_device: Option<String>,
    extra_args: Vec<String>,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

impl VoiceConfig {
    /// Reads the configuration. `exe_dir` is the folder of the running app;
    /// `dev_dir` the repository's `voice/` (debug builds only); `exists` says
    /// whether a path is there.
    pub fn read(
        env: &dyn Fn(&str) -> Option<String>,
        exe_dir: Option<&Path>,
        dev_dir: Option<&Path>,
        exists: &dyn Fn(&Path) -> bool,
    ) -> VoiceConfig {
        let enabled = !matches!(
            non_empty(env("NIVA_VOICE"))
                .map(|v| v.to_lowercase())
                .as_deref(),
            Some("off" | "0" | "false" | "no")
        );
        let mut tried: Vec<PathBuf> = Vec::new();
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(dir) = non_empty(env("NIVA_VOICE_DIR")) {
            candidates.push(PathBuf::from(dir));
        } else {
            candidates.extend(exe_dir.map(|d| d.join("voice")));
            candidates.extend(dev_dir.map(Path::to_path_buf));
        }
        let mut found = None;
        for dir in candidates {
            if exists(&dir.join("main.py")) {
                found = Some(dir);
                break;
            }
            tried.push(dir);
        }
        let voice_dir = found.ok_or_else(|| {
            let list: Vec<String> = tried.iter().map(|p| p.display().to_string()).collect();
            if list.is_empty() {
                "no voice folder known (set NIVA_VOICE_DIR)".to_owned()
            } else {
                format!("no main.py in {} (set NIVA_VOICE_DIR)", list.join(", "))
            }
        });
        VoiceConfig {
            enabled,
            voice_dir,
            python: non_empty(env("NIVA_VOICE_PYTHON")),
            wake_model_dir: non_empty(env("NIVA_WAKE_MODEL_DIR")),
            piper_dir: non_empty(env("NIVA_PIPER_DIR")),
            whisper: non_empty(env("NIVA_WHISPER_MODEL")),
            mic_device: non_empty(env("NIVA_MIC_DEVICE")),
            extra_args: non_empty(env("NIVA_VOICE_ARGS"))
                .map(|v| v.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
        }
    }

    /// The real environment.
    pub fn detect() -> VoiceConfig {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        // Only a debug build knows where the repository is; a release build
        // must not carry the build machine's path.
        let dev_dir = cfg!(debug_assertions)
            .then(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../voice")));
        VoiceConfig::read(
            &|name| std::env::var(name).ok(),
            exe_dir.as_deref(),
            dev_dir.as_deref(),
            &|path| path.exists(),
        )
    }

    /// Whisper models from the preferred one down (the out-of-memory ladder).
    pub fn ladder(&self) -> Vec<String> {
        match &self.whisper {
            // An explicit choice leads. `base` stays as the one fallback unless
            // the choice is already that small (or smaller).
            Some(first) => {
                let mut ladder = vec![first.clone()];
                if !matches!(first.as_str(), "base" | "base.en" | "tiny" | "tiny.en") {
                    ladder.push(DEFAULT_LADDER[DEFAULT_LADDER.len() - 1].to_owned());
                }
                ladder.dedup();
                ladder
            }
            None => DEFAULT_LADDER.iter().map(|m| (*m).to_owned()).collect(),
        }
    }

    /// Why the sidecar cannot be started, if it cannot.
    pub fn absent(&self) -> Option<String> {
        if !self.enabled {
            return Some(TURNED_OFF.to_owned());
        }
        self.voice_dir.as_ref().err().cloned()
    }

    /// The command that starts the sidecar with `whisper`.
    pub fn launch_spec(&self, whisper: &str) -> Result<LaunchSpec, String> {
        if let Some(why) = self.absent() {
            return Err(why);
        }
        let cwd = self.voice_dir.clone()?;
        let (program, mut args): (OsString, Vec<OsString>) = match &self.python {
            Some(python) => (python.into(), vec!["main.py".into()]),
            None => (
                "uv".into(),
                ["run", "--offline", "--no-project", "python", "main.py"]
                    .map(OsString::from)
                    .to_vec(),
            ),
        };
        args.push("--whisper-model".into());
        args.push(whisper.into());
        if let Some(dir) = &self.wake_model_dir {
            args.push("--wake-model-dir".into());
            args.push(dir.into());
        }
        if let Some(dir) = &self.piper_dir {
            args.push("--piper-dir".into());
            args.push(dir.into());
        }
        if let Some(device) = &self.mic_device {
            args.push("--mic-device".into());
            args.push(device.into());
        }
        args.extend(self.extra_args.iter().map(OsString::from));
        Ok(LaunchSpec {
            program,
            args,
            cwd,
            env: vec![
                ("PYTHONUNBUFFERED".into(), "1".into()),
                ("PYTHONUTF8".into(), "1".into()),
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn read(
        vars: &[(&str, &str)],
        exe: Option<&str>,
        dev: Option<&str>,
        present: &[&str],
    ) -> VoiceConfig {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let present: Vec<PathBuf> = present.iter().map(PathBuf::from).collect();
        VoiceConfig::read(
            &|name| vars.get(name).cloned(),
            exe.map(Path::new),
            dev.map(Path::new),
            &|path| present.iter().any(|p| p == path),
        )
    }

    fn args(spec: &LaunchSpec) -> Vec<String> {
        spec.args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_default_runs_the_sidecar_through_uv_offline_in_its_folder() {
        let cfg = read(&[], None, Some("D:/repo/voice"), &["D:/repo/voice/main.py"]);
        let spec = cfg.launch_spec("small").unwrap();
        assert_eq!(spec.program, "uv");
        assert_eq!(
            args(&spec),
            [
                "run",
                "--offline",
                "--no-project",
                "python",
                "main.py",
                "--whisper-model",
                "small"
            ]
        );
        assert_eq!(spec.cwd, PathBuf::from("D:/repo/voice"));
        assert!(spec.env.iter().any(|(k, v)| k == "PYTHONUTF8" && v == "1"));
    }

    #[test]
    fn a_python_override_skips_uv() {
        let cfg = read(
            &[("NIVA_VOICE_PYTHON", "D:/v/.venv/Scripts/python.exe")],
            None,
            Some("D:/repo/voice"),
            &["D:/repo/voice/main.py"],
        );
        let spec = cfg.launch_spec("base").unwrap();
        assert_eq!(spec.program, "D:/v/.venv/Scripts/python.exe");
        assert_eq!(args(&spec)[0], "main.py");
        assert!(!args(&spec).contains(&"uv".to_owned()));
    }

    #[test]
    fn the_models_folders_and_extra_options_are_passed_on() {
        let cfg = read(
            &[
                ("NIVA_WAKE_MODEL_DIR", "C:/wake"),
                ("NIVA_PIPER_DIR", "C:/piper"),
                ("NIVA_VOICE_ARGS", "--no-edge  --verbose"),
            ],
            None,
            Some("D:/repo/voice"),
            &["D:/repo/voice/main.py"],
        );
        let line = args(&cfg.launch_spec("small").unwrap()).join(" ");
        assert!(line.contains("--wake-model-dir C:/wake"), "{line}");
        assert!(line.contains("--piper-dir C:/piper"), "{line}");
        assert!(line.ends_with("--no-edge --verbose"), "{line}");
    }

    #[test]
    fn the_voice_dir_variable_wins_and_must_hold_main_py() {
        let cfg = read(
            &[("NIVA_VOICE_DIR", "E:/mine")],
            Some("C:/app"),
            Some("D:/repo/voice"),
            &["C:/app/voice/main.py", "D:/repo/voice/main.py"],
        );
        let why = cfg.launch_spec("small").unwrap_err();
        assert!(why.contains("E:/mine"), "{why}");
        let cfg = read(
            &[("NIVA_VOICE_DIR", "E:/mine")],
            None,
            None,
            &["E:/mine/main.py"],
        );
        assert_eq!(
            cfg.launch_spec("small").unwrap().cwd,
            PathBuf::from("E:/mine")
        );
    }

    #[test]
    fn the_folder_next_to_the_app_comes_before_the_repository_one() {
        let cfg = read(
            &[],
            Some("C:/app"),
            Some("D:/repo/voice"),
            &["C:/app/voice/main.py", "D:/repo/voice/main.py"],
        );
        assert_eq!(
            cfg.launch_spec("small").unwrap().cwd,
            PathBuf::from("C:/app/voice")
        );
    }

    #[test]
    fn a_release_build_has_no_repository_fallback() {
        let cfg = read(&[], Some("C:/app"), None, &[]);
        let why = cfg.launch_spec("small").unwrap_err();
        assert!(why.contains("C:/app"), "{why}");
        assert!(cfg.absent().is_some());
    }

    #[test]
    fn voice_can_be_turned_off() {
        for value in ["off", "OFF", "0", "false", "no"] {
            let cfg = read(
                &[("NIVA_VOICE", value)],
                None,
                Some("D:/repo/voice"),
                &["D:/repo/voice/main.py"],
            );
            assert!(cfg.launch_spec("small").is_err(), "{value}");
        }
        let cfg = read(
            &[("NIVA_VOICE", "on")],
            None,
            Some("D:/repo/voice"),
            &["D:/repo/voice/main.py"],
        );
        assert!(cfg.launch_spec("small").is_ok());
    }

    #[test]
    fn the_ladder_is_small_then_base_and_an_override_leads_it() {
        let present = &["D:/repo/voice/main.py"];
        let dev = Some("D:/repo/voice");
        assert_eq!(read(&[], None, dev, present).ladder(), ["small", "base"]);
        assert_eq!(
            read(&[("NIVA_WHISPER_MODEL", "D:/m/large")], None, dev, present).ladder(),
            ["D:/m/large", "base"]
        );
        assert_eq!(
            read(&[("NIVA_WHISPER_MODEL", "base")], None, dev, present).ladder(),
            ["base"]
        );
        assert_eq!(
            read(&[("NIVA_WHISPER_MODEL", "small")], None, dev, present).ladder(),
            ["small", "base"]
        );
        assert_eq!(
            read(&[("NIVA_WHISPER_MODEL", "tiny")], None, dev, present).ladder(),
            ["tiny"]
        );
    }

    #[test]
    fn a_microphone_name_with_spaces_stays_one_argument() {
        let cfg = read(
            &[("NIVA_MIC_DEVICE", "Microphone (USB Audio)")],
            None,
            Some("D:/repo/voice"),
            &["D:/repo/voice/main.py"],
        );
        let spec = cfg.launch_spec("small").unwrap();
        let a = args(&spec);
        let at = a.iter().position(|x| x == "--mic-device").unwrap();
        assert_eq!(a[at + 1], "Microphone (USB Audio)");
    }

    #[test]
    fn turned_off_has_its_own_reason() {
        let cfg = read(
            &[("NIVA_VOICE", "off")],
            None,
            Some("D:/repo/voice"),
            &["D:/repo/voice/main.py"],
        );
        assert_eq!(cfg.launch_spec("small").unwrap_err(), TURNED_OFF);
    }

    #[test]
    fn blank_variables_count_as_unset() {
        let cfg = read(
            &[("NIVA_VOICE_PYTHON", "  "), ("NIVA_PIPER_DIR", "")],
            None,
            Some("D:/repo/voice"),
            &["D:/repo/voice/main.py"],
        );
        let spec = cfg.launch_spec("small").unwrap();
        assert_eq!(spec.program, "uv");
        assert!(!args(&spec).contains(&"--piper-dir".to_owned()));
    }
}
