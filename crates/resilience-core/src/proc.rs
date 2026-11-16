//! Running short helper programs with a hard timeout.
//!
//! Used for read-only queries such as `systemctl is-system-running`, and for
//! viewers (pdftotext, bsdtar) whose output can be large: stdout and stderr
//! are read while the child runs, up to [`MAX_OUTPUT`] bytes each (the rest is
//! discarded, see [`Output::truncated`]), and decoded lossily.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Run `argv` with stdin closed, returning stdout if the program finished
/// within `timeout`. The exit status is ignored on purpose: tools such as
/// `systemctl is-system-running` report useful text with a non-zero status.
/// A program that exceeds the timeout is killed and `None` is returned.
pub fn run_capture(argv: &[&str], timeout: Duration) -> Option<String> {
    run_full(argv, timeout).map(|o| o.stdout)
}

/// Bytes kept of each of stdout and stderr.
pub const MAX_OUTPUT: usize = 8 * 1024 * 1024;

/// Captured result of [`run_full`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Exit code; `None` if killed by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// stdout was longer than [`MAX_OUTPUT`] and was cut.
    pub truncated: bool,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Like [`run_capture`] but also returns the exit code and stderr.
/// Returns `None` if the program could not be started or timed out.
pub fn run_full(argv: &[&str], timeout: Duration) -> Option<Output> {
    run_inner(argv, None, &[], timeout)
}

/// Like [`run_full`], with `env` added to (not replacing) the environment.
pub fn run_full_env(argv: &[&str], env: &[(&str, &str)], timeout: Duration) -> Option<Output> {
    run_inner(argv, None, env, timeout)
}

/// Like [`run_full`], with `input` written to the child's stdin (then
/// closed). Used to hand a secret to a program without putting it in argv
/// or the environment; the caller should wipe its copy afterwards.
pub fn run_with_input(argv: &[&str], input: &[u8], timeout: Duration) -> Option<Output> {
    run_inner(argv, Some(input), &[], timeout)
}

/// Run `argv` with a cleared environment (only `env` is set), stdin closed
/// and stdout/stderr inherited, so a long operation shows its progress on
/// the caller's terminal. Returns the exit code; `None` if the program could
/// not start, was killed by a signal, or exceeded `timeout`. The program
/// runs in its own process group (so keyboard signals do not reach it), and
/// on timeout the whole group is killed, including programs it started.
/// Used by the privileged helper for tools whose output can be long.
pub fn run_visible(argv: &[&str], env: &[(&str, &str)], timeout: Duration) -> Option<i32> {
    use std::os::unix::process::CommandExt;
    let (program, args) = argv.split_first()?;
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()?;
    let pgid = child.id() as libc::pid_t;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return st.code(),
            Ok(None) if Instant::now() >= deadline => {
                // SAFETY: kill(2) on our child's own process group (its
                // pgid is its pid, set at spawn); no memory is involved.
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
                let _ = child.wait();
                tracing::warn!(program, "helper command timed out");
                return None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
}

fn run_inner(
    argv: &[&str],
    input: Option<&[u8]>,
    env: &[(&str, &str)],
    timeout: Duration,
) -> Option<Output> {
    let (program, args) = argv.split_first()?;
    let mut child = Command::new(program)
        .args(args)
        .envs(env.iter().copied())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
        // Small inputs fit in the pipe buffer; a child that never reads
        // gets EPIPE, which is not an error here. Dropping closes it.
        let _ = stdin.write_all(data);
    }
    // Read both pipes in threads while the child runs: a child writing more
    // than the pipe buffer would otherwise block until the timeout.
    fn drain(mut r: impl Read + Send + 'static) -> thread::JoinHandle<(Vec<u8>, bool)> {
        thread::spawn(move || {
            let mut kept = Vec::new();
            let mut buf = [0u8; 65536];
            let mut cut = false;
            loop {
                match r.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let room = MAX_OUTPUT.saturating_sub(kept.len());
                        kept.extend_from_slice(&buf[..n.min(room)]);
                        cut |= n > room;
                    }
                }
            }
            (kept, cut)
        })
    }
    let out_t = drain(child.stdout.take()?);
    let err_t = drain(child.stderr.take()?);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(program, "helper command timed out");
                return None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    };
    // A grandchild may still hold the pipes open: the threads end with it.
    let (out, truncated) = out_t.join().ok()?;
    let (err, _) = err_t.join().ok()?;
    Some(Output {
        code: status.code(),
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_and_non_utf8_output_is_captured() {
        // More than the pipe buffer: this used to block until the timeout.
        let out = run_full(
            &[
                "sh",
                "-c",
                "head -c 300000 /dev/zero | tr '\\0' a; printf '\\377'",
            ],
            Duration::from_secs(10),
        )
        .expect("finished");
        assert!(out.success());
        assert_eq!(out.stdout.len(), 300_000 + '\u{fffd}'.len_utf8());
        assert!(!out.truncated);
    }

    #[test]
    fn captures_output() {
        let out = run_capture(&["sh", "-c", "echo hello; exit 3"], Duration::from_secs(5));
        assert_eq!(out.as_deref(), Some("hello\n"));
    }

    #[test]
    fn captures_status_and_stderr() {
        let o = run_full(
            &["sh", "-c", "echo oops >&2; exit 2"],
            Duration::from_secs(5),
        )
        .expect("ran");
        assert_eq!(o.code, Some(2));
        assert!(!o.success());
        assert_eq!(o.stderr, "oops\n");
    }

    #[test]
    fn visible_runs_with_a_clean_environment() {
        // HOME is set for cargo test (also in the Nix build sandbox).
        assert!(std::env::var_os("HOME").is_some());
        let code = run_visible(
            &["/bin/sh", "-c", "test -z \"$HOME\" && test \"$KEEP\" = yes"],
            &[("KEEP", "yes")],
            Duration::from_secs(5),
        );
        assert_eq!(code, Some(0));
        assert_eq!(
            run_visible(&["/bin/sh", "-c", "exit 4"], &[], Duration::from_secs(5)),
            Some(4)
        );
        let start = Instant::now();
        assert_eq!(
            run_visible(
                &["/bin/sh", "-c", "while :; do :; done"],
                &[],
                Duration::from_millis(200)
            ),
            None
        );
        assert!(start.elapsed() < Duration::from_secs(5));
        // A grandchild is killed with it (whole process group).
        let dir = std::env::temp_dir().join(format!("resilience-pg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let marker = dir.join("late");
        let script = format!("(sleep 1 && : > {}) & wait", marker.display());
        let path = std::env::var("PATH").unwrap_or_default();
        assert_eq!(
            run_visible(
                &["/bin/sh", "-c", &script],
                &[("PATH", path.as_str())],
                Duration::from_millis(200)
            ),
            None
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            assert!(!marker.exists(), "grandchild survived the timeout");
            thread::sleep(Duration::from_millis(50));
        }
        // Positive control: without a timeout the same script does write it.
        assert_eq!(
            run_visible(
                &["/bin/sh", "-c", &script],
                &[("PATH", path.as_str())],
                Duration::from_secs(10)
            ),
            Some(0)
        );
        assert!(marker.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enforces_timeout() {
        let start = Instant::now();
        let out = run_capture(&["sleep", "10"], Duration::from_millis(200));
        assert!(out.is_none());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn missing_program_is_none() {
        assert!(run_capture(&["/nonexistent/program"], Duration::from_secs(1)).is_none());
        assert!(run_capture(&[], Duration::from_secs(1)).is_none());
    }
}
