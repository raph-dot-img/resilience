//! Community network (ADR 0025): the rules of `resilience-admin
//! set-share`, and the state file the share units write for the
//! interface. Pure: nothing here touches the system.
//!
//! ```text
//! resilience-admin set-share on INTERFACE --name NAME SERVICE...
//! resilience-admin set-share off
//! ```

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Runtime directory of the share units (root, 0755).
pub const STATE_DIR: &str = "/run/resilience-share";

/// The services a machine can share (ADR 0025 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Service {
    Packs,
    Library,
    Files,
    Pages,
}

impl Service {
    pub const ALL: [Service; 4] = [
        Service::Packs,
        Service::Library,
        Service::Files,
        Service::Pages,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Service::Packs => "packs",
            Service::Library => "library",
            Service::Files => "files",
            Service::Pages => "pages",
        }
    }

    pub fn parse(s: &str) -> Option<Service> {
        Service::ALL.into_iter().find(|v| v.id() == s)
    }
}

/// The `[share]` section of admin.toml (only where sharing is offered).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareSettings {
    /// Prefix of the template units: `<unit>@IFACE.service`,
    /// `<unit>-web@IFACE.service`, `<unit>-library@IFACE.service`.
    pub unit: String,
    /// The services this system offers (ids of [`Service`]).
    pub services: Vec<String>,
    /// Where the helper writes the request for the units (normally
    /// [`STATE_DIR`]).
    pub state_dir: PathBuf,
}

/// The name other machines see: 1 to 32 of `[A-Za-z0-9 -]`, starting
/// and ending with a letter or digit. Every installed machine has the
/// host name `resilience`, so the host name cannot serve (ADR 0022).
pub fn valid_name(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=32).contains(&b.len())
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b' ' || *c == b'-')
        && b[0].is_ascii_alphanumeric()
        && b[b.len() - 1].is_ascii_alphanumeric()
}

/// A network interface name the helper accepts: 1 to 15 of
/// `[A-Za-z0-9_.]`, starting with a letter, never the loopback. No `-`,
/// which systemd escapes in device unit names.
pub fn valid_interface(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=15).contains(&b.len())
        && b[0].is_ascii_alphabetic()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'.')
        && s != "lo"
}

/// What the caller asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareRequest {
    On {
        interface: String,
        name: String,
        services: Vec<Service>,
    },
    Off,
}

/// Why `set-share` refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareRefusal {
    /// Sharing is not offered on this system.
    Unavailable,
    /// The arguments are not `on IFACE --name NAME SERVICE...` or `off`.
    Usage,
    BadInterface,
    NoSuchInterface,
    InterfaceDown,
    Loopback,
    BadName,
    UnknownService(String),
    NotOffered(Service),
    Duplicate(Service),
    NoService,
}

impl std::fmt::Display for ShareRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShareRefusal::Unavailable => f.write_str("sharing is not offered on this system"),
            ShareRefusal::Usage => {
                f.write_str("usage: set-share on INTERFACE --name NAME SERVICE... | set-share off")
            }
            ShareRefusal::BadInterface => f.write_str("not a network interface name"),
            ShareRefusal::NoSuchInterface => f.write_str("this network interface does not exist"),
            ShareRefusal::InterfaceDown => f.write_str("this network interface is not up"),
            ShareRefusal::Loopback => f.write_str("the loopback interface cannot be shared on"),
            ShareRefusal::BadName => f.write_str(
                "the name must be 1 to 32 letters, digits, spaces or dashes, \
                 starting and ending with a letter or digit",
            ),
            ShareRefusal::UnknownService(s) => {
                write!(f, "unknown service {:?}", crate::printable(s))
            }
            ShareRefusal::NotOffered(s) => {
                write!(f, "the service {} is not offered on this system", s.id())
            }
            ShareRefusal::Duplicate(s) => write!(f, "the service {} is given twice", s.id()),
            ShareRefusal::NoService => f.write_str("choose at least one service to share"),
        }
    }
}

/// Parse the arguments after `set-share`.
pub fn parse_request(args: &[&str]) -> Result<ShareRequest, ShareRefusal> {
    match args {
        ["off"] => Ok(ShareRequest::Off),
        ["on", interface, "--name", name, services @ ..] => {
            if !valid_interface(interface) {
                return Err(if *interface == "lo" {
                    ShareRefusal::Loopback
                } else {
                    ShareRefusal::BadInterface
                });
            }
            if !valid_name(name) {
                return Err(ShareRefusal::BadName);
            }
            if services.is_empty() {
                return Err(ShareRefusal::NoService);
            }
            let mut out: Vec<Service> = Vec::new();
            for s in services {
                let v =
                    Service::parse(s).ok_or_else(|| ShareRefusal::UnknownService(s.to_string()))?;
                if out.contains(&v) {
                    return Err(ShareRefusal::Duplicate(v));
                }
                out.push(v);
            }
            out.sort();
            Ok(ShareRequest::On {
                interface: interface.to_string(),
                name: name.to_string(),
                services: out,
            })
        }
        _ => Err(ShareRefusal::Usage),
    }
}

/// What the kernel says about an interface (`/sys/class/net/IF/flags`
/// and `type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceFacts {
    pub up: bool,
    pub loopback: bool,
}

const IFF_UP: u32 = 0x1;
const IFF_LOOPBACK: u32 = 0x8;
const ARPHRD_LOOPBACK: u32 = 772;

/// Parse the contents of `flags` (`0x1003`) and `type` (`1`).
pub fn interface_facts(flags: &str, kind: &str) -> Option<InterfaceFacts> {
    let flags = u32::from_str_radix(flags.trim().strip_prefix("0x")?, 16).ok()?;
    let kind: u32 = kind.trim().parse().ok()?;
    Some(InterfaceFacts {
        up: flags & IFF_UP != 0,
        loopback: flags & IFF_LOOPBACK != 0 || kind == ARPHRD_LOOPBACK,
    })
}

/// What the helper does, in order: stop any sharing, write the request
/// files (for `on`), start the units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharePlan {
    pub stop: Vec<String>,
    /// Files to write in the state directory: (name, content).
    pub files: Vec<(&'static str, String)>,
    pub start: Option<Vec<String>>,
}

fn unit_ok(u: &str) -> bool {
    !u.is_empty()
        && u.as_bytes()[0].is_ascii_alphabetic()
        && u.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// `set-share` (ADR 0025): validated against the offered services and
/// the facts about the interface (`None`: it does not exist).
pub fn plan_set_share(
    settings: Option<&ShareSettings>,
    systemctl: Option<&Path>,
    req: &ShareRequest,
    facts: Option<InterfaceFacts>,
) -> Result<SharePlan, ShareRefusal> {
    let (Some(s), Some(systemctl)) = (settings, systemctl) else {
        return Err(ShareRefusal::Unavailable);
    };
    if !unit_ok(&s.unit) || !systemctl.is_absolute() {
        return Err(ShareRefusal::Unavailable);
    }
    let sc = systemctl.to_string_lossy().into_owned();
    let stop = vec![
        sc.clone(),
        "stop".into(),
        "--".into(),
        format!("{}@*.service", s.unit),
    ];
    let ShareRequest::On {
        interface,
        name,
        services,
    } = req
    else {
        return Ok(SharePlan {
            stop,
            files: Vec::new(),
            start: None,
        });
    };
    let facts = facts.ok_or(ShareRefusal::NoSuchInterface)?;
    if facts.loopback {
        return Err(ShareRefusal::Loopback);
    }
    if !facts.up {
        return Err(ShareRefusal::InterfaceDown);
    }
    if let Some(v) = services
        .iter()
        .find(|v| !s.services.iter().any(|o| o == v.id()))
    {
        return Err(ShareRefusal::NotOffered(*v));
    }
    let ids: Vec<&str> = services.iter().map(|v| v.id()).collect();
    let mut start = vec![
        sc,
        "start".into(),
        "--".into(),
        format!("{}@{interface}.service", s.unit),
        format!("{}-web@{interface}.service", s.unit),
    ];
    if services.contains(&Service::Library) {
        start.push(format!("{}-library@{interface}.service", s.unit));
    }
    Ok(SharePlan {
        stop,
        files: vec![
            ("services", format!("{}\n", ids.join(" "))),
            ("name", format!("{name}\n")),
        ],
        start: Some(start),
    })
}

/// `/run/resilience-share/state.toml`, written by the share unit while
/// sharing is on (absent otherwise).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ShareState {
    pub interface: String,
    pub name: String,
    pub services: Vec<String>,
    /// NetworkManager connection active on the interface when sharing
    /// started (empty: not managed by NetworkManager, e.g. the mesh).
    #[serde(default)]
    pub connection: String,
}

impl ShareState {
    pub fn parse(text: &str) -> Option<ShareState> {
        let s: ShareState = toml::from_str(text).ok()?;
        (valid_interface(&s.interface) && valid_name(&s.name)).then_some(s)
    }
}

/// Machines heard sharing on the network are kept this long (seconds).
pub const HEARD_FOR: u64 = 3600;
/// At most this many machines are shown.
pub const MAX_HEARD: usize = 64;

/// An IP address of a local network (ADR 0025 §4): IPv4 private and
/// link-local ranges, IPv6 link-local and unique-local. Anything else is
/// refused, so this feature can never reach the Internet.
pub fn is_local_address(a: &std::net::IpAddr) -> bool {
    match a {
        std::net::IpAddr::V4(v) => v.is_private() || v.is_link_local(),
        std::net::IpAddr::V6(v) => {
            let first = v.segments()[0];
            (first & 0xffc0) == 0xfe80 || (first & 0xfe00) == 0xfc00
        }
    }
}

/// A machine heard sharing, from the account's `shares.json` (written by
/// its messaging program). Everything in it is a claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeardShare {
    /// Its Reticulum destination (32 hexadecimal digits).
    pub destination: String,
    pub name: String,
    pub services: Vec<Service>,
    pub port: u16,
    pub addresses: Vec<std::net::IpAddr>,
    /// When it was last heard (Unix seconds).
    pub heard: u64,
    /// The announcing identity (32 hexadecimal digits), to tell machines
    /// with the same name apart.
    pub identity: String,
}

fn hex32(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Parse `shares.json`, keeping only well-formed entries heard in the last
/// hour with local addresses only, most recently heard first.
pub fn parse_heard(text: &str, now: u64) -> Vec<HeardShare> {
    #[derive(Deserialize)]
    struct Raw {
        name: String,
        services: Vec<String>,
        port: u16,
        addresses: Vec<String>,
        heard: u64,
        #[serde(default)]
        identity: String,
    }
    let Ok(map) =
        serde_json::from_str::<std::collections::BTreeMap<String, serde_json::Value>>(text)
    else {
        return Vec::new();
    };
    let mut out: Vec<HeardShare> = map
        .into_iter()
        .filter(|(k, _)| hex32(k))
        .filter_map(|(k, v)| {
            let r: Raw = serde_json::from_value(v).ok()?;
            if !valid_name(&r.name) || r.port == 0 || r.heard > now + 60 {
                return None;
            }
            if now.saturating_sub(r.heard) > HEARD_FOR {
                return None;
            }
            let mut services = Vec::new();
            for s in &r.services {
                let v = Service::parse(s)?;
                if services.contains(&v) {
                    return None;
                }
                services.push(v);
            }
            if services.is_empty() || r.addresses.is_empty() || r.addresses.len() > 8 {
                return None;
            }
            let addresses = r
                .addresses
                .iter()
                .map(|a| a.parse::<std::net::IpAddr>().ok().filter(is_local_address))
                .collect::<Option<Vec<_>>>()?;
            if !(r.identity.is_empty() || hex32(&r.identity)) {
                return None;
            }
            Some(HeardShare {
                destination: k,
                name: r.name,
                services,
                port: r.port,
                addresses,
                heard: r.heard,
                identity: r.identity,
            })
        })
        .collect();
    out.sort_by(|a, b| b.heard.cmp(&a.heard).then(a.name.cmp(&b.name)));
    out.truncate(MAX_HEARD);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> ShareSettings {
        ShareSettings {
            unit: "resilience-share".into(),
            services: vec![
                "packs".into(),
                "library".into(),
                "files".into(),
                "pages".into(),
            ],
            state_dir: STATE_DIR.into(),
        }
    }

    const UP: Option<InterfaceFacts> = Some(InterfaceFacts {
        up: true,
        loopback: false,
    });
    const SC: &str = "/bin/systemctl";

    fn plan(args: &[&str], facts: Option<InterfaceFacts>) -> Result<SharePlan, ShareRefusal> {
        let req = parse_request(args)?;
        plan_set_share(Some(&settings()), Some(Path::new(SC)), &req, facts)
    }

    #[test]
    fn names_and_interfaces() {
        for ok in ["School library", "A", "shelter-2", "Z9"] {
            assert!(valid_name(ok), "{ok}");
        }
        let long = "a".repeat(33);
        for bad in [
            "",
            " x",
            "x ",
            "-x",
            "x-",
            "café",
            "a\"b",
            "a/b",
            "a\nb",
            long.as_str(),
        ] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        for ok in ["wlan0", "bat0", "enp0s31f6", "eth0.10", "wlp2s0"] {
            assert!(valid_interface(ok), "{ok}");
        }
        for bad in [
            "",
            "lo",
            "-i",
            "0eth",
            "br-lan",
            "a/b",
            "eth0 ",
            "abcdefghijklmnop",
            "*",
        ] {
            assert!(!valid_interface(bad), "{bad:?}");
        }
    }

    #[test]
    fn on_writes_the_request_and_starts_the_units() {
        let p = plan(
            &[
                "on",
                "wlan0",
                "--name",
                "School library",
                "library",
                "packs",
            ],
            UP,
        )
        .expect("plan");
        assert_eq!(p.stop, [SC, "stop", "--", "resilience-share@*.service"]);
        assert_eq!(
            p.files,
            [
                ("services", "packs library\n".to_string()),
                ("name", "School library\n".to_string())
            ]
        );
        assert_eq!(
            p.start.expect("start"),
            [
                SC,
                "start",
                "--",
                "resilience-share@wlan0.service",
                "resilience-share-web@wlan0.service",
                "resilience-share-library@wlan0.service"
            ]
        );
        let p = plan(&["on", "bat0", "--name", "x", "files"], UP).expect("plan");
        assert_eq!(p.start.expect("start").len(), 5, "no library unit");
    }

    #[test]
    fn off_only_stops() {
        let p = plan(&["off"], None).expect("off");
        assert_eq!(p.stop[1], "stop");
        assert!(p.files.is_empty() && p.start.is_none());
    }

    #[test]
    fn refusals() {
        use ShareRefusal::*;
        let cases: &[(&[&str], Option<InterfaceFacts>, ShareRefusal)] = &[
            (&[], UP, Usage),
            (&["on"], UP, Usage),
            (&["on", "wlan0", "packs"], UP, Usage),
            (&["on", "wlan0", "--name"], UP, Usage),
            (&["off", "now"], UP, Usage),
            (&["start", "wlan0"], UP, Usage),
            (&["on", "lo", "--name", "x", "packs"], UP, Loopback),
            (&["on", "a b", "--name", "x", "packs"], UP, BadInterface),
            (&["on", "wlan0", "--name", "", "packs"], UP, BadName),
            (&["on", "wlan0", "--name", "x"], UP, NoService),
            (
                &["on", "wlan0", "--name", "x", "shell"],
                UP,
                UnknownService("shell".into()),
            ),
            (
                &["on", "wlan0", "--name", "x", "packs", "packs"],
                UP,
                Duplicate(Service::Packs),
            ),
            (
                &["on", "wlan0", "--name", "x", "packs"],
                None,
                NoSuchInterface,
            ),
            (
                &["on", "wlan0", "--name", "x", "packs"],
                Some(InterfaceFacts {
                    up: false,
                    loopback: false,
                }),
                InterfaceDown,
            ),
            (
                &["on", "lo0", "--name", "x", "packs"],
                Some(InterfaceFacts {
                    up: true,
                    loopback: true,
                }),
                Loopback,
            ),
        ];
        for (args, facts, want) in cases {
            assert_eq!(plan(args, *facts).as_ref(), Err(want), "{args:?}");
        }
    }

    #[test]
    fn only_offered_services_and_a_configured_system() {
        let mut s = settings();
        s.services = vec!["packs".into()];
        let req = parse_request(&["on", "wlan0", "--name", "x", "packs", "files"]).expect("req");
        assert_eq!(
            plan_set_share(Some(&s), Some(Path::new(SC)), &req, UP),
            Err(ShareRefusal::NotOffered(Service::Files))
        );
        assert_eq!(
            plan_set_share(None, Some(Path::new(SC)), &req, UP),
            Err(ShareRefusal::Unavailable)
        );
        assert_eq!(
            plan_set_share(Some(&s), None, &ShareRequest::Off, UP),
            Err(ShareRefusal::Unavailable)
        );
        for bad in ["", "x y", "a@b", "-a", "a.service"] {
            s.unit = bad.into();
            assert_eq!(
                plan_set_share(Some(&s), Some(Path::new(SC)), &ShareRequest::Off, UP),
                Err(ShareRefusal::Unavailable),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn interface_facts_from_sysfs() {
        assert_eq!(
            interface_facts("0x1003\n", "1\n"),
            Some(InterfaceFacts {
                up: true,
                loopback: false
            })
        );
        assert_eq!(
            interface_facts("0x1002", "1"),
            Some(InterfaceFacts {
                up: false,
                loopback: false
            })
        );
        assert_eq!(
            interface_facts("0x9", "772"),
            Some(InterfaceFacts {
                up: true,
                loopback: true
            })
        );
        assert_eq!(interface_facts("1003", "1"), None);
        assert_eq!(interface_facts("0xzz", "1"), None);
    }

    #[test]
    fn state_file() {
        let s = ShareState::parse(
            "interface = \"wlan0\"\nname = \"School library\"\nservices = [\"packs\"]\nconnection = \"abc\"\n",
        )
        .expect("state");
        assert_eq!(s.interface, "wlan0");
        assert_eq!(s.services, ["packs"]);
        assert!(ShareState::parse("interface = \"lo\"\nname = \"x\"\nservices = []\n").is_none());
        assert!(ShareState::parse("garbage").is_none());
    }

    #[test]
    fn local_addresses_only() {
        for ok in [
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.5",
            "169.254.1.1",
            "fe80::1",
            "fd12::3",
            "fc00::1",
        ] {
            assert!(is_local_address(&ok.parse().expect(ok)), "{ok}");
        }
        for bad in [
            "8.8.8.8",
            "127.0.0.1",
            "172.32.0.1",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "2001:db8::1",
            "::ffff:192.168.1.5",
            "ff02::1",
        ] {
            assert!(!is_local_address(&bad.parse().expect(bad)), "{bad}");
        }
    }

    #[test]
    fn heard_machines() {
        let d1 = "0123456789abcdef0123456789abcdef";
        let d2 = "fedcba9876543210fedcba9876543210";
        let text = format!(
            r#"{{
              "{d1}": {{"name": "School library", "services": ["packs", "library"], "port": 80,
                        "addresses": ["192.168.1.5", "fe80::1"], "heard": 4000, "identity": "{d2}"}},
              "{d2}": {{"name": "Town hall", "services": ["pages"], "port": 80,
                        "addresses": ["10.0.0.2"], "heard": 4500}},
              "badkey": {{"name": "x", "services": ["packs"], "port": 80, "addresses": ["10.0.0.1"], "heard": 4500}},
              "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa": {{"name": "Free", "services": ["packs"], "port": 80, "addresses": ["8.8.8.8"], "heard": 4500}},
              "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb": {{"name": "<b>", "services": ["packs"], "port": 80, "addresses": ["10.0.0.1"], "heard": 4500}},
              "cccccccccccccccccccccccccccccccc": {{"name": "Old", "services": ["packs"], "port": 80, "addresses": ["10.0.0.1"], "heard": 1}},
              "dddddddddddddddddddddddddddddddd": {{"name": "Shell", "services": ["shell"], "port": 80, "addresses": ["10.0.0.1"], "heard": 4500}},
              "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee": {{"name": "Twice", "services": ["packs", "packs"], "port": 80, "addresses": ["10.0.0.1"], "heard": 4500}},
              "ffffffffffffffffffffffffffffffff": {{"name": "Future", "services": ["packs"], "port": 80, "addresses": ["10.0.0.1"], "heard": 99999}}
            }}"#
        );
        let got = parse_heard(&text, 5000);
        let names: Vec<&str> = got.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["Town hall", "School library"]);
        assert_eq!(got[1].services, [Service::Packs, Service::Library]);
        assert_eq!(got[1].addresses.len(), 2);
        assert_eq!(got[1].identity, d2);
        assert!(parse_heard("garbage", 0).is_empty());
        assert!(parse_heard("[]", 0).is_empty());
    }
}
