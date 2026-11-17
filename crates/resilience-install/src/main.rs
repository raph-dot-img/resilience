//! `resilience-install` — installs Resilience on a whole disk (ADR 0022).
//!
//! Runs as root, on the live system only, through pkexec
//! (`org.resilience.install.run`, allowed to the active local session).
//! Every argument and the disk rules are checked here again
//! (`resilience_core::install`); tools come from the root-owned
//! `/etc/resilience/install.toml`, never from the environment. Secrets (the
//! encryption passphrase, account passwords) are typed into cryptsetup and
//! passwd themselves, on the console: this program never sees them.
//!
//! ```text
//! resilience-install run --disk DEV --size BYTES --model M --firmware uefi|bios
//!     --encrypt yes|no --keymap ID --account NAME:MODE:FULL NAME ...
//! ```

use resilience_core::admin::{groups_for, now_rfc3339};
use resilience_core::install::{
    self, BOOT_LABEL, Firmware, INSTALL_CONFIG_PATH, InstallConfig, LSBLK_COLUMNS, ROOT_LABEL,
    Request, check, check_accounts, names_in, partition_path, sfdisk_script,
};
use resilience_core::keyboard::state_text;
use resilience_core::media::parse_pairs;
use resilience_core::proc::{run_full, run_visible, run_with_input};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const EXIT_ERROR: u8 = 1;
const EXIT_REFUSED: u8 = 2;
const EXIT_USAGE: u8 = 64;
const EXIT_PERMISSION: u8 = 77;

/// Where the new system is mounted while installing.
const TARGET: &str = "/run/resilience-install/target";
/// Device-mapper name of the opened encrypted root while installing.
const MAPPER: &str = "resilience-install-root";
/// Login shell of every account (stable across generations).
const SESSION_SHELL: &str = "/run/current-system/sw/bin/resilience-session";
/// Tries for a passphrase or password typed twice differently.
const TRIES: usize = 3;

const MINUTE: Duration = Duration::from_secs(60);

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

enum Failure {
    Usage,
    Permission,
    Refused(String),
    Error(String),
}

type Result<T> = std::result::Result<T, Failure>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("--version") => {
            println!("resilience-install {}", resilience_core::VERSION);
            Ok(())
        }
        _ => Err(Failure::Usage),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage) => {
            eprintln!(
                "Usage: resilience-install run --disk DEV --size BYTES --model MODEL \
                 --firmware uefi|bios --encrypt yes|no --keymap ID --account NAME:MODE:FULL NAME..."
            );
            ExitCode::from(EXIT_USAGE)
        }
        Err(Failure::Permission) => {
            eprintln!("resilience-install: must run as root (through the installer screen)");
            ExitCode::from(EXIT_PERMISSION)
        }
        Err(Failure::Refused(msg)) => {
            eprintln!("\nNothing was changed: {msg}");
            ExitCode::from(EXIT_REFUSED)
        }
        Err(Failure::Error(msg)) => {
            eprintln!("\nThe installation FAILED: {msg}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

fn arg(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// A program run in the foreground with the console as its terminal (for
/// cryptsetup's and passwd's own prompts), a minimal environment, no
/// timeout: it waits for a person. Returns whether it succeeded.
fn interactive(argv: &[String], path: &str) -> bool {
    let Some((program, args)) = argv.split_first() else {
        return false;
    };
    let term = std::env::var("TERM").unwrap_or_else(|_| "linux".into());
    Command::new(program)
        .args(args)
        .env_clear()
        .env("PATH", path)
        .env("TERM", term)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .is_ok_and(|s| s.success())
}

/// A program whose output is shown (progress), stdin closed, with a timeout.
fn visible(argv: &[String], path: &str, timeout: Duration) -> bool {
    let v: Vec<&str> = argv.iter().map(String::as_str).collect();
    run_visible(&v, &[("PATH", path), ("LC_ALL", "C.UTF-8")], timeout) == Some(0)
}

/// A short program whose output is only shown when it fails.
fn quiet(argv: &[String], timeout: Duration) -> std::result::Result<(), String> {
    let v: Vec<&str> = argv.iter().map(String::as_str).collect();
    match run_full(&v, timeout) {
        Some(o) if o.success() => Ok(()),
        Some(o) => Err(format!("{}: {}", v[0], o.stderr.trim())),
        None => Err(format!("{} did not finish", v[0])),
    }
}

/// [`interactive`] up to [`TRIES`] times (a person may mistype), saying
/// `again` between tries; never again after Ctrl-C.
fn retry(argv: &[String], path: &str, again: &str) -> bool {
    for i in 0..TRIES {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return false;
        }
        if i > 0 {
            println!("\n{again}");
        }
        if interactive(argv, path) {
            return true;
        }
    }
    false
}

fn s(x: &str) -> String {
    x.to_string()
}

struct Installer {
    cfg: InstallConfig,
    req: Request,
    step: usize,
    steps: usize,
    mounted: bool,
    opened: bool,
}

fn run(args: &[String]) -> Result<()> {
    if !is_root() {
        return Err(Failure::Permission);
    }
    let req = install::parse_args(args).map_err(|e| match e {
        install::Refusal::BadArguments(_) => Failure::Usage,
        other => Failure::Refused(other.to_string()),
    })?;
    let text = fs::read_to_string(INSTALL_CONFIG_PATH).map_err(|e| {
        Failure::Refused(format!("{INSTALL_CONFIG_PATH}: {e} (not a live system?)"))
    })?;
    let cfg = InstallConfig::parse(&text)
        .map_err(|e| Failure::Refused(format!("{INSTALL_CONFIG_PATH}: {e}")))?;

    let detected = if Path::new("/sys/firmware/efi").exists() {
        Firmware::Uefi
    } else {
        Firmware::Bios
    };
    if detected != req.firmware {
        return Err(Failure::Refused(format!(
            "this computer started in {} mode, not {}",
            detected.label(),
            req.firmware.label()
        )));
    }
    // The disk as it is now, not as the interface saw it.
    let out = run_full(
        &[&arg(&cfg.lsblk), "-P", "-b", "-o", LSBLK_COLUMNS],
        Duration::from_secs(10),
    )
    .filter(|o| o.success())
    .ok_or_else(|| Failure::Refused("lsblk failed".into()))?;
    let disks = install::disks(&parse_pairs(&out.stdout));
    check(&req, &disks, &cfg.keymaps).map_err(|e| Failure::Refused(e.to_string()))?;
    let disk = disks
        .iter()
        .find(|d| d.path == req.disk)
        .ok_or_else(|| Failure::Refused("the disk disappeared".into()))?
        .clone();
    if !cfg.system(req.firmware, req.encrypt).exists() {
        return Err(Failure::Refused(
            "the system to install is missing from this live system".into(),
        ));
    }

    // Ctrl-C ends the program running now (it gets the signal); the
    // installer then cleans up and stops instead of dying half-way.
    // SAFETY: the handler only stores to an atomic; signal() replaces the
    // disposition of SIGINT/SIGQUIT for this process, reset in children by exec.
    unsafe {
        libc::signal(
            libc::SIGINT,
            on_interrupt as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGQUIT,
            on_interrupt as *const () as libc::sighandler_t,
        );
    }

    println!(
        "Installing Resilience ({}) on {} {}.",
        cfg.edition,
        disk.path,
        disk.describe()
    );
    let mut inst = Installer {
        steps: 10 + usize::from(req.encrypt) + usize::from(req.firmware == Firmware::Bios),
        cfg,
        req,
        step: 0,
        mounted: false,
        opened: false,
    };
    let res = inst.install(&disk);
    inst.cleanup();
    match res {
        Ok(()) => {
            println!(
                "\nResilience is installed on {}.\n\
                 Shut down, remove the live USB stick, and start the computer again.",
                disk.path
            );
            Ok(())
        }
        Err(Failure::Error(msg)) => Err(Failure::Error(format!(
            "{msg}\nThe disk {} is not usable as it is: start the installation again.",
            disk.path
        ))),
        Err(e) => Err(e),
    }
}

impl Installer {
    fn begin(&mut self, what: &str) -> Result<()> {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return Err(Failure::Error("interrupted (Ctrl-C)".into()));
        }
        self.step += 1;
        println!("\n[{}/{}] {what}", self.step, self.steps);
        Ok(())
    }

    fn path(&self) -> &str {
        &self.cfg.path
    }

    fn install(&mut self, disk: &install::Disk) -> Result<()> {
        let fail = |m: String| Failure::Error(m);
        let d = self.req.disk.clone();
        let boot = partition_path(&d, 2);
        let root_part = partition_path(&d, 3);

        self.begin(&format!("Keyboard layout: {}", self.req.keymap))?;
        quiet(
            &[
                arg(&self.cfg.loadkeys),
                s("-q"),
                s("--"),
                self.req.keymap.clone(),
            ],
            Duration::from_secs(10),
        )
        .map_err(fail)?;

        self.begin(&format!("Erasing {d}"))?;
        for p in disk.parts.iter().filter(|p| p.path.starts_with(&d)) {
            quiet(
                &[arg(&self.cfg.wipefs), s("-a"), s("-q"), p.path.clone()],
                MINUTE,
            )
            .map_err(fail)?;
        }
        quiet(
            &[arg(&self.cfg.wipefs), s("-a"), s("-q"), d.clone()],
            MINUTE,
        )
        .map_err(fail)?;

        self.begin("Creating the partitions")?;
        let sfdisk = [
            arg(&self.cfg.sfdisk),
            s("--quiet"),
            s("--wipe"),
            s("always"),
            s("--wipe-partitions"),
            s("always"),
            d.clone(),
        ];
        let v: Vec<&str> = sfdisk.iter().map(String::as_str).collect();
        match run_with_input(&v, sfdisk_script().as_bytes(), MINUTE) {
            Some(o) if o.success() => {}
            Some(o) => return Err(fail(format!("sfdisk: {}", o.stderr.trim()))),
            None => return Err(fail("sfdisk did not finish".into())),
        }
        quiet(
            &[arg(&self.cfg.udevadm), s("settle"), s("--timeout=30")],
            MINUTE,
        )
        .map_err(fail)?;
        for p in [&boot, &root_part] {
            if !Path::new(p).exists() {
                return Err(fail(format!("{p} did not appear after partitioning")));
            }
        }

        self.begin("Formatting the boot partition")?;
        quiet(
            &[
                arg(&self.cfg.mkfs_vfat),
                s("-F"),
                s("32"),
                s("-n"),
                s(BOOT_LABEL),
                boot.clone(),
            ],
            MINUTE,
        )
        .map_err(fail)?;

        let root_dev = if self.req.encrypt {
            self.begin("Encrypting the disk")?;
            println!(
                "Choose the passphrase that will be asked at every start. Type it, then \
                 type it again.\nThe keyboard layout is {} here and when the computer starts.\n\
                 If it is lost, nothing on this disk can be recovered.",
                self.req.keymap
            );
            let format = [
                arg(&self.cfg.cryptsetup),
                s("--batch-mode"),
                s("--verify-passphrase"),
                s("--type"),
                s("luks2"),
                s("luksFormat"),
                root_part.clone(),
            ];
            if !retry(
                &format,
                self.path(),
                "The two passphrases were different or empty. Again:",
            ) {
                return Err(fail("the disk could not be encrypted".into()));
            }
            println!("\nNow type the same passphrase once more, to open the disk:");
            let open = [
                arg(&self.cfg.cryptsetup),
                s("open"),
                root_part.clone(),
                s(MAPPER),
            ];
            if !retry(
                &open,
                self.path(),
                "That passphrase does not open the disk. Again:",
            ) {
                return Err(fail("the encrypted disk could not be opened".into()));
            }
            self.opened = true;
            format!("/dev/mapper/{MAPPER}")
        } else {
            root_part.clone()
        };

        self.begin("Formatting the system partition")?;
        quiet(
            &[
                arg(&self.cfg.mkfs_ext4),
                s("-q"),
                s("-F"),
                s("-L"),
                s(ROOT_LABEL),
                root_dev.clone(),
            ],
            Duration::from_secs(600),
        )
        .map_err(fail)?;

        self.begin("Mounting the new system")?;
        fs::create_dir_all(TARGET).map_err(|e| fail(format!("{TARGET}: {e}")))?;
        quiet(&[arg(&self.cfg.mount), root_dev, s(TARGET)], MINUTE).map_err(fail)?;
        self.mounted = true;
        let target = PathBuf::from(TARGET);
        fs::create_dir_all(target.join("boot")).map_err(|e| fail(format!("boot: {e}")))?;
        quiet(
            &[
                arg(&self.cfg.mount),
                s("-o"),
                s("fmask=0077,dmask=0077"),
                boot,
                arg(&target.join("boot")),
            ],
            MINUTE,
        )
        .map_err(fail)?;

        self.begin("Writing this computer's settings")?;
        let (local, modes) = install::account_state(&self.req.accounts, &now_rfc3339());
        write_state(
            &target,
            &[
                ("keyboard", state_text(&self.req.keymap)),
                ("accounts.toml", local.to_toml()),
            ],
        )
        .map_err(fail)?;

        self.begin("Copying the system (this takes a while)")?;
        let system = self.cfg.system(self.req.firmware, self.req.encrypt).clone();
        if !visible(
            &[
                arg(&self.cfg.nixos_install),
                s("--system"),
                arg(&system),
                s("--root"),
                s(TARGET),
                s("--no-root-passwd"),
                s("--no-channel-copy"),
            ],
            self.path(),
            Duration::from_secs(4 * 3600),
        ) {
            return Err(fail("copying the system failed (see above)".into()));
        }

        self.begin("Creating the accounts")?;
        self.create_accounts(&target)?;
        // Only now: the activation run by nixos-install applies modes.toml
        // and would complain about accounts that do not exist yet. useradd
        // already gave each account its groups; the first start applies
        // the file again.
        write_state(&target, &[("modes.toml", modes.to_toml())]).map_err(fail)?;

        if self.req.firmware == Firmware::Bios {
            self.begin("Installing the BIOS boot loader")?;
            if !visible(
                &[
                    arg(&self.cfg.grub_install),
                    s("--target=i386-pc"),
                    format!("--boot-directory={}", arg(&target.join("boot"))),
                    d.clone(),
                ],
                self.path(),
                Duration::from_secs(300),
            ) {
                return Err(fail("grub-install failed (see above)".into()));
            }
        }

        self.begin("Finishing")?;
        debug_assert_eq!(self.step, self.steps);
        quiet(&[arg(&self.cfg.sync)], Duration::from_secs(600)).map_err(fail)?;
        Ok(())
    }

    fn enter(&self, cmd: &[String]) -> Vec<String> {
        let mut v = vec![
            arg(&self.cfg.nixos_enter),
            s("--root"),
            s(TARGET),
            s("--silent"),
            s("--"),
        ];
        v.extend_from_slice(cmd);
        v
    }

    fn create_accounts(&self, target: &Path) -> Result<()> {
        let fail = |m: String| Failure::Error(m);
        let mut taken =
            names_in(&fs::read_to_string(target.join("etc/passwd")).unwrap_or_default());
        taken.extend(names_in(
            &fs::read_to_string(target.join("etc/group")).unwrap_or_default(),
        ));
        check_accounts(&self.req.accounts, &taken).map_err(|e| fail(e.to_string()))?;
        for a in &self.req.accounts {
            let mut useradd = vec![
                s("/run/current-system/sw/bin/useradd"),
                s("--create-home"),
                s("--gid"),
                s("users"),
                s("--shell"),
                s(SESSION_SHELL),
                s("--comment"),
                a.description.clone(),
            ];
            let groups = groups_for(a.mode);
            if !groups.is_empty() {
                useradd.push(s("--groups"));
                useradd.push(groups.join(","));
            }
            useradd.push(a.name.clone());
            if !visible(&self.enter(&useradd), self.path(), MINUTE) {
                return Err(fail(format!(
                    "the account '{}' could not be created",
                    a.name
                )));
            }
            println!(
                "\nChoose the password of '{}' ({}). Type it, then type it again.",
                a.name,
                resilience_core::admin::mode_id(a.mode)
            );
            let passwd = self.enter(&[s("/run/current-system/sw/bin/passwd"), a.name.clone()]);
            if !retry(&passwd, self.path(), "The password was not set. Again:") {
                return Err(fail(format!("no password was set for '{}'", a.name)));
            }
        }
        Ok(())
    }

    /// Unmount and close whatever is still mounted or open.
    fn cleanup(&mut self) {
        if self.mounted {
            let _ = quiet(&[arg(&self.cfg.umount), s("-R"), s(TARGET)], MINUTE);
            self.mounted = false;
        }
        if self.opened {
            let _ = quiet(&[arg(&self.cfg.cryptsetup), s("close"), s(MAPPER)], MINUTE);
            self.opened = false;
        }
    }
}

/// State files of ADR 0022 §2 in the new system's /var/lib/resilience.
/// The keyboard layout is written before the system is copied: the boot
/// loader puts it into the initrd.
fn write_state(target: &Path, files: &[(&str, String)]) -> std::result::Result<(), String> {
    let dir = target.join("var/lib/resilience");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    for (name, text) in files {
        let p = dir.join(name);
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(&p)
            .map_err(|e| format!("{}: {e}", p.display()))?;
        f.write_all(text.as_bytes())
            .and_then(|()| f.sync_all())
            .map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(())
}
