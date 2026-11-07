//! The process boundary: how the sidecar is started, read, written and taken
//! down, behind two small traits so the supervisor is tested with fake streams.
//!
//! Windows keeps the microphone busy for as long as the sidecar lives, so a
//! sidecar that outlives the app is a real problem. Three layers prevent it:
//!
//! 1. The host closes the sidecar's stdin (and sends `shutdown` first). The
//!    sidecar ends on EOF (`voice/main.py`: the reader queues `eof`).
//! 2. A sidecar that does not exit within the grace is killed, together with
//!    everything it started (`uv run` puts a launcher in front of Python), by
//!    ending a Windows job object.
//! 3. Every sidecar belongs to a job object with `KILL_ON_JOB_CLOSE`, so even a
//!    host that is killed without running any code takes the sidecar with it.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};

use super::config::LaunchSpec;
use super::wire::MAX_LINE_BYTES;

/// A running sidecar, as the supervisor sees it.
pub trait Proc: Send {
    /// `Ok(Some(code))` once it ended; `code` is `None` when it was killed and
    /// has no exit code.
    fn try_exit(&mut self) -> io::Result<Option<Option<i32>>>;
    /// Ends it and everything it started, and waits for it. Idempotent.
    fn kill_tree(&mut self);
}

/// The pipes and the handle of one sidecar process.
pub struct Spawned {
    pub stdin: Box<dyn Write + Send>,
    pub stdout: Box<dyn Read + Send>,
    pub stderr: Box<dyn Read + Send>,
    pub proc: Box<dyn Proc>,
}

/// Starts sidecars. `whisper` is the model for this run (the OOM ladder).
pub trait Launcher: Send + Sync {
    fn spawn(&self, whisper: &str) -> io::Result<Spawned>;
}

/// Builds the command line for a Whisper model, or says why it cannot.
type Resolve = Box<dyn Fn(&str) -> Result<LaunchSpec, String> + Send + Sync>;

/// Real launcher: the command line comes from [`LaunchSpec`].
pub struct CommandLauncher {
    resolve: Resolve,
}

impl CommandLauncher {
    /// `resolve` is called at every spawn, so a restart sees files that
    /// appeared since (the "Restart voice" entry after an install).
    pub fn new(
        resolve: impl Fn(&str) -> Result<LaunchSpec, String> + Send + Sync + 'static,
    ) -> Self {
        CommandLauncher {
            resolve: Box::new(resolve),
        }
    }
}

impl Launcher for CommandLauncher {
    fn spawn(&self, whisper: &str) -> io::Result<Spawned> {
        let spec = (self.resolve)(whisper).map_err(|why| {
            // "Turned off" is the user's choice, not a missing install.
            let kind = if why == super::config::TURNED_OFF {
                io::ErrorKind::Unsupported
            } else {
                io::ErrorKind::NotFound
            };
            io::Error::new(kind, why)
        })?;
        let mut command = Command::new(&spec.program);
        command.args(&spec.args).current_dir(&spec.cwd);
        for (key, value) in &spec.env {
            command.env(key, value);
        }
        spawn_command(command)
    }
}

/// Pipes `command`, starts it without a console window and puts it in a job.
pub fn spawn_command(mut command: Command) -> io::Result<Spawned> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("no stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("no stderr"))?;
    #[cfg(windows)]
    let job = job::Job::kill_on_close().filter(|job| job.assign(&child));
    #[cfg(windows)]
    if job.is_none() {
        // The process runs, but killing the app would not take it along.
        eprintln!("[niva-voice] could not put the sidecar in a job object: it may outlive the app");
    }
    Ok(Spawned {
        stdin: Box::new(stdin),
        stdout: Box::new(stdout),
        stderr: Box::new(stderr),
        proc: Box::new(ChildProc {
            child,
            #[cfg(windows)]
            job,
        }),
    })
}

struct ChildProc {
    child: Child,
    #[cfg(windows)]
    job: Option<job::Job>,
}

impl Proc for ChildProc {
    fn try_exit(&mut self) -> io::Result<Option<Option<i32>>> {
        Ok(self.child.try_wait()?.map(|status| status.code()))
    }

    fn kill_tree(&mut self) {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(windows)]
mod job {
    //! A Windows job object that ends its processes when it is closed.

    use std::mem::{size_of, zeroed};
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::ptr::null;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };

    pub struct Job(HANDLE);

    // SAFETY: a job handle is a kernel object handle, usable from any thread.
    unsafe impl Send for Job {}

    impl Job {
        /// `None` if Windows refuses (the sidecar then runs without the net).
        pub fn kill_on_close() -> Option<Job> {
            // SAFETY: plain Win32 calls with a handle owned by `Job` and a
            // zeroed limit structure of the size the call is told.
            unsafe {
                let handle = CreateJobObjectW(null(), null());
                if handle.is_null() {
                    return None;
                }
                let job = Job(handle);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                (ok != 0).then_some(job)
            }
        }

        pub fn assign(&self, child: &Child) -> bool {
            // SAFETY: both handles are valid for the duration of the call.
            unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle()) != 0 }
        }

        pub fn terminate(&self) {
            // SAFETY: the handle is valid; the exit code is arbitrary.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: closing the handle this value owns, once. With
            // KILL_ON_JOB_CLOSE this ends any process still in the job.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// One line read from a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Text(String),
    /// The line went over [`MAX_LINE_BYTES`]; the rest of it was thrown away.
    TooLong,
}

/// Reads `reader` line by line until it ends, never holding more than
/// [`MAX_LINE_BYTES`] of one line. Empty lines are skipped; a last line with no
/// newline still counts.
pub fn read_lines(reader: impl Read, mut each: impl FnMut(Line)) {
    let mut reader = BufReader::new(reader);
    let mut line: Vec<u8> = Vec::new();
    let mut overflow = false;
    loop {
        let chunk = match reader.fill_buf() {
            Ok(chunk) => chunk,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        if chunk.is_empty() {
            break;
        }
        let end = chunk.iter().position(|&b| b == b'\n');
        let part = &chunk[..end.unwrap_or(chunk.len())];
        if line.len() + part.len() > MAX_LINE_BYTES {
            overflow = true;
            line.clear();
        } else if !overflow {
            line.extend_from_slice(part);
        }
        let used = end.map_or(chunk.len(), |i| i + 1);
        reader.consume(used);
        if end.is_some() {
            finish(&mut line, &mut overflow, &mut each);
        }
    }
    finish(&mut line, &mut overflow, &mut each);
}

fn finish(line: &mut Vec<u8>, overflow: &mut bool, each: &mut impl FnMut(Line)) {
    if *overflow {
        each(Line::TooLong);
    } else {
        let text = String::from_utf8_lossy(line);
        let text = text.trim_end_matches('\r');
        if !text.trim().is_empty() {
            each(Line::Text(text.to_owned()));
        }
    }
    line.clear();
    *overflow = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(bytes: &[u8]) -> Vec<Line> {
        let mut out = Vec::new();
        read_lines(bytes, |l| out.push(l));
        out
    }

    fn text(s: &str) -> Line {
        Line::Text(s.to_owned())
    }

    #[test]
    fn splits_lines_and_skips_empty_ones() {
        assert_eq!(
            lines(b"a\n\nb\r\n  \nc"),
            vec![text("a"), text("b"), text("c")]
        );
    }

    #[test]
    fn invalid_utf8_does_not_stop_the_reader() {
        let got = lines(b"ok\n\xff\xfe broken\nnext\n");
        assert_eq!(got.len(), 3);
        assert_eq!(got[2], text("next"));
    }

    #[test]
    fn a_runaway_line_is_dropped_and_the_next_one_still_arrives() {
        let mut input = vec![b'x'; MAX_LINE_BYTES + 10];
        input.extend_from_slice(b"\nafter\n");
        assert_eq!(lines(&input), vec![Line::TooLong, text("after")]);
    }

    #[test]
    fn a_line_exactly_at_the_limit_is_kept() {
        let mut input = vec![b'y'; MAX_LINE_BYTES];
        input.push(b'\n');
        let got = lines(&input);
        assert!(matches!(&got[..], [Line::Text(t)] if t.len() == MAX_LINE_BYTES));
    }

    #[test]
    fn a_line_split_across_reads_is_joined() {
        struct Drip<'a>(&'a [u8]);
        impl Read for Drip<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = self.0.len().min(buf.len()).min(3);
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        let mut out = Vec::new();
        read_lines(Drip(b"{\"type\":\"wake\"}\nsecond\n"), |l| out.push(l));
        assert_eq!(out, vec![text("{\"type\":\"wake\"}"), text("second")]);
    }

    /// Real processes: nothing here is the sidecar, only Windows' own tools.
    #[cfg(windows)]
    mod real {
        use std::time::{Duration, Instant};

        use super::*;

        fn wait_exit(proc: &mut dyn Proc, within: Duration) -> Option<Option<i32>> {
            let until = Instant::now() + within;
            while Instant::now() < until {
                if let Some(code) = proc.try_exit().unwrap() {
                    return Some(code);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            None
        }

        #[test]
        fn closing_stdin_ends_a_process_that_reads_it() {
            // `more` copies stdin to stdout until EOF, as the sidecar reads
            // commands until EOF.
            let mut spawned = spawn_command(Command::new("more.com")).unwrap();
            spawned.stdin.write_all(b"hello\r\n").unwrap();
            drop(spawned.stdin);
            assert_eq!(
                wait_exit(&mut *spawned.proc, Duration::from_secs(10)),
                Some(Some(0))
            );
            let mut out = String::new();
            spawned.stdout.read_to_string(&mut out).unwrap();
            assert!(out.contains("hello"));
        }

        /// The two tree tests count `ping.exe` processes: one at a time.
        static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

        fn pings() -> usize {
            let out = Command::new("tasklist")
                .args(["/FI", "IMAGENAME eq ping.exe", "/NH"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .to_lowercase()
                .matches("ping.exe")
                .count()
        }

        /// Starts `cmd`, which starts `ping` (a tree), and waits until `ping` runs.
        fn spawn_tree() -> (Spawned, usize) {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping -n 60 127.0.0.1 > nul"]);
            let spawned = spawn_command(command).unwrap();
            let until = Instant::now() + Duration::from_secs(5);
            while pings() == 0 && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
            }
            let before = pings();
            assert!(before >= 1, "the child never started ping");
            (spawned, before)
        }

        #[test]
        fn kill_tree_ends_a_process_that_ignores_its_stdin() {
            let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
            let (mut spawned, before) = spawn_tree();
            assert_eq!(spawned.proc.try_exit().unwrap(), None);
            let started = Instant::now();
            spawned.proc.kill_tree();
            assert!(started.elapsed() < Duration::from_secs(10));
            assert!(spawned.proc.try_exit().unwrap().is_some());
            std::thread::sleep(Duration::from_millis(300));
            assert!(pings() < before, "the grandchild survived the kill");
            // Idempotent.
            spawned.proc.kill_tree();
        }

        #[test]
        fn dropping_the_handle_takes_the_whole_tree_with_it() {
            // The net under a host that dies without running its shutdown: the
            // job object closes with the handle and ends what is in it.
            let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
            let (spawned, before) = spawn_tree();
            drop(spawned);
            std::thread::sleep(Duration::from_millis(500));
            assert!(pings() < before, "the tree outlived its handle");
        }

        #[test]
        fn a_missing_program_is_an_error_not_a_panic() {
            let err = spawn_command(Command::new("niva-no-such-program-xyz"));
            assert!(err.is_err());
        }
    }
}
