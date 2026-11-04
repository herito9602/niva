//! The `niva-eval` command line.
//!
//! ```text
//! cargo run -p niva --features eval --bin niva-eval -- --runs 3 --out eval/results/E1-run
//! ```
//!
//! It needs Ollama running with the product model. It waits for Ollama (with
//! the app's backoff, a few attempts), warms the model up, runs every case of
//! `eval/commands.yaml` per run through the real assistant path with a fake PC,
//! and writes one raw JSON per run plus the combined per-case summary
//! (`summary.json`, `summary.md`). The exit code is 0 when every gate passes,
//! 1 when a gate fails, 2 when the run could not be made.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use niva_core::ports::{Clock, LlmProvider};
use niva_core::prompt;
use niva_core::replies::Lang;
use serde_json::json;

use super::cases::{self, Case};
use super::report::{self, Combined, Meta, Settings};
use super::runner::{self, Row};
use crate::adapters::ollama::health::{StartUpError, start_up};
use crate::adapters::ollama::wire::{KEEP_ALIVE, NUM_CTX};
use crate::adapters::ollama::{MODEL, OllamaProvider};
use crate::adapters::system_clock::SystemClock;

const USAGE: &str = "niva-eval: E1 tool-choice evaluation through the real assistant path

Usage: niva-eval [--runs N] [--pause SECS] [--out DIR] [--cases FILE] [--limit N] [--model NAME]

  --runs N      full passes over the cases (default 1)
  --pause SECS  wait between runs (a laptop GPU that heats up slows the model down;
                the pause tells that apart from a regression). Default 0
  --out DIR     where the JSON and markdown go (default eval/results/<timestamp>)
  --cases FILE  default eval/commands.yaml
  --limit N     only the first N cases (for a quick look; never for a gate)
  --model NAME  another Ollama model, for comparisons (default the product model)

Needs Ollama running locally. Exit code: 0 all gates pass, 1 a gate failed, 2 could not run.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub runs: usize,
    pub pause: Duration,
    pub out: Option<PathBuf>,
    pub cases: Option<PathBuf>,
    pub limit: Option<usize>,
    pub model: String,
}

/// `Ok(None)` means `--help` was asked for.
pub fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut options = Options {
        runs: 1,
        pause: Duration::ZERO,
        out: None,
        cases: None,
        limit: None,
        model: MODEL.into(),
    };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match flag.as_str() {
            "--help" | "-h" => return Ok(None),
            "--runs" => {
                options.runs = value("--runs")?
                    .parse()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or("--runs needs a whole number of at least 1")?;
            }
            "--pause" => {
                options.pause = Duration::from_secs(
                    value("--pause")?
                        .parse()
                        .map_err(|_| "--pause needs a whole number of seconds")?,
                );
            }
            "--limit" => {
                options.limit = Some(
                    value("--limit")?
                        .parse()
                        .ok()
                        .filter(|n| *n >= 1)
                        .ok_or("--limit needs a whole number of at least 1")?,
                );
            }
            "--out" => options.out = Some(value("--out")?.into()),
            "--cases" => options.cases = Some(value("--cases")?.into()),
            "--model" => options.model = value("--model")?,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(Some(options))
}

/// The repository root: the nearest parent of `start` holding `eval/commands.yaml`.
pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join("eval").join("commands.yaml").is_file())
        .map(Path::to_path_buf)
}

/// `20261104-1405`, for a default output folder.
pub fn stamp(clock: &dyn Clock) -> String {
    match clock.local_now() {
        Some(t) => format!(
            "{:04}{:02}{:02}-{:02}{:02}",
            t.year, t.month, t.day, t.hour, t.minute
        ),
        None => "undated".into(),
    }
}

fn pretty(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| e.to_string())
}

/// Writes `run-N.json` for each run, `summary.json` and `summary.md`.
pub fn write_outputs(dir: &Path, runs: &[Vec<Row>], combined: &Combined) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (index, rows) in runs.iter().enumerate() {
        let per_run = &combined.per_run[index];
        let raw = json!({
            "run": index + 1,
            "model": combined.meta.model,
            "digest": combined.meta.digest,
            "ollama_version": combined.meta.ollama_version,
            "settings": combined.meta.settings,
            "started": combined.meta.started,
            "summary": per_run.summary,
            "gates": per_run.gates,
            "rows": rows,
        });
        write(dir.join(format!("run-{}.json", index + 1)), &pretty(&raw)?)?;
    }
    write(dir.join("summary.json"), &pretty(combined)?)?;
    write(dir.join("summary.md"), &report::render_markdown(combined))
}

fn write(path: PathBuf, text: &str) -> Result<(), String> {
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn one_line(row: &Row) -> String {
    let flag = if row.tool_pass { "PASS" } else { "FAIL" };
    let calls = if row.calls.is_empty() {
        "(no tool)".to_string()
    } else {
        row.calls
            .iter()
            .map(|c| c.signature())
            .collect::<Vec<_>>()
            .join(" + ")
    };
    let extra = if row.unrequested_risky.is_empty() {
        ""
    } else {
        "  !! UNREQUESTED RISKY"
    };
    let rejected = if row.plan_rejected {
        "  !! REJECTED BY VALIDATION"
    } else {
        ""
    };
    format!(
        "{flag} {:<28} {:>6.0} ms  {calls}{extra}{rejected}",
        row.id, row.latency_ms
    )
}

fn load_cases(options: &Options, root: Option<&Path>) -> Result<Vec<Case>, String> {
    let path = match (&options.cases, root) {
        (Some(path), _) => path.clone(),
        (None, Some(root)) => root.join("eval").join("commands.yaml"),
        (None, None) => return Err("cannot find eval/commands.yaml; pass --cases".into()),
    };
    let mut cases = cases::load(&path)?;
    cases::validate(&cases)?;
    if let Some(limit) = options.limit {
        cases.truncate(limit);
    }
    Ok(cases)
}

/// Runs the evaluation; returns the process exit code.
pub fn run(args: Vec<String>) -> i32 {
    let options = match parse_args(&args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(why) => {
            eprintln!("niva-eval: {why}\n\n{USAGE}");
            return 2;
        }
    };
    let root = std::env::current_dir()
        .ok()
        .and_then(|dir| find_repo_root(&dir));
    let cases = match load_cases(&options, root.as_deref()) {
        Ok(cases) => cases,
        Err(why) => {
            eprintln!("niva-eval: {why}");
            return 2;
        }
    };
    if options.limit.is_some() {
        eprintln!("niva-eval: --limit is for a quick look; its numbers are not a gate result.");
    }

    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let provider = Arc::new(OllamaProvider::new().with_model(&options.model));
    let log = |line: String| eprintln!("{line}");
    let started = match start_up(&provider, Some(3), &std::thread::sleep, &log) {
        Ok(started) => started,
        Err(StartUpError::NotReady(health)) => {
            eprintln!("niva-eval: {}", health.message(provider.model()));
            return 2;
        }
        Err(StartUpError::WarmUp(why)) => {
            eprintln!("niva-eval: warm-up failed: {why}");
            return 2;
        }
    };
    let loaded = provider.loaded().ok().flatten();
    let meta = Meta {
        model: options.model.clone(),
        digest: provider.digest().ok().flatten(),
        ollama_version: provider.version().ok(),
        settings: Settings {
            think: false,
            temperature: 0,
            num_ctx: NUM_CTX,
            keep_alive: KEEP_ALIVE.into(),
        },
        started: stamp(&*clock),
        runs: options.runs,
        cases: cases.len(),
        warm_up_ms: Some(started.warm_up.as_secs_f64() * 1000.0),
        offload: started.offload.map(|o| format!("{o:?}")),
        model_bytes: loaded.as_ref().map(|m| m.size),
        model_vram_bytes: loaded.as_ref().map(|m| m.size_vram),
        system_prompt_en: prompt::system_prompt(Lang::En, clock.local_now().as_ref()),
    };

    let llm: Arc<dyn LlmProvider> = provider;
    let mut runs: Vec<Vec<Row>> = Vec::new();
    for run in 1..=options.runs {
        if run > 1 && !options.pause.is_zero() {
            eprintln!(
                "
pausing {} s before run {run}",
                options.pause.as_secs()
            );
            std::thread::sleep(options.pause);
        }
        eprintln!("\n== run {run}/{} ==", options.runs);
        let rows = runner::run_all(&llm, &clock, &cases, &mut |n, total, row| {
            eprintln!("[{n:>2}/{total}] {}", one_line(row));
        });
        runs.push(rows);
    }

    let combined = report::combine(meta, &runs);
    let dir = options.out.clone().unwrap_or_else(|| {
        root.clone()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("eval")
            .join("results")
            .join(stamp(&*clock))
    });
    if let Err(why) = write_outputs(&dir, &runs, &combined) {
        eprintln!("niva-eval: could not write the results: {why}");
        return 2;
    }
    println!("{}", report::render_markdown(&combined));
    println!("Results written to {}", dir.display());
    if combined.all_gates_pass { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults_are_one_run_of_the_product_model() {
        let options = parse_args(&[]).unwrap().unwrap();
        assert_eq!(options.runs, 1);
        assert_eq!(options.pause, Duration::ZERO);
        assert_eq!(options.model, MODEL);
        assert!(options.out.is_none() && options.cases.is_none() && options.limit.is_none());
    }

    #[test]
    fn every_option_is_read() {
        let options = parse_args(&args(&[
            "--runs", "3", "--pause", "90", "--out", "x/y", "--cases", "c.yaml", "--limit", "5",
            "--model", "m:1b",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(options.runs, 3);
        assert_eq!(options.pause, Duration::from_secs(90));
        assert_eq!(options.out, Some("x/y".into()));
        assert_eq!(options.cases, Some("c.yaml".into()));
        assert_eq!(options.limit, Some(5));
        assert_eq!(options.model, "m:1b");
    }

    #[test]
    fn bad_input_is_an_error_and_help_is_not() {
        for bad in [
            &["--runs"][..],
            &["--runs", "0"],
            &["--runs", "many"],
            &["--limit", "0"],
            &["--pause", "-1"],
            &["--pause"],
            &["--wat"],
            &["--model"],
        ] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
        }
        assert_eq!(parse_args(&args(&["--help"])).unwrap(), None);
        assert_eq!(parse_args(&args(&["-h"])).unwrap(), None);
    }

    #[test]
    fn the_repo_root_is_found_from_any_folder_below_it() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let root = find_repo_root(&here).expect("root");
        assert!(root.join("eval").join("commands.yaml").is_file());
        assert!(find_repo_root(Path::new("/")).is_none() || cfg!(not(windows)));
    }

    #[test]
    fn the_stamp_is_sortable_and_zero_padded() {
        struct At;
        impl Clock for At {
            fn now(&self) -> std::time::SystemTime {
                std::time::SystemTime::UNIX_EPOCH
            }
            fn local_now(&self) -> Option<niva_core::ports::LocalDateTime> {
                Some(niva_core::ports::LocalDateTime {
                    year: 2026,
                    month: 1,
                    day: 2,
                    hour: 3,
                    minute: 4,
                })
            }
        }
        assert_eq!(stamp(&At), "20260102-0304");
    }

    #[test]
    fn the_real_cases_load_and_a_limit_truncates() {
        let root = find_repo_root(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let mut options = parse_args(&[]).unwrap().unwrap();
        assert_eq!(load_cases(&options, Some(&root)).unwrap().len(), 51);
        options.limit = Some(7);
        assert_eq!(load_cases(&options, Some(&root)).unwrap().len(), 7);
        assert!(load_cases(&options, None).is_err());
    }

    #[test]
    fn the_outputs_are_one_json_per_run_plus_the_combined_summary() {
        use crate::eval::runner::CallRecord;
        let row = |pass: bool| Row {
            id: "a".into(),
            category: "open_app".into(),
            lang: "en".into(),
            risky_expected: false,
            utterance: "x".into(),
            calls: vec![CallRecord {
                name: "open_app".into(),
                args: json!({"name": "x"}),
            }],
            text: String::new(),
            tool_pass: pass,
            unrequested_risky: Vec::new(),
            reply_lang_detected: None,
            reply_lang_ok: None,
            latency_ms: 500.0,
            error: None,
            plan: "run-safe".into(),
            plan_text: String::new(),
            plan_rejected: false,
            reply: "Opened x.".into(),
            pc_calls: Vec::new(),
        };
        let runs = vec![vec![row(true)], vec![row(false)]];
        let meta = Meta {
            model: "m".into(),
            digest: None,
            ollama_version: None,
            settings: Settings {
                think: false,
                temperature: 0,
                num_ctx: 4096,
                keep_alive: "30m".into(),
            },
            started: "t".into(),
            runs: 2,
            cases: 1,
            warm_up_ms: None,
            offload: None,
            model_bytes: None,
            model_vram_bytes: None,
            system_prompt_en: String::new(),
        };
        let combined = report::combine(meta, &runs);
        let dir = std::env::temp_dir().join(format!("niva-eval-test-{}", std::process::id()));
        write_outputs(&dir, &runs, &combined).unwrap();
        for name in ["run-1.json", "run-2.json", "summary.json", "summary.md"] {
            assert!(dir.join(name).is_file(), "{name}");
        }
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("run-2.json")).unwrap())
                .unwrap();
        assert_eq!(raw["run"], 2);
        assert_eq!(raw["rows"][0]["tool_pass"], false);
        assert_eq!(raw["rows"][0]["reply"], "Opened x.");
        let summary: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("summary.json")).unwrap())
                .unwrap();
        assert_eq!(summary["varied_case_ids"], json!(["a"]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_progress_line_flags_the_two_things_that_must_be_zero() {
        let row = |risky: bool, rejected: bool| {
            let mut r = crate::eval::runner::Row {
                id: "adv-es-me-voy-a-dormir".into(),
                category: "adversarial".into(),
                lang: "es".into(),
                risky_expected: false,
                utterance: String::new(),
                calls: Vec::new(),
                text: String::new(),
                tool_pass: false,
                unrequested_risky: Vec::new(),
                reply_lang_detected: None,
                reply_lang_ok: None,
                latency_ms: 812.0,
                error: None,
                plan: "none".into(),
                plan_text: String::new(),
                plan_rejected: rejected,
                reply: String::new(),
                pc_calls: Vec::new(),
            };
            if risky {
                r.unrequested_risky.push(crate::eval::runner::CallRecord {
                    name: "power_action".into(),
                    args: json!({"action": "sleep"}),
                });
            }
            r
        };
        assert!(one_line(&row(true, false)).contains("UNREQUESTED RISKY"));
        assert!(one_line(&row(false, true)).contains("REJECTED BY VALIDATION"));
        let calm = one_line(&row(false, false));
        assert!(calm.starts_with("FAIL adv-es-me-voy-a-dormir"), "{calm}");
        assert!(!calm.contains("!!"));
    }
}
