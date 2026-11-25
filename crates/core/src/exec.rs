//! Safe process execution: always an argument vector (never a host shell), a
//! timeout, a controlled environment and cancellation of the whole process
//! group.

use crate::i18n::{tr, trc};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Default)]
pub struct CommandResult {
    pub argv: Vec<String>,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub duration: Duration,
}

impl CommandResult {
    pub fn ok(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }

    /// stdout + stderr, trimmed.
    pub fn combined(&self) -> String {
        let mut text = self.stdout.trim_end().to_string();
        let err = self.stderr.trim();
        if !err.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(err);
        }
        text
    }

    /// Last non-empty line, to show short errors.
    pub fn summary(&self) -> String {
        if self.timed_out {
            return tr("Timed out").into();
        }
        let combined = self.combined();
        combined.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(if self.ok() { trc("status", "OK") } else { tr("No output") }).trim().to_string()
    }

    pub fn into_result(self) -> Result<Self> {
        if self.ok() {
            Ok(self)
        } else {
            Err(Error::Command { summary: self.summary(), code: self.code })
        }
    }
}

/// Minimal, predictable environment: C locale so outputs can always be parsed
/// the same way.
pub fn base_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("LC_ALL", "C").env("LANG", "C");
    cmd.stdin(Stdio::null());
    // Own group: cancelling also kills the children (e.g. adb fork-server).
    cmd.process_group(0);
    // Because it is in another group, the child does not die with ADB Scan. If
    // the app closes abruptly (crash, kill), an orphaned `adb shell` keeps its
    // loop alive on the phone forever (telemetry every 0.5 s: CPU maxed out and
    // the phone hot). Linux sends it SIGTERM when the thread that created it
    // dies: that is why it must be created with [`spawn`].
    #[cfg(target_os = "linux")]
    {
        let parent = std::process::id() as libc::pid_t;
        // SAFETY: between fork and exec only system calls that are safe at that
        // point are made (prctl, getppid), without allocating memory.
        unsafe {
            cmd.pre_exec(move || {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                // ADB Scan died before the prctl: the signal would no longer
                // arrive.
                if libc::getppid() != parent {
                    return Err(std::io::Error::other("ADB Scan has already closed"));
                }
                Ok(())
            });
        }
    }
    cmd
}

/// Creates the process of a [`base_command`]. It is always created from the
/// same thread, which lives as long as the app: Linux notifies the child when
/// the *thread* that created it dies, and a worker thread that ends before the
/// process would kill it at the wrong time.
pub fn spawn(mut cmd: Command) -> std::io::Result<Child> {
    type Job = Box<dyn FnOnce() + Send>;
    static SPAWNER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let jobs = SPAWNER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("spawner".into())
            .spawn(move || {
                for job in rx {
                    job();
                }
            })
            .expect("process spawner thread");
        tx
    });
    let (tx, rx) = mpsc::sync_channel(1);
    jobs.send(Box::new(move || {
        let _ = tx.send(cmd.spawn());
    }))
    .map_err(|_| std::io::Error::other("the process spawner thread is not available"))?;
    rx.recv().unwrap_or_else(|_| Err(std::io::Error::other("the process spawner thread did not respond")))
}

/// Looks for an executable in `PATH` without invoking any shell.
pub fn which(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return is_executable(&p).then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| is_executable(p))
}

pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

fn kill_group(pid: u32, signal: i32) {
    // SAFETY: kill(2) with a negative pgid sends the signal to the group; the
    // pid belongs to a child we created with process_group(0).
    unsafe {
        libc::kill(-(pid as i32), signal);
    }
}

fn read_all<R: Read + Send + 'static>(reader: Option<R>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = reader {
            let _ = r.read_to_end(&mut buf);
        }
        buf
    })
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> (Option<ExitStatus>, bool) {
    let start = Instant::now();
    let mut delay = Duration::from_millis(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (Some(status), false),
            Ok(None) => {}
            Err(_) => return (None, false),
        }
        if start.elapsed() >= timeout {
            kill_group(child.id(), libc::SIGTERM);
            thread::sleep(Duration::from_millis(150));
            if !matches!(child.try_wait(), Ok(Some(_))) {
                kill_group(child.id(), libc::SIGKILL);
            }
            let status = child.wait().ok();
            return (status, true);
        }
        thread::sleep(delay);
        delay = (delay * 2).min(Duration::from_millis(25));
    }
}

/// Runs a command and waits for its full output.
pub fn run(program: &Path, args: &[String], timeout: Duration, input: Option<&[u8]>) -> Result<CommandResult> {
    run_raw(program, args, timeout, input).map(|(r, _)| r)
}

/// Same as [`run`], but also returns the raw stdout (PNG screenshots…).
pub fn run_raw(program: &Path, args: &[String], timeout: Duration, input: Option<&[u8]>) -> Result<(CommandResult, Vec<u8>)> {
    let mut cmd = base_command(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let start = Instant::now();
    let mut child = spawn(cmd).map_err(|e| Error::Spawn { program: program.display().to_string(), source: e })?;
    if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
        let data = data.to_vec();
        thread::spawn(move || {
            let _ = stdin.write_all(&data);
        });
    }
    let out = read_all(child.stdout.take());
    let err = read_all(child.stderr.take());
    let (status, timed_out) = wait_with_timeout(&mut child, timeout);
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    let mut argv = vec![program.display().to_string()];
    argv.extend(args.iter().cloned());
    let result = CommandResult {
        argv,
        code: status.and_then(|s| s.code()),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out,
        duration: start.elapsed(),
    };
    Ok((result, stdout))
}

/// Event of a streaming process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Stdout(String),
    Stderr(String),
    /// End of the process: exit code and whether we cancelled it.
    Exit {
        code: Option<i32>,
        cancelled: bool,
    },
}

/// Long-running process (logcat, telemetry, terminal, track-devices). Dropping
/// it cancels it.
pub struct StreamHandle {
    pid: u32,
    cancelled: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    stdin: Mutex<Option<std::process::ChildStdin>>,
}

impl StreamHandle {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn running(&self) -> bool {
        !self.finished.load(Ordering::SeqCst)
    }

    pub fn write_stdin(&self, data: &[u8]) -> std::io::Result<()> {
        let mut guard = self.stdin.lock().unwrap();
        match guard.as_mut() {
            Some(stdin) => {
                stdin.write_all(data)?;
                stdin.flush()
            }
            None => Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdin closed")),
        }
    }

    /// Cancels with SIGTERM and, if it does not end within 1 s, SIGKILL to the
    /// group.
    pub fn cancel(&self) {
        if self.finished.load(Ordering::SeqCst) || self.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        self.stdin.lock().unwrap().take();
        kill_group(self.pid, libc::SIGTERM);
        let pid = self.pid;
        let finished = self.finished.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                if finished.load(Ordering::SeqCst) {
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
            kill_group(pid, libc::SIGKILL);
        });
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    pub with_stdin: bool,
    /// Extra environment variables only for this process.
    pub env: Vec<(String, String)>,
    /// Keep the user's locale instead of `LC_ALL=C`: for programs with a window
    /// (scrcpy), whose title with accents or “·” gets garbled in C.
    pub user_locale: bool,
}

/// Starts a process and hands each complete line to `on_event` from reader
/// threads. EOF is not an empty line: `Exit` is emitted only once, after
/// draining stdout and stderr.
pub fn stream<F>(program: &Path, args: &[String], opts: StreamOptions, on_event: F) -> Result<StreamHandle>
where
    F: Fn(StreamEvent) + Send + Sync + 'static,
{
    let mut cmd = base_command(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if opts.with_stdin {
        cmd.stdin(Stdio::piped());
    }
    if opts.user_locale {
        cmd.env_remove("LC_ALL").env_remove("LANG");
        for key in ["LC_ALL", "LANG"] {
            if let Some(v) = std::env::var_os(key) {
                cmd.env(key, v);
            }
        }
    }
    for (k, v) in &opts.env {
        cmd.env(k, v);
    }
    let mut child = spawn(cmd).map_err(|e| Error::Spawn { program: program.display().to_string(), source: e })?;
    let pid = child.id();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let on_event = Arc::new(on_event);
    let cancelled = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));

    let spawn_reader = |reader: Option<Box<dyn Read + Send>>, is_err: bool| {
        let cb = on_event.clone();
        thread::spawn(move || {
            let Some(mut reader) = reader else { return };
            let emit = |bytes: &[u8]| {
                let line = String::from_utf8_lossy(bytes).into_owned();
                cb(if is_err { StreamEvent::Stderr(line) } else { StreamEvent::Stdout(line) });
            };
            // Splits on '\n' and also on '\r' (progress bars that rewrite the
            // same line). "\r\n" counts as a single break.
            let mut chunk = vec![0u8; 64 * 1024];
            let mut line: Vec<u8> = Vec::with_capacity(256);
            let mut last_cr = false;
            loop {
                let n = match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                for &b in &chunk[..n] {
                    match b {
                        b'\n' => {
                            if !(last_cr && line.is_empty()) {
                                emit(&line);
                            }
                            line.clear();
                            last_cr = false;
                        }
                        b'\r' => {
                            if !line.is_empty() {
                                emit(&line);
                                line.clear();
                            }
                            last_cr = true;
                        }
                        _ => {
                            line.push(b);
                            last_cr = false;
                        }
                    }
                }
            }
            if !line.is_empty() {
                emit(&line);
            }
        })
    };
    let out_t = spawn_reader(stdout.map(|s| Box::new(s) as Box<dyn Read + Send>), false);
    let err_t = spawn_reader(stderr.map(|s| Box::new(s) as Box<dyn Read + Send>), true);

    {
        let cb = on_event.clone();
        let cancelled = cancelled.clone();
        let finished = finished.clone();
        thread::spawn(move || {
            let status = child.wait().ok();
            let _ = out_t.join();
            let _ = err_t.join();
            finished.store(true, Ordering::SeqCst);
            cb(StreamEvent::Exit { code: status.and_then(|s| s.code()), cancelled: cancelled.load(Ordering::SeqCst) });
        });
    }

    Ok(StreamHandle { pid, cancelled, finished, stdin: Mutex::new(stdin) })
}

/// Equivalent to `shlex.quote`: quotes for Android's shell.
pub fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".into();
    }
    let safe = arg.bytes().all(|b| b.is_ascii_alphanumeric() || b"@%+=:,./-_".contains(&b));
    if safe {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\"'\"'"))
}

/// Joins already quoted arguments into a remote shell line.
pub fn shell_join<S: AsRef<str>>(args: &[S]) -> String {
    args.iter().map(|a| shell_quote(a.as_ref())).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("abc"), "abc");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\"'\"'s'");
        assert_eq!(shell_quote("/sdcard/Mis fotos/$x;rm"), "'/sdcard/Mis fotos/$x;rm'");
        assert_eq!(shell_join(&["ls", "-la", "/sdcard/A B"]), "ls -la '/sdcard/A B'");
    }

    #[test]
    fn run_and_timeout() {
        let sh = which("sh").unwrap();
        let r = run(&sh, &["-c".into(), "echo hola; echo err >&2; exit 3".into()], Duration::from_secs(5), None).unwrap();
        assert_eq!(r.stdout.trim(), "hola");
        assert_eq!(r.stderr.trim(), "err");
        assert_eq!(r.code, Some(3));
        assert!(!r.ok());
        let r = run(&sh, &["-c".into(), "sleep 5".into()], Duration::from_millis(200), None).unwrap();
        assert!(r.timed_out);
        assert!(r.duration < Duration::from_secs(2));
    }

    #[test]
    fn stdin_input() {
        let cat = which("cat").unwrap();
        let r = run(&cat, &[], Duration::from_secs(5), Some(b"entrada")).unwrap();
        assert_eq!(r.stdout, "entrada");
    }

    #[test]
    fn streaming_lines_and_exit_once() {
        let sh = which("sh").unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let _h = stream(&sh, &["-c".into(), "printf 'a\\nb\\r\\n10%%\\r20%%\\rsin-salto'; echo e >&2".into()], StreamOptions::default(), move |ev| {
            tx.lock().unwrap().send(ev).unwrap();
        })
        .unwrap();
        let mut out = Vec::new();
        let mut exits = 0;
        while let Ok(ev) = rx.recv_timeout(Duration::from_secs(5)) {
            match ev {
                StreamEvent::Stdout(l) => out.push(l),
                StreamEvent::Stderr(l) => assert_eq!(l, "e"),
                StreamEvent::Exit { code, cancelled } => {
                    exits += 1;
                    assert_eq!(code, Some(0));
                    assert!(!cancelled);
                    break;
                }
            }
        }
        assert_eq!(out, vec!["a", "b", "10%", "20%", "sin-salto"]);
        assert_eq!(exits, 1);
    }

    #[test]
    fn cancel_kills_group() {
        let sh = which("sh").unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let h = stream(&sh, &["-c".into(), "sleep 30 & sleep 30; wait".into()], StreamOptions::default(), move |ev| {
            let _ = tx.lock().unwrap().send(ev);
        })
        .unwrap();
        thread::sleep(Duration::from_millis(100));
        h.cancel();
        let ev = rx.recv_timeout(Duration::from_secs(3)).expect("must end");
        assert!(matches!(ev, StreamEvent::Exit { cancelled: true, .. }));
    }
    /// A process created from a worker thread does not die when that thread
    /// ends: only when ADB Scan ends (see [`spawn`]).
    #[test]
    fn child_outlives_worker_thread() {
        let sh = which("sh").unwrap();
        let mut child = thread::spawn(move || {
            let mut cmd = base_command(&sh);
            cmd.args(["-c", "sleep 0.5; exit 7"]);
            spawn(cmd).unwrap()
        })
        .join()
        .unwrap();
        // If Linux's signal followed the thread, it would exit through SIGTERM and
        // without a code.
        assert_eq!(child.wait().unwrap().code(), Some(7));
    }
}
