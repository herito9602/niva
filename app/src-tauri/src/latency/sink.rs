//! Where the latency lines go: a bounded JSON Lines file in the app's local
//! data folder, written by a thread of its own so a slow or failing disk can
//! never hold up (or break) a turn.
//!
//! - Path: `NIVA_LATENCY_LOG` if it is set and not empty, else
//!   `%LOCALAPPDATA%\Niva\latency.jsonl`. With neither there is no log.
//! - Bound: when the file would pass [`MAX_BYTES`] it is renamed to
//!   `<name>.1` (replacing the previous `.1`) and a new file starts, so the
//!   disk holds at most about twice that, and the newest turns are always
//!   kept.
//! - Failure: every error is swallowed (reported once on stderr, by kind
//!   only), and the next line tries again.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::thread;

/// The override for the log's path.
pub const ENV_PATH: &str = "NIVA_LATENCY_LOG";

/// Size at which the file rolls over: ~1 MiB, about 2,500 records.
pub const MAX_BYTES: u64 = 1024 * 1024;

/// Where the log lives. `var` reads an environment variable.
pub fn log_path(var: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(over) = var(ENV_PATH).filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(over));
    }
    let base = var("LOCALAPPDATA").filter(|value| !value.is_empty())?;
    Some(PathBuf::from(base).join("Niva").join("latency.jsonl"))
}

/// The path from the real environment.
pub fn default_log_path() -> Option<PathBuf> {
    log_path(&|name| std::env::var_os(name))
}

/// The file with its bound. Not thread-safe by itself: one writer owns it.
pub struct FileSink {
    path: PathBuf,
    max_bytes: u64,
    file: Option<File>,
    size: u64,
}

impl FileSink {
    pub fn new(path: PathBuf, max_bytes: u64) -> Self {
        FileSink {
            path,
            max_bytes,
            file: None,
            size: 0,
        }
    }

    /// The previous file, `<name>.1`.
    pub fn rolled_path(path: &Path) -> PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push(".1");
        PathBuf::from(name)
    }

    /// Appends one line (a newline is added). On an error the file handle is
    /// dropped so the next call opens it again.
    pub fn append(&mut self, line: &str) -> io::Result<()> {
        let result = self.try_append(line);
        if result.is_err() {
            self.file = None;
        }
        result
    }

    fn try_append(&mut self, line: &str) -> io::Result<()> {
        let needed = u64::try_from(line.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        if self.file.is_none() {
            self.open()?;
        }
        // An empty file never rolls: one line longer than the bound is still
        // written whole.
        if self.size > 0 && self.size.saturating_add(needed) > self.max_bytes {
            // A roll that fails (the old file is held open by a reader, say)
            // must not cost the line: keep appending here and retry on the
            // next one.
            let _ = self.roll();
            if self.file.is_none() {
                self.open()?;
            }
        }
        let file = self.file.as_mut().ok_or(io::ErrorKind::Other)?;
        // One write call per line, so a crash leaves at most a torn last line.
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
        file.write_all(&bytes)?;
        self.size = self.size.saturating_add(needed);
        Ok(())
    }

    fn open(&mut self) -> io::Result<()> {
        if let Some(dir) = self.path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let size = file.metadata()?.len();
        self.file = Some(file);
        self.size = size;
        // A crash can leave the last line torn; end it, so the next record
        // starts on a line of its own (the reader skips the torn one).
        if size > 0 && !self.ends_with_newline()? {
            if let Some(file) = self.file.as_mut() {
                file.write_all(b"\n")?;
            }
            self.size = self.size.saturating_add(1);
        }
        Ok(())
    }

    fn ends_with_newline(&self) -> io::Result<bool> {
        let mut reader = File::open(&self.path)?;
        reader.seek(SeekFrom::End(-1))?;
        let mut last = [0u8; 1];
        reader.read_exact(&mut last)?;
        Ok(last[0] == b'\n')
    }

    fn roll(&mut self) -> io::Result<()> {
        // Close before the rename: Windows will not move an open file.
        self.file = None;
        let rolled = Self::rolled_path(&self.path);
        // `rename` replaces an existing target on Windows, so the old rolled
        // file is only lost when the new one is really in place.
        let moved = fs::rename(&self.path, &rolled);
        self.open()?;
        moved
    }
}

/// Starts the writer thread; the returned function queues a line for it and
/// never blocks or fails. `None` when the thread cannot start.
pub fn start(path: PathBuf) -> Option<Box<dyn Fn(String) + Send>> {
    start_with(FileSink::new(path, MAX_BYTES))
}

pub fn start_with(mut sink: FileSink) -> Option<Box<dyn Fn(String) + Send>> {
    let (tx, rx) = channel::<String>();
    thread::Builder::new()
        .name("niva-latency-log".into())
        .spawn(move || {
            let mut reported = false;
            for line in rx {
                if let Err(err) = sink.append(&line)
                    && !reported
                {
                    reported = true;
                    eprintln!(
                        "[niva] latency log: cannot write ({:?}); turns are not affected",
                        err.kind()
                    );
                }
            }
        })
        .ok()?;
    Some(sender(tx))
}

fn sender(tx: Sender<String>) -> Box<dyn Fn(String) + Send> {
    Box::new(move |line| {
        let _ = tx.send(line);
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("niva-latency-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn the_default_path_is_in_the_local_app_data_folder() {
        let got = log_path(&env(&[("LOCALAPPDATA", r"C:\Users\x\AppData\Local")]));
        assert_eq!(
            got,
            Some(PathBuf::from(
                r"C:\Users\x\AppData\Local\Niva\latency.jsonl"
            ))
        );
    }

    #[test]
    fn the_environment_override_wins() {
        let got = log_path(&env(&[
            ("NIVA_LATENCY_LOG", r"D:\tmp\run1.jsonl"),
            ("LOCALAPPDATA", r"C:\Users\x\AppData\Local"),
        ]));
        assert_eq!(got, Some(PathBuf::from(r"D:\tmp\run1.jsonl")));
    }

    #[test]
    fn an_empty_override_is_ignored_and_no_folder_means_no_log() {
        let got = log_path(&env(&[("NIVA_LATENCY_LOG", ""), ("LOCALAPPDATA", r"C:\L")]));
        assert_eq!(got, Some(PathBuf::from(r"C:\L\Niva\latency.jsonl")));
        assert_eq!(log_path(&env(&[])), None);
        assert_eq!(log_path(&env(&[("LOCALAPPDATA", "")])), None);
    }

    #[test]
    fn lines_are_appended_one_per_line_and_the_folder_is_created() {
        let root = dir("append");
        let path = root.join("deep").join("latency.jsonl");
        let mut sink = FileSink::new(path.clone(), MAX_BYTES);
        sink.append(r#"{"a":1}"#).unwrap();
        sink.append(r#"{"a":2}"#).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":1}\n{\"a\":2}\n");
        // A new sink on the same file continues it, and counts its size.
        let mut again = FileSink::new(path.clone(), MAX_BYTES);
        again.append(r#"{"a":3}"#).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 3);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_file_rolls_over_at_the_bound_and_keeps_only_one_old_file() {
        let root = dir("roll");
        let path = root.join("latency.jsonl");
        // 10 bytes per line with the newline; room for 3 lines.
        let mut sink = FileSink::new(path.clone(), 30);
        for n in 0..8 {
            sink.append(&format!("line-{n:04}")).unwrap();
        }
        let current = fs::read_to_string(&path).unwrap();
        let old = fs::read_to_string(FileSink::rolled_path(&path)).unwrap();
        assert!(
            current.len() <= 30 && old.len() <= 30,
            "{current:?} {old:?}"
        );
        // The newest line is in the current file, and nothing older than the
        // previous file survives.
        assert!(current.contains("line-0007"));
        assert!(!old.contains("line-0000") && !current.contains("line-0000"));
        assert!(!FileSink::rolled_path(&FileSink::rolled_path(&path)).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_roll_that_fails_keeps_appending_to_the_current_file_and_retries_later() {
        let root = dir("roll-fails");
        let path = root.join("latency.jsonl");
        fs::create_dir_all(&root).unwrap();
        // The rolled name is taken by a non-empty folder: the rename cannot work
        // (a stand-in for a reader holding `.1` open).
        let rolled = FileSink::rolled_path(&path);
        fs::create_dir_all(rolled.join("keep")).unwrap();
        let mut sink = FileSink::new(path.clone(), 30);
        for n in 0..6 {
            sink.append(&format!("line-{n:04}"))
                .expect("a failed roll must not drop the line");
        }
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 6, "{text:?}");
        // Once the obstacle is gone the next line rolls the oversized file.
        fs::remove_dir_all(&rolled).unwrap();
        sink.append("line-0006").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "line-0006\n");
        assert_eq!(fs::read_to_string(&rolled).unwrap().lines().count(), 6);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_torn_last_line_does_not_swallow_the_next_record() {
        let root = dir("torn");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("latency.jsonl");
        fs::write(&path, "{\"a\":1}\n{\"a\":2,\"pa").unwrap();
        let mut sink = FileSink::new(path.clone(), MAX_BYTES);
        sink.append("{\"a\":3}").unwrap();
        sink.append("{\"a\":4}").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text:?}");
        assert_eq!(lines[2], "{\"a\":3}");
        assert_eq!(lines[3], "{\"a\":4}");
        // A file that ends cleanly gets no blank line.
        let mut again = FileSink::new(path.clone(), MAX_BYTES);
        again.append("{\"a\":5}").unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("\n\n"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_line_longer_than_the_bound_is_still_written_whole() {
        let root = dir("long");
        let path = root.join("latency.jsonl");
        let mut sink = FileSink::new(path.clone(), 5);
        sink.append("0123456789").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "0123456789\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_path_that_cannot_be_written_is_an_error_not_a_panic_and_a_later_line_retries() {
        let root = dir("fail");
        fs::create_dir_all(&root).unwrap();
        // The log path is a directory: opening it for append fails.
        let blocked = root.join("latency.jsonl");
        fs::create_dir_all(&blocked).unwrap();
        let mut sink = FileSink::new(blocked.clone(), MAX_BYTES);
        assert!(sink.append("x").is_err());
        assert!(sink.append("x").is_err());
        // Once the obstacle is gone the same sink writes again.
        fs::remove_dir_all(&blocked).unwrap();
        sink.append("ok").unwrap();
        assert_eq!(fs::read_to_string(&blocked).unwrap(), "ok\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_writer_thread_queues_lines_and_survives_a_failing_file() {
        let root = dir("thread");
        fs::create_dir_all(&root).unwrap();
        let blocked = root.join("blocked");
        fs::create_dir_all(&blocked).unwrap();
        // Sending to a sink that cannot write returns at once and does not panic.
        let send = start_with(FileSink::new(blocked, MAX_BYTES)).unwrap();
        send("lost".into());
        let path = root.join("ok.jsonl");
        let send = start_with(FileSink::new(path.clone(), MAX_BYTES)).unwrap();
        send("one".into());
        send("two".into());
        let deadline = Instant::now() + Duration::from_secs(5);
        while fs::read_to_string(&path).map_or(true, |text| text.lines().count() < 2) {
            assert!(Instant::now() < deadline, "the writer never wrote");
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        let _ = fs::remove_dir_all(root);
    }
}
