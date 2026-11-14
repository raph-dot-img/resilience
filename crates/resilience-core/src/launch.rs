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
/// as separate arguments (for example a file name chosen in the TUI); `env`
/// overrides individual environment variables of the child.
pub fn run_foreground(
    argv: &[String],
    extra_args: &[&std::ffi::OsStr],
    env: &[(String, String)],
) -> io::Result<ExitStatus> {
    run_foreground_env(argv, extra_args, env, false)
}

/// [`run_foreground`]; with `clean`, the child gets only `TERM`, `LANG`,
/// `HOME`, `USER`, a fixed `PATH` and `env`: nothing else of this process's
/// environment reaches it.
pub fn run_foreground_env(
    argv: &[String],
    extra_args: &[&std::ffi::OsStr],
    env: &[(String, String)],
    clean: bool,
) -> io::Result<ExitStatus> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty command"))?;
    let mut cmd = Command::new(prog);
    cmd.args(args).args(extra_args);
    if clean {
        let keep: Vec<(String, String)> = ["TERM", "LANG", "HOME", "USER"]
            .iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
            .collect();
        cmd.env_clear()
            .envs(keep)
            .env("PATH", "/run/current-system/sw/bin");
    }
    cmd.envs(env.iter().map(|(k, v)| (k, v)));
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

/// polkit's text agent `pkttyagent`, registered for this process while the
/// value lives, so that an administrator's password is asked on the
/// terminal for `pkexec` commands this process starts.
///
/// pkexec's built-in agent cannot complete a successful authentication
/// when polkit uses its socket-activated helper (the helper reports pkexec's
/// effective uid 0, polkitd expects the calling user; upstream polkit issue
/// 17). pkttyagent runs as the user, so the uids match. pkexec checks the
/// authorisation of its parent, which must be this process.
pub struct PolkitAgent {
    child: std::process::Child,
}

impl PolkitAgent {
    pub fn start(agent: &str) -> io::Result<PolkitAgent> {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: pipe2 writes two descriptors into the provided array.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let (read_fd, write_fd) = (fds[0], fds[1]);
        // SAFETY: getpid cannot fail.
        let pid = unsafe { libc::getpid() };
        let mut cmd = Command::new(agent);
        cmd.args([
            "--process".to_string(),
            pid.to_string(),
            "--notify-fd".to_string(),
            write_fd.to_string(),
            "--fallback".to_string(),
        ]);
        // SAFETY: between fork and exec, only fcntl (async-signal-safe) is
        // called to let the child inherit the write end of the pipe.
        unsafe {
            cmd.pre_exec(move || {
                if libc::fcntl(write_fd, libc::F_SETFD, 0) != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let spawned = cmd.spawn();
        // SAFETY: the parent no longer needs the write end.
        unsafe { libc::close(write_fd) };
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                // SAFETY: closing a descriptor we own.
                unsafe { libc::close(read_fd) };
                return Err(e);
            }
        };
        // pkttyagent closes the notify fd once it is registered: wait for EOF.
        let mut pfd = libc::pollfd {
            fd: read_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll on one valid descriptor with a 5 s timeout.
        let ready = unsafe { libc::poll(&mut pfd, 1, 5000) };
        // SAFETY: closing a descriptor we own.
        unsafe { libc::close(read_fd) };
        if ready == 1 {
            Ok(PolkitAgent { child })
        } else {
            let _ = child.kill();
            let _ = child.wait();
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the polkit authentication agent did not start",
            ))
        }
    }
}

impl Drop for PolkitAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run `argv` (a `pkexec` command) in the foreground with [`PolkitAgent`]
/// registered for this process.
pub fn run_with_polkit_agent(
    agent: &str,
    argv: &[String],
    env: &[(String, String)],
) -> io::Result<ExitStatus> {
    let _agent = PolkitAgent::start(agent)?;
    run_foreground(argv, &[], env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_fixed_argv_without_shell() {
        // If a shell interpreted the argument, `;` would run `false`.
        let argv = vec!["echo".to_string(), "a;false".to_string()];
        let st = run_foreground(&argv, &[], &[]).expect("run");
        assert!(st.success());
    }

    #[test]
    fn reports_errors() {
        assert!(run_foreground(&[], &[], &[]).is_err());
        assert!(run_foreground(&["/nonexistent/x".into()], &[], &[]).is_err());
        let st =
            run_foreground(&["sh".into(), "-c".into(), "exit 4".into()], &[], &[]).expect("run");
        assert_eq!(st.code(), Some(4));
    }

    #[test]
    fn applies_environment_overrides() {
        let argv: Vec<String> = ["sh", "-c", "test \"$RESILIENCE_X\" = 'a b'"]
            .map(String::from)
            .into();
        let env = [("RESILIENCE_X".to_string(), "a b".to_string())];
        assert!(run_foreground(&argv, &[], &env).expect("run").success());
        assert!(!run_foreground(&argv, &[], &[]).expect("run").success());
    }

    #[test]
    fn polkit_agent_is_registered_first_and_stopped_after() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("resilience-agent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("mkdir");
        // Fake agent: records its arguments, closes the notify fd, waits.
        let agent = d.join("agent");
        std::fs::write(
            &agent,
            format!(
                "#!/bin/sh\necho \"$@\" > {0}/args\neval \"exec $4>&-\"\nexec sleep 30\n",
                d.display()
            ),
        )
        .expect("write");
        std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let argv: Vec<String> = ["sh", "-c", "test -s \"$1\"", "sh"]
            .map(String::from)
            .into_iter()
            .chain([d.join("args").display().to_string()])
            .collect();
        let st = run_with_polkit_agent(&agent.to_string_lossy(), &argv, &[]).expect("run");
        assert!(st.success(), "agent ran before the command");
        let args = std::fs::read_to_string(d.join("args")).expect("args");
        // SAFETY: getpid cannot fail.
        let pid = unsafe { libc::getpid() };
        assert!(
            args.starts_with(&format!("--process {pid} --notify-fd ")),
            "{args}"
        );
        assert!(args.trim_end().ends_with("--fallback"), "{args}");
        assert!(run_with_polkit_agent("/nonexistent/agent", &argv, &[]).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
