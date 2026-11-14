//! Running interactive programs in the foreground.
//!
//! Programs are started from a fixed argv with `execve` semantics — never via
//! `sh -c` — so no user input is ever interpreted by a shell.
//!
//! While a child runs, the TUI ignores SIGINT/SIGQUIT/SIGTSTP so that Ctrl-C
//! inside the child does not kill the interface. The child gets the default
//! SIGINT/SIGQUIT handlers back; SIGTSTP stays ignored because the session has
//! no job control to resume a stopped program.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus};

struct IgnoreSignals {
    saved: [(libc::c_int, libc::sighandler_t); 3],
}

impl IgnoreSignals {
    fn new() -> Self {
        let sigs = [libc::SIGINT, libc::SIGQUIT, libc::SIGTSTP];
        // SAFETY: installing SIG_IGN is always valid; we keep the previous
        // dispositions to restore them in Drop.
        let saved = sigs.map(|s| (s, unsafe { libc::signal(s, libc::SIG_IGN) }));
        IgnoreSignals { saved }
    }
}

impl Drop for IgnoreSignals {
    fn drop(&mut self) {
        for (sig, old) in self.saved {
            // SAFETY: restoring a disposition previously returned by signal().
            unsafe {
                libc::signal(sig, old);
            }
        }
    }
}

/// Run `argv` in the foreground and wait for it. `extra_args` are appended
/// as separate arguments (for example a file name chosen in the TUI).
pub fn run_foreground(argv: &[String], extra_args: &[&std::ffi::OsStr]) -> io::Result<ExitStatus> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty command"))?;
    let mut cmd = Command::new(prog);
    cmd.args(args).args(extra_args);
    // SAFETY: the closure runs between fork and exec and only calls signal(),
    // which is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            Ok(())
        });
    }
    let _guard = IgnoreSignals::new();
    tracing::info!(program = %prog, "launching program");
    let status = cmd.status();
    tracing::info!(program = %prog, ?status, "program finished");
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_fixed_argv_without_shell() {
        // If a shell interpreted the argument, `;` would run `false`.
        let argv = vec!["echo".to_string(), "a;false".to_string()];
        let st = run_foreground(&argv, &[]).expect("run");
        assert!(st.success());
    }

    #[test]
    fn reports_errors() {
        assert!(run_foreground(&[], &[]).is_err());
        assert!(run_foreground(&["/nonexistent/x".into()], &[]).is_err());
        let st = run_foreground(&["sh".into(), "-c".into(), "exit 4".into()], &[]).expect("run");
        assert_eq!(st.code(), Some(4));
    }
}
