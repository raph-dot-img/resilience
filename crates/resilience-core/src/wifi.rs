//! Wi-Fi through NetworkManager's `nmcli`, as the user (ADR 0010).
//!
//! Nothing here is privileged. What an account may do is decided by polkit:
//! accounts outside the `networkmanager` group may only scan, connect and
//! disconnect, switch the Wi-Fi radio, and manage profiles that belong to
//! them alone (`connection.permissions user:NAME`). See nix/modules/networking.nix.
//!
//! Passwords never appear in argv, the environment, logs or files we write:
//! profiles are created without a secret, and the password typed in the
//! interface reaches nmcli through a pipe (`passwd-file /dev/stdin`).
//! NetworkManager stores it in its root-only profile file.
//!
//! (`nmcli --ask` was tried first: after a wrong password it asks again
//! with the old one prefilled, and Ctrl-C does not leave the prompt, which
//! traps a user who does not know the password. Found in vm-wifi.)

use std::path::Path;
use std::time::Duration;

use crate::admin::valid_user_name;
use crate::proc::{run_full, run_with_input};

const TIMEOUT: Duration = Duration::from_secs(15);
/// How long `nmcli connection up` may take (association + DHCP). With a
/// wrong password NetworkManager keeps retrying (nmcli's password file
/// answers every new request with the same password) until this expires.
pub const CONNECT_WAIT_SECS: u32 = 45;

/// Split one line of `nmcli -t` output. Fields are separated by `:`;
/// a literal `:` is written `\:` and a backslash `\\`.
pub fn split_terse(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.last_mut().expect("never empty").push(n);
                }
            }
            ':' => out.push(String::new()),
            c => out.last_mut().expect("never empty").push(c),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Security {
    /// No encryption at all.
    Open,
    /// WPA/WPA2 personal (also WPA2/WPA3 transition networks).
    Psk,
    /// WPA3 personal only.
    Sae,
    /// Enterprise (802.1X), WEP or anything else we do not set up.
    Unsupported(String),
}

impl Security {
    /// From nmcli's SECURITY column, e.g. `WPA2`, `WPA1 WPA2`, `WPA3`,
    /// `WPA2 WPA3`, `WPA2 802.1X`, `WEP`, `` or `--`.
    pub fn from_nmcli(s: &str) -> Security {
        let words: Vec<&str> = s.split_whitespace().collect();
        let has = |w: &str| words.contains(&w);
        if words.is_empty() || words == ["--"] {
            Security::Open
        } else if has("802.1X") || has("WEP") {
            Security::Unsupported(s.trim().to_string())
        } else if has("WPA1") || has("WPA2") {
            Security::Psk
        } else if has("WPA3") {
            Security::Sae
        } else {
            Security::Unsupported(s.trim().to_string())
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Security::Open => "open",
            Security::Psk => "WPA2",
            Security::Sae => "WPA3",
            Security::Unsupported(s) if s.contains("802.1X") => "802.1X",
            Security::Unsupported(s) if s.contains("WEP") => "WEP",
            Security::Unsupported(_) => "other",
        }
    }
}

/// One visible network (strongest access point per name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    pub security: Security,
    /// 0–100.
    pub signal: u8,
    pub in_use: bool,
    /// A saved profile with this name, if the account can see one.
    pub saved: Option<Profile>,
}

impl Network {
    /// Why this network cannot be joined from the interface, if so.
    pub fn problem(&self) -> Option<&'static str> {
        match &self.security {
            Security::Unsupported(s) if s.contains("802.1X") => {
                Some("enterprise (802.1X) networks need an administrator to set up")
            }
            Security::Unsupported(s) if s.contains("WEP") => {
                Some("WEP is broken encryption and is not offered")
            }
            Security::Unsupported(_) => Some("this kind of security is not supported"),
            _ if self.saved.is_none() && !valid_ssid(&self.ssid) => {
                Some("this network name cannot be used")
            }
            _ => None,
        }
    }
}

/// A saved Wi-Fi profile visible to the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub uuid: String,
    pub active: bool,
}

/// Network names we create profiles for: 1–32 bytes, no control
/// characters (they would be unreadable on screen and in logs).
pub fn valid_ssid(s: &str) -> bool {
    (1..=32).contains(&s.len()) && !s.chars().any(char::is_control)
}

fn valid_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// `nmcli -t -f NAME,UUID,TYPE,ACTIVE connection show`: Wi-Fi profiles only.
pub fn parse_profiles(text: &str) -> Vec<Profile> {
    text.lines()
        .filter_map(|l| {
            let f = split_terse(l);
            let [name, uuid, kind, active] = f.as_slice() else {
                return None;
            };
            (kind == "802-11-wireless" && valid_uuid(uuid)).then(|| Profile {
                name: name.clone(),
                uuid: uuid.clone(),
                active: active == "yes",
            })
        })
        .collect()
}

/// `nmcli -t -f IN-USE,SSID,SECURITY,SIGNAL device wifi list`: one entry
/// per network name (hidden networks skipped), the network in use first,
/// then by signal strength. Saved profiles are matched by name.
pub fn parse_scan(text: &str, profiles: &[Profile]) -> Vec<Network> {
    let mut nets: Vec<Network> = Vec::new();
    for line in text.lines() {
        let f = split_terse(line);
        let [in_use, ssid, security, signal] = f.as_slice() else {
            continue;
        };
        if ssid.is_empty() {
            continue;
        }
        let signal = signal.trim().parse::<u8>().unwrap_or(0).min(100);
        let in_use = in_use.trim() == "*";
        match nets.iter_mut().find(|n| n.ssid == *ssid) {
            Some(n) => {
                n.in_use |= in_use;
                if signal > n.signal {
                    n.signal = signal;
                    n.security = Security::from_nmcli(security);
                }
            }
            None => nets.push(Network {
                ssid: ssid.clone(),
                security: Security::from_nmcli(security),
                signal,
                in_use,
                saved: profiles.iter().find(|p| p.name == *ssid).cloned(),
            }),
        }
    }
    nets.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    nets
}

/// Signal strength as four ASCII bars.
pub fn bars(signal: u8) -> &'static str {
    match signal {
        0..=19 => "[    ]",
        20..=39 => "[#   ]",
        40..=59 => "[##  ]",
        60..=79 => "[### ]",
        _ => "[####]",
    }
}

/// State of the Wi-Fi hardware as NetworkManager sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overview {
    /// Wi-Fi devices managed by NetworkManager.
    pub devices: usize,
    /// The Wi-Fi radio switch (software; a hardware switch shows as off).
    pub radio_on: bool,
    pub networks: Vec<Network>,
}

/// `nmcli -t -f TYPE device`: number of Wi-Fi devices.
pub fn count_wifi_devices(text: &str) -> usize {
    text.lines().filter(|l| l.trim() == "wifi").count()
}

fn nmcli(nmcli: &str, args: &[&str]) -> Result<String, String> {
    let mut argv = vec![nmcli];
    argv.extend_from_slice(args);
    match run_full(&argv, TIMEOUT) {
        Some(o) if o.success() => Ok(o.stdout),
        Some(o) => Err(clean_error(&o.stderr)),
        None => Err("NetworkManager did not answer in time".into()),
    }
}

/// First line of an nmcli error, without the `Error: ` prefix.
pub fn clean_error(stderr: &str) -> String {
    let line = stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = line.trim().trim_start_matches("Error: ").trim();
    if line.is_empty() {
        "nmcli failed without a message".into()
    } else {
        line.to_string()
    }
}

/// Everything the Wi-Fi screen shows, from cached scan results (no wait).
pub fn overview(nmcli_path: &str) -> Result<Overview, String> {
    let devices = count_wifi_devices(&nmcli(nmcli_path, &["-t", "-f", "TYPE", "device"])?);
    let radio_on = nmcli(nmcli_path, &["-t", "radio", "wifi"])?.trim() == "enabled";
    if devices == 0 || !radio_on {
        return Ok(Overview {
            devices,
            radio_on,
            networks: Vec::new(),
        });
    }
    let profiles = parse_profiles(&nmcli(
        nmcli_path,
        &["-t", "-f", "NAME,UUID,TYPE,ACTIVE", "connection", "show"],
    )?);
    let scan = nmcli(
        nmcli_path,
        &[
            "-t",
            "-f",
            "IN-USE,SSID,SECURITY,SIGNAL",
            "device",
            "wifi",
            "list",
            "--rescan",
            "no",
        ],
    )?;
    Ok(Overview {
        devices,
        radio_on,
        networks: parse_scan(&scan, &profiles),
    })
}

/// Ask for a new scan; results appear in later `overview` calls.
pub fn request_scan(nmcli_path: &str) -> Result<(), String> {
    nmcli(nmcli_path, &["device", "wifi", "rescan"]).map(|_| ())
}

pub fn radio_on(nmcli_path: &str) -> Result<(), String> {
    nmcli(nmcli_path, &["radio", "wifi", "on"]).map(|_| ())
}

pub fn disconnect(nmcli_path: &str, p: &Profile) -> Result<(), String> {
    if !valid_uuid(&p.uuid) {
        return Err("invalid profile".into());
    }
    nmcli(nmcli_path, &["connection", "down", "uuid", &p.uuid]).map(|_| ())
}

pub fn forget(nmcli_path: &str, uuid: &str) -> Result<(), String> {
    if !valid_uuid(uuid) {
        return Err("invalid profile".into());
    }
    nmcli(nmcli_path, &["connection", "delete", "uuid", uuid]).map(|_| ())
}

/// A random (version 4) UUID for a new profile, so that the profile can be
/// addressed exactly even if another one has the same name.
pub fn new_uuid() -> std::io::Result<String> {
    let mut b = [0u8; 16];
    std::io::Read::read_exact(&mut std::fs::File::open("/dev/urandom")?, &mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}

/// argv that creates a profile for `net` owned by `user` alone, without
/// any secret. The password is asked for later by `connect_argv`.
pub fn add_profile_argv(
    nmcli_path: &str,
    net: &Network,
    uuid: &str,
    user: &str,
) -> Result<Vec<String>, String> {
    if !valid_ssid(&net.ssid) {
        return Err("this network name cannot be used".into());
    }
    if !valid_user_name(user) {
        return Err("invalid user name".into());
    }
    if !valid_uuid(uuid) {
        return Err("invalid profile".into());
    }
    let mut argv: Vec<String> = [
        nmcli_path,
        "connection",
        "add",
        "type",
        "wifi",
        "ifname",
        "*",
        "con-name",
        &net.ssid,
        "ssid",
        &net.ssid,
        "connection.uuid",
        uuid,
        "connection.permissions",
        &format!("user:{user}"),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let key_mgmt = match &net.security {
        Security::Open => None,
        Security::Psk => Some("wpa-psk"),
        Security::Sae => Some("sae"),
        Security::Unsupported(_) => return Err("this kind of security is not supported".into()),
    };
    if let Some(k) = key_mgmt {
        // psk-flags 0: NetworkManager keeps the password in its own
        // root-only file, so a saved network reconnects without asking.
        for a in ["wifi-sec.key-mgmt", k, "wifi-sec.psk-flags", "0"] {
            argv.push(a.into());
        }
    }
    Ok(argv)
}

pub fn add_profile(argv: &[String]) -> Result<(), String> {
    let a: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (prog, rest) = a.split_first().ok_or("empty command")?;
    nmcli(prog, rest).map(|_| ())
}

/// Is this a usable WPA personal password? 8–63 printable ASCII
/// characters, or exactly 64 hexadecimal digits (a raw key). nmcli's
/// password file trims spaces around values, so those are refused.
pub fn check_password(p: &str) -> Result<(), &'static str> {
    let hex = p.len() == 64 && p.chars().all(|c| c.is_ascii_hexdigit());
    if hex {
        return Ok(());
    }
    if !p.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        return Err("Wi-Fi passwords use only plain letters, digits and symbols");
    }
    if p.len() < 8 {
        return Err("Wi-Fi passwords have at least 8 characters");
    }
    if p.len() > 63 {
        return Err("Wi-Fi passwords have at most 63 characters");
    }
    if p.starts_with(' ') || p.ends_with(' ') {
        return Err("a password starting or ending with a space is not supported here");
    }
    Ok(())
}

/// argv to bring a profile up, giving up after `CONNECT_WAIT_SECS`. With
/// `with_password`, nmcli reads the password from its stdin; without it,
/// nmcli never asks (no `--ask`) and fails if a password is needed.
pub fn connect_argv(
    nmcli_path: &str,
    uuid: &str,
    with_password: bool,
) -> Result<Vec<String>, String> {
    if !valid_uuid(uuid) {
        return Err("invalid profile".into());
    }
    let wait = CONNECT_WAIT_SECS.to_string();
    let mut argv = vec![
        nmcli_path,
        "--wait",
        &wait,
        "connection",
        "up",
        "uuid",
        uuid,
    ];
    if with_password {
        argv.extend(["passwd-file", "/dev/stdin"]);
    }
    Ok(argv.iter().map(|s| s.to_string()).collect())
}

/// Overwrite a secret's bytes before freeing it (best effort: copies made
/// by the allocator or the kernel's pipe are out of reach).
pub fn wipe(secret: String) {
    let mut b = secret.into_bytes();
    b.iter_mut().for_each(|x| *x = 0);
    std::hint::black_box(&b);
}

/// Connect, blocking (run it in a thread). The password, if any, goes to
/// nmcli's stdin in its password-file format and is wiped afterwards.
pub fn connect(nmcli_path: &str, uuid: &str, password: Option<String>) -> Result<(), String> {
    let argv = connect_argv(nmcli_path, uuid, password.is_some())?;
    let a: Vec<&str> = argv.iter().map(String::as_str).collect();
    let timeout = Duration::from_secs(u64::from(CONNECT_WAIT_SECS) + 15);
    let out = match password {
        Some(p) => {
            if let Err(e) = check_password(&p) {
                wipe(p);
                return Err(e.into());
            }
            let input = format!("802-11-wireless-security.psk:{p}\n");
            wipe(p);
            let out = run_with_input(&a, input.as_bytes(), timeout);
            wipe(input);
            out
        }
        None => run_full(&a, timeout),
    };
    match out {
        Some(o) if o.success() => Ok(()),
        Some(o) => Err(clean_error(&o.stderr)),
        None => Err("the connection attempt took too long and was stopped".into()),
    }
}

/// A short explanation for a failed attempt, if there is a likely one.
pub fn failure_hint(error: &str, with_password: bool) -> Option<&'static str> {
    if needs_password(error) {
        Some("The password is probably wrong.")
    } else if with_password && error.to_ascii_lowercase().contains("timeout") {
        // Observed in vm-wifi: a wrong WPA password ends as a timeout.
        Some(
            "The network did not accept the password in time: it is probably \
             wrong, or the network is too far away.",
        )
    } else {
        None
    }
}

/// Did NetworkManager fail because the password was missing or refused?
pub fn needs_password(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    e.contains("secrets were required") || e.contains("no secrets") || e.contains(".psk")
}

/// True if the program exists, for a clear message on systems without
/// NetworkManager.
pub fn available(nmcli_path: &str) -> bool {
    let p = Path::new(nmcli_path);
    !p.is_absolute() || p.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terse_fields_unescape() {
        assert_eq!(split_terse("a:b"), vec!["a", "b"]);
        assert_eq!(
            split_terse(r"Caf\: 2:WPA2:70"),
            vec!["Caf: 2", "WPA2", "70"]
        );
        assert_eq!(split_terse(r"back\\slash::x"), vec![r"back\slash", "", "x"]);
        assert_eq!(split_terse(""), vec![""]);
    }

    #[test]
    fn security_kinds() {
        assert_eq!(Security::from_nmcli("WPA2"), Security::Psk);
        assert_eq!(Security::from_nmcli("WPA1 WPA2"), Security::Psk);
        assert_eq!(Security::from_nmcli("WPA2 WPA3"), Security::Psk);
        assert_eq!(Security::from_nmcli("WPA3"), Security::Sae);
        assert_eq!(Security::from_nmcli(""), Security::Open);
        assert_eq!(Security::from_nmcli("--"), Security::Open);
        assert!(matches!(
            Security::from_nmcli("WPA2 802.1X"),
            Security::Unsupported(_)
        ));
        assert!(matches!(
            Security::from_nmcli("WEP"),
            Security::Unsupported(_)
        ));
        assert!(matches!(
            Security::from_nmcli("OWE"),
            Security::Unsupported(_)
        ));
        assert_eq!(Security::from_nmcli("WPA2 802.1X").label(), "802.1X");
    }

    const PROFILES: &str = "\
Wired connection 1:5f0c1a52-1b6f-3a39-9d1a-2a4c9d2c8e01:802-3-ethernet:no
Home\\: upstairs:0b7e1d0a-9a7c-4c1e-8f53-4f5f2b2c9a10:802-11-wireless:yes
Cafe:3c1f6a2e-7d5b-4e9a-b2a1-1e2d3c4b5a69:802-11-wireless:no
Broken:not-a-uuid:802-11-wireless:no
";

    #[test]
    fn profiles_are_wifi_only_with_valid_uuids() {
        let p = parse_profiles(PROFILES);
        assert_eq!(p.len(), 2, "{p:?}");
        assert_eq!(p[0].name, "Home: upstairs");
        assert!(p[0].active);
        assert!(!p[1].active);
    }

    #[test]
    fn scan_dedupes_sorts_and_matches_saved_profiles() {
        let scan = "\
:Cafe:WPA2:40
:Cafe:WPA2:75
*:Home\\: upstairs:WPA1 WPA2:55
::WPA2:90
:Library:--:62
:Office:WPA2 802.1X:80
";
        let nets = parse_scan(scan, &parse_profiles(PROFILES));
        let names: Vec<&str> = nets.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, ["Home: upstairs", "Office", "Cafe", "Library"]);
        assert!(nets[0].in_use && nets[0].saved.as_ref().is_some_and(|p| p.active));
        assert_eq!(nets[2].signal, 75, "strongest access point kept");
        assert!(nets[2].saved.is_some());
        assert_eq!(nets[3].security, Security::Open);
        assert!(nets[3].saved.is_none());
        assert!(nets[1].problem().is_some_and(|p| p.contains("802.1X")));
        assert!(nets[3].problem().is_none());
    }

    #[test]
    fn devices_and_errors() {
        assert_eq!(
            count_wifi_devices("ethernet\nwifi\nwifi-p2p\nloopback\n"),
            1
        );
        assert_eq!(
            clean_error("Error: NetworkManager is not running.\n"),
            "NetworkManager is not running."
        );
        assert_eq!(clean_error(""), "nmcli failed without a message");
    }

    fn net(ssid: &str, security: Security) -> Network {
        Network {
            ssid: ssid.into(),
            security,
            signal: 50,
            in_use: false,
            saved: None,
        }
    }

    #[test]
    fn new_profiles_belong_to_the_user_and_carry_no_secret() {
        let uuid = new_uuid().expect("urandom");
        assert!(valid_uuid(&uuid), "{uuid}");
        assert_eq!(&uuid[14..15], "4");
        let argv = add_profile_argv("/bin/nmcli", &net("Cafe", Security::Psk), &uuid, "alice")
            .expect("valid");
        let s = argv.join(" ");
        assert!(s.contains("connection.permissions user:alice"), "{s}");
        assert!(
            s.contains("wifi-sec.key-mgmt wpa-psk wifi-sec.psk-flags 0"),
            "{s}"
        );
        for secret in ["wifi-sec.psk", "802-11-wireless-security.psk", "password"] {
            assert!(
                !argv.iter().any(|x| x == secret),
                "no password in argv: {s}"
            );
        }
        assert!(s.contains(&format!("connection.uuid {uuid}")));
        let open = add_profile_argv(
            "/bin/nmcli",
            &net("Library", Security::Open),
            &uuid,
            "alice",
        )
        .expect("valid");
        assert!(!open.iter().any(|a| a.starts_with("wifi-sec")));
        let sae = add_profile_argv("/bin/nmcli", &net("New", Security::Sae), &uuid, "alice")
            .expect("valid");
        assert!(sae.contains(&"sae".to_string()));
        // Each value stays one argument, whatever it contains.
        let odd = add_profile_argv(
            "/bin/nmcli",
            &net("a b;$(x)", Security::Psk),
            &uuid,
            "alice",
        )
        .expect("valid");
        assert_eq!(odd[8], "a b;$(x)");
        for bad in ["", "tab\there", &"x".repeat(33)] {
            assert!(
                add_profile_argv("/bin/nmcli", &net(bad, Security::Psk), &uuid, "alice").is_err()
            );
        }
        assert!(
            add_profile_argv("/bin/nmcli", &net("Cafe", Security::Psk), &uuid, "Alice;x").is_err()
        );
        assert!(add_profile_argv("/bin/nmcli", &net("Cafe", Security::Psk), "x", "alice").is_err());
        assert!(
            add_profile_argv(
                "/bin/nmcli",
                &net("Office", Security::Unsupported("802.1X".into())),
                &uuid,
                "alice"
            )
            .is_err()
        );
    }

    #[test]
    fn connect_never_prompts_and_reads_the_password_from_stdin() {
        let uuid = "3c1f6a2e-7d5b-4e9a-b2a1-1e2d3c4b5a69";
        assert_eq!(
            connect_argv("/bin/nmcli", uuid, false).expect("valid"),
            [
                "/bin/nmcli",
                "--wait",
                "45",
                "connection",
                "up",
                "uuid",
                uuid
            ]
        );
        let with = connect_argv("/bin/nmcli", uuid, true).expect("valid");
        assert!(with.ends_with(&["passwd-file".into(), "/dev/stdin".into()]));
        assert!(!with.iter().any(|a| a == "--ask"));
        assert!(connect_argv("/bin/nmcli", "Cafe", false).is_err());
    }

    #[test]
    fn the_password_goes_through_stdin_only() {
        // A stand-in for nmcli that records its argv and its stdin.
        let dir = std::env::temp_dir().join(format!("resilience-wifi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let fake = dir.join("nmcli");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\necho \"$@\" > {0}/argv\ncat > {0}/stdin\n",
                dir.display()
            ),
        )
        .expect("write");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let uuid = "3c1f6a2e-7d5b-4e9a-b2a1-1e2d3c4b5a69";
        // Executing a script just written can fail with ETXTBSY while another
        // test thread forks (rust-lang/rust#114554); retry briefly.
        let mut result = Err(String::new());
        for _ in 0..5 {
            result = connect(
                &fake.display().to_string(),
                uuid,
                Some("hunter2: ok!".into()),
            );
            if result.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        result.expect("ok");
        let argv = std::fs::read_to_string(dir.join("argv")).expect("argv");
        let stdin = std::fs::read_to_string(dir.join("stdin")).expect("stdin");
        assert!(!argv.contains("hunter2"), "{argv}");
        assert_eq!(stdin, "802-11-wireless-security.psk:hunter2: ok!\n");
        // A refused password never starts nmcli.
        std::fs::remove_file(dir.join("argv")).expect("rm");
        assert!(connect(&fake.display().to_string(), uuid, Some("short".into())).is_err());
        assert!(!dir.join("argv").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn passwords_are_checked_before_use() {
        assert!(check_password("correct-horse-42").is_ok());
        assert!(check_password("a b c d e").is_ok());
        assert!(check_password(&"f".repeat(64)).is_ok());
        assert!(check_password(&"x".repeat(63)).is_ok());
        for bad in [
            "",
            "1234567",
            &"x".repeat(64),
            " leading8",
            "trailing8 ",
            "tab\there1",
            "pässwörd12",
        ] {
            assert!(check_password(bad).is_err(), "{bad:?}");
        }
        assert!(needs_password(
            "Connection activation failed: Secrets were required, but not provided"
        ));
        assert!(!needs_password(
            "No suitable device found for this connection"
        ));
        let timeout = "Timeout expired (45 seconds)";
        assert!(failure_hint(timeout, true).is_some_and(|h| h.contains("probably wrong")));
        assert_eq!(failure_hint(timeout, false), None);
        assert!(failure_hint("Secrets were required, but not provided", false).is_some());
        assert_eq!(failure_hint("No suitable device found", true), None);
    }

    #[test]
    fn bars_cover_the_range() {
        assert_eq!(bars(0), "[    ]");
        assert_eq!(bars(55), "[##  ]");
        assert_eq!(bars(100), "[####]");
    }
}
