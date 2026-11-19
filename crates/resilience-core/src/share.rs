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
    /// A drop-in for the web server's instance: the kernel keeps it to
    /// this one interface (`RestrictNetworkInterfaces`; systemd does not
    /// expand `%i` there). Written before `reload`, then the units start.
    pub web_dropin: Option<(PathBuf, String)>,
    pub reload: Option<Vec<String>>,
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
            web_dropin: None,
            reload: None,
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
    let dropin = PathBuf::from(format!(
        "/run/systemd/system/{}-web@{interface}.service.d/50-interface.conf",
        s.unit
    ));
    Ok(SharePlan {
        stop,
        web_dropin: Some((
            dropin,
            format!(
                "[Service]\nRestrictNetworkInterfaces=\nRestrictNetworkInterfaces=lo {interface}\n"
            ),
        )),
        reload: Some(vec![
            systemctl.to_string_lossy().into_owned(),
            "daemon-reload".into(),
        ]),
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

/// A machine to fetch from (ADR 0025 §5): a local address, the interface
/// for an IPv6 link-local one, and the port. Written `ADDRESS`,
/// `ADDRESS:PORT`, `fe80::1%eth1` or `[fe80::1%eth1]:PORT` (port 80 when
/// absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub address: std::net::IpAddr,
    /// Required for (and only for) an IPv6 link-local address.
    pub interface: Option<String>,
    pub port: u16,
}

impl Peer {
    pub fn parse(s: &str) -> Option<Peer> {
        use std::net::IpAddr;
        if s.len() > 64
            || s.bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        {
            return None;
        }
        // [v6%if]:port | [v6%if] | v4:port | v4 | v6%if | v6
        let (host, port) = if let Some(rest) = s.strip_prefix('[') {
            let (h, after) = rest.split_once(']')?;
            let port = match after {
                "" => 80,
                p => p.strip_prefix(':')?.parse().ok()?,
            };
            (h, port)
        } else if s.matches(':').count() == 1 {
            let (h, p) = s.split_once(':')?;
            (h, p.parse().ok()?)
        } else {
            (s, 80)
        };
        if port == 0 {
            return None;
        }
        let (addr, interface) = match host.split_once('%') {
            Some((a, i)) => (a, Some(i.to_string())),
            None => (host, None),
        };
        let address: IpAddr = addr.parse().ok()?;
        if !is_local_address(&address) {
            return None;
        }
        let link_local_v6 =
            matches!(address, IpAddr::V6(v) if (v.segments()[0] & 0xffc0) == 0xfe80);
        match (&interface, link_local_v6) {
            (Some(i), true) if valid_interface(i) => {}
            (None, false) => {}
            _ => return None,
        }
        Some(Peer {
            address,
            interface,
            port,
        })
    }

    /// `http://…` for `path` (which starts with `/`). The zone of a
    /// link-local address is written `%25IF`, as URLs require.
    pub fn url(&self, path: &str) -> String {
        let host = match (&self.address, &self.interface) {
            (std::net::IpAddr::V6(a), Some(i)) => format!("[{a}%25{i}]"),
            (std::net::IpAddr::V6(a), None) => format!("[{a}]"),
            (a, _) => a.to_string(),
        };
        format!("http://{host}:{}{path}", self.port)
    }

    /// How it is shown and passed on the command line.
    pub fn display(&self) -> String {
        match (&self.address, &self.interface) {
            (std::net::IpAddr::V6(a), Some(i)) => format!("[{a}%{i}]:{}", self.port),
            (std::net::IpAddr::V6(a), None) => format!("[{a}]:{}", self.port),
            (a, _) => format!("{a}:{}", self.port),
        }
    }
}

/// Whether `peer` is on one of this machine's own networks, from the
/// contents of `/proc/net/route` and `/proc/net/ipv6_route`: a route with
/// no gateway that contains it (and, for a link-local address, the named
/// interface). Not the default route: a peer must be on the local link.
pub fn on_link(peer: &Peer, route4: &str, route6: &str) -> bool {
    use std::net::IpAddr;
    match peer.address {
        IpAddr::V4(a) => {
            let a = u32::from(a);
            route4.lines().skip(1).any(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                if f.len() < 8 {
                    return false;
                }
                // Little-endian hexadecimal, as the kernel prints them.
                let hex = |s: &str| u32::from_str_radix(s, 16).ok().map(u32::from_be);
                match (hex(f[1]), hex(f[2]), hex(f[7])) {
                    (Some(dest), Some(0), Some(mask)) => mask != 0 && (a & mask) == (dest & mask),
                    _ => false,
                }
            })
        }
        IpAddr::V6(a) => {
            if let Some(i) = &peer.interface {
                // Link-local: the interface must have its fe80::/64 route.
                return route6.lines().any(|l| {
                    let f: Vec<&str> = l.split_whitespace().collect();
                    f.len() == 10 && f[9] == i && f[0].starts_with("fe80") && f[1] == "40"
                });
            }
            let a = u128::from(a);
            route6.lines().any(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                if f.len() != 10 || f[4] != "00000000000000000000000000000000" {
                    return false;
                }
                let (Ok(dest), Ok(len)) = (
                    u128::from_str_radix(f[0], 16),
                    u32::from_str_radix(f[1], 16),
                ) else {
                    return false;
                };
                if len == 0 || len > 128 {
                    return false;
                }
                let mask = u128::MAX << (128 - len);
                (a & mask) == (dest & mask)
            })
        }
    }
}

/// The catalog packs a peer offers, from its index page (`/`): links
/// `/packs/FILE` whose file is in `catalog`. Bounded; nothing else in the
/// page is used.
pub fn packs_in_index(index: &str, catalog: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in index.split("href=\"/packs/").skip(1).take(10_000) {
        let Some((name, _)) = part.split_once('"') else {
            continue;
        };
        if catalog.contains(&name) && !out.iter().any(|o| o == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// The address to reach a machine heard sharing, from this computer: an
/// IPv4 address on one of its networks first, then a link-local IPv6
/// address on an interface that has one (the mesh's `bat0` first), then
/// a unique-local one on one of its networks.
pub fn peer_for(h: &HeardShare, route4: &str, route6: &str) -> Option<Peer> {
    use std::net::IpAddr;
    let with = |address: IpAddr, interface: Option<String>| Peer {
        address,
        interface,
        port: h.port,
    };
    for a in &h.addresses {
        if a.is_ipv4() {
            let p = with(*a, None);
            if on_link(&p, route4, route6) {
                return Some(p);
            }
        }
    }
    let mut ifaces: Vec<&str> = route6
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() == 10 && f[0].starts_with("fe80") && f[1] == "40" && f[9] != "lo")
                .then_some(f[9])
        })
        .filter(|i| valid_interface(i))
        .collect();
    ifaces.sort_by_key(|i| (*i != "bat0", *i));
    ifaces.dedup();
    for a in &h.addresses {
        if let IpAddr::V6(v) = a
            && (v.segments()[0] & 0xffc0) == 0xfe80
            && let Some(i) = ifaces.first()
        {
            return Some(with(*a, Some(i.to_string())));
        }
    }
    h.addresses
        .iter()
        .filter(|a| a.is_ipv6())
        .map(|a| with(*a, None))
        .find(|p| {
            p.interface.is_none()
                && !matches!(p.address, IpAddr::V6(v) if (v.segments()[0] & 0xffc0) == 0xfe80)
                && on_link(p, route4, route6)
        })
}

/// The largest file taken from a nearby computer's shared folder.
pub const MAX_FOLDER_FILE: u64 = 4 * 1024 * 1024 * 1024;
/// At most this many files are listed.
pub const MAX_FOLDER_FILES: usize = 2000;

/// A file in a nearby computer's shared folder, as its listing says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedFile {
    pub name: String,
    pub size: u64,
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `name` for a URL path: everything but unreserved characters escaped.
pub fn percent_encode(name: &str) -> String {
    name.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// A file name as served: one path component, no dot first, no control
/// characters, at most 255 bytes.
pub fn valid_shared_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 255
        && !n.starts_with('.')
        && !n.contains(['/', '\\'])
        && !n.chars().any(char::is_control)
}

/// The files of nginx's listing of the shared folder (`autoindex` with
/// exact sizes): `<a href="NAME">...</a>  DATE TIME  SIZE`. Folders, the
/// parent link and anything malformed are left out.
pub fn parse_listing(html: &str) -> Vec<SharedFile> {
    let mut out = Vec::new();
    for line in html.lines() {
        let Some(rest) = line.trim_start().strip_prefix("<a href=\"") else {
            continue;
        };
        let Some((href, after)) = rest.split_once('"') else {
            continue;
        };
        if href.ends_with('/') {
            continue;
        }
        let Some(name) = percent_decode(href).filter(|n| valid_shared_name(n)) else {
            continue;
        };
        let Some(size) = after
            .split_whitespace()
            .last()
            .and_then(|t| t.parse::<u64>().ok())
        else {
            continue;
        };
        if !out.iter().any(|f: &SharedFile| f.name == name) {
            out.push(SharedFile { name, size });
        }
        if out.len() >= MAX_FOLDER_FILES {
            break;
        }
    }
    out
}

/// The `[hotspot]` section of admin.toml (ADR 0025 §6).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotspotSettings {
    /// The unit `set-hotspot` starts and stops.
    pub unit: String,
    /// Root-only directory where the helper leaves the password for the
    /// unit (0700).
    pub state_dir: PathBuf,
    /// The network's name.
    pub ssid: String,
    /// Present while the mesh is on: the card cannot do both.
    pub mesh_devices: PathBuf,
}

/// Why `set-hotspot` refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotspotRefusal {
    Unavailable,
    BadState,
    MeshOn,
}

impl std::fmt::Display for HotspotRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            HotspotRefusal::Unavailable => "this system offers no hotspot",
            HotspotRefusal::BadState => "the hotspot can only be turned on or off",
            HotspotRefusal::MeshOn => {
                "the mesh network is on: turn it off first (the Wi-Fi card cannot do both)"
            }
        })
    }
}

/// `set-hotspot on|off`: the systemctl argv, and whether it turns it on.
pub fn plan_set_hotspot(
    settings: Option<&HotspotSettings>,
    systemctl: Option<&Path>,
    state: &str,
    mesh_on: bool,
) -> Result<(Vec<String>, bool), HotspotRefusal> {
    let (Some(s), Some(sc)) = (settings, systemctl) else {
        return Err(HotspotRefusal::Unavailable);
    };
    let unit_ok = s.unit.ends_with(".service")
        && unit_ok(s.unit.trim_end_matches(".service"))
        && sc.is_absolute()
        && s.state_dir.is_absolute();
    if !unit_ok {
        return Err(HotspotRefusal::Unavailable);
    }
    let on = match state {
        "on" => true,
        "off" => false,
        _ => return Err(HotspotRefusal::BadState),
    };
    if on && mesh_on {
        return Err(HotspotRefusal::MeshOn);
    }
    let verb = if on { "start" } else { "stop" };
    Ok((
        vec![
            sc.to_string_lossy().into_owned(),
            verb.into(),
            "--".into(),
            s.unit.clone(),
        ],
        on,
    ))
}

/// A hotspot password from 16 random bytes: four groups of four
/// characters from 32 that cannot be confused when read aloud or copied
/// (no 0/o, 1/l/i), 80 bits. WPA2 needs 8 to 63 characters.
pub fn hotspot_password(random: &[u8; 16]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghjkmnpqrstuvwxyz23456789#";
    let mut out = String::with_capacity(19);
    for (i, b) in random.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        let c = ALPHABET[(b & 31) as usize];
        // '#' would be awkward to say: use the next letter instead.
        out.push(if c == b'#' { 'x' } else { c as char });
    }
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
        let p2 = plan(&["on", "wlan0", "--name", "x", "packs"], UP).expect("plan");
        let (path, text) = p2.web_dropin.expect("drop-in");
        assert_eq!(
            path,
            PathBuf::from(
                "/run/systemd/system/resilience-share-web@wlan0.service.d/50-interface.conf"
            )
        );
        assert_eq!(
            text,
            "[Service]\nRestrictNetworkInterfaces=\nRestrictNetworkInterfaces=lo wlan0\n"
        );
        assert_eq!(p2.reload.expect("reload"), [SC, "daemon-reload"]);
        let p = plan(&["on", "bat0", "--name", "x", "files"], UP).expect("plan");
        assert_eq!(p.start.expect("start").len(), 5, "no library unit");
    }

    #[test]
    fn off_only_stops() {
        let p = plan(&["off"], None).expect("off");
        assert_eq!(p.stop[1], "stop");
        assert!(p.files.is_empty() && p.start.is_none() && p.web_dropin.is_none());
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

    #[test]
    fn peers() {
        let p = Peer::parse("192.168.1.5").expect("v4");
        assert_eq!((p.port, p.interface.as_deref()), (80, None));
        assert_eq!(p.url("/packs/a.zim"), "http://192.168.1.5:80/packs/a.zim");
        assert_eq!(Peer::parse("10.0.0.2:8080").expect("port").port, 8080);
        let l = Peer::parse("[fe80::1%eth1]:80").expect("link-local");
        assert_eq!(l.interface.as_deref(), Some("eth1"));
        assert_eq!(l.url("/"), "http://[fe80::1%25eth1]:80/");
        assert_eq!(l.display(), "[fe80::1%eth1]:80");
        assert_eq!(
            Peer::parse("fe80::1%bat0"),
            Peer::parse("[fe80::1%bat0]:80")
        );
        assert_eq!(Peer::parse(&l.display()), Some(l));
        assert!(Peer::parse("fd00::5").is_some());
        for bad in [
            "",
            "8.8.8.8",
            "127.0.0.1",
            "example.org",
            "192.168.1.5:0",
            "192.168.1.5:99999",
            "fe80::1",
            "[fe80::1]:80",
            "fd00::5%eth1",
            "fe80::1%a/b",
            "192.168.1.5%eth1",
            "192.168.1.5 ",
            "[fe80::1%eth1]x",
            "http://192.168.1.5/",
            "2001:db8::1",
        ] {
            assert!(Peer::parse(bad).is_none(), "{bad:?}");
        }
    }

    const ROUTE4: &str =
        "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0
eth1\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0
";
    const ROUTE6: &str = "fe800000000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001     eth1
fd000000000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001     eth1
00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00000003     eth0
";

    #[test]
    fn peers_on_this_machines_networks_only() {
        let on = |s: &str| on_link(&Peer::parse(s).expect(s), ROUTE4, ROUTE6);
        assert!(on("192.168.1.7"));
        assert!(!on("192.168.2.7"), "only through the default route");
        assert!(!on("10.0.0.1"));
        assert!(on("fe80::5%eth1"));
        assert!(!on("fe80::5%eth9"));
        assert!(on("fd00::7"));
        assert!(!on("fd01::7"));
        assert!(!on_link(&Peer::parse("192.168.1.7").expect("p"), "", ""));
    }

    #[test]
    fn packs_offered_by_a_peer() {
        let index = r#"<ul><li><a href="/packs/a_en.zim">a_en.zim</a></li>
            <li><a href="/packs/evil.zim">evil</a></li><li><a href="/packs/a_en.zim">again</a>
            <a href="/packs/b.mbtiles">b</a><a href="/library/">Library</a>"#;
        assert_eq!(
            packs_in_index(index, &["a_en.zim", "b.mbtiles", "c.zim"]),
            ["a_en.zim", "b.mbtiles"]
        );
        assert!(packs_in_index("", &["a"]).is_empty());
    }

    #[test]
    fn the_address_to_reach_a_heard_machine() {
        let h = |addrs: &[&str]| HeardShare {
            destination: "0".repeat(32),
            name: "x".into(),
            services: vec![Service::Packs],
            port: 80,
            addresses: addrs.iter().map(|a| a.parse().expect(a)).collect(),
            heard: 0,
            identity: String::new(),
        };
        let p = peer_for(&h(&["fe80::9", "192.168.1.9"]), ROUTE4, ROUTE6).expect("v4");
        assert_eq!(p.display(), "192.168.1.9:80");
        let p = peer_for(&h(&["10.9.9.9", "fe80::9"]), ROUTE4, ROUTE6).expect("v6");
        assert_eq!(p.display(), "[fe80::9%eth1]:80");
        let mesh = format!(
            "{ROUTE6}fe800000000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001     bat0\n"
        );
        assert_eq!(
            peer_for(&h(&["fe80::9"]), ROUTE4, &mesh)
                .expect("mesh")
                .interface
                .as_deref(),
            Some("bat0")
        );
        assert_eq!(
            peer_for(&h(&["fd00::9"]), ROUTE4, ROUTE6)
                .expect("ula")
                .display(),
            "[fd00::9]:80"
        );
        assert!(peer_for(&h(&["10.9.9.9", "fd01::9"]), ROUTE4, ROUTE6).is_none());
        assert!(peer_for(&h(&["fe80::9"]), "", "").is_none());
    }

    #[test]
    fn shared_folder_listing() {
        let html = "<html><head><title>Index of /files/</title></head>\n<body>\n<h1>Index of /files/</h1><hr><pre><a href=\"../\">../</a>\n\
<a href=\"sub/\">sub/</a>                                               19-Nov-2026 12:00                   -\n\
<a href=\"notice.txt\">notice.txt</a>                                         19-Nov-2026 12:00                  30\n\
<a href=\"Plan%20de%20l%27%C3%A9cole.pdf\">Plan de l&apos;école.pdf</a>      19-Nov-2026 12:00              123456\n\
<a href=\"%2Eevil\">x</a>  19-Nov-2026 12:00  5\n\
<a href=\"a%2Fb\">x</a>  19-Nov-2026 12:00  5\n\
<a href=\"bad%0Aname\">x</a>  19-Nov-2026 12:00  5\n\
<a href=\"nosize\">x</a>  19-Nov-2026 12:00  -\n\
<a href=\"%zz\">x</a>  19-Nov-2026 12:00  5\n\
</pre><hr></body></html>";
        let got = parse_listing(html);
        assert_eq!(
            got,
            [
                SharedFile {
                    name: "notice.txt".into(),
                    size: 30
                },
                SharedFile {
                    name: "Plan de l'école.pdf".into(),
                    size: 123456
                },
            ]
        );
        assert_eq!(
            percent_encode("Plan de l'école.pdf"),
            "Plan%20de%20l%27%C3%A9cole.pdf"
        );
        assert_eq!(
            percent_decode(&percent_encode("a b%c")).as_deref(),
            Some("a b%c")
        );
        assert!(parse_listing("").is_empty());
    }

    #[test]
    fn hotspot_rules() {
        let s = HotspotSettings {
            unit: "resilience-hotspot.service".into(),
            state_dir: "/run/resilience-hotspot".into(),
            ssid: "Resilience-hotspot".into(),
            mesh_devices: "/run/resilience-mesh/devices".into(),
        };
        let sc = Path::new(SC);
        let (argv, on) = plan_set_hotspot(Some(&s), Some(sc), "on", false).expect("on");
        assert_eq!(argv, [SC, "start", "--", "resilience-hotspot.service"]);
        assert!(on);
        assert_eq!(
            plan_set_hotspot(Some(&s), Some(sc), "off", true)
                .expect("off")
                .0[1],
            "stop"
        );
        assert_eq!(
            plan_set_hotspot(Some(&s), Some(sc), "on", true),
            Err(HotspotRefusal::MeshOn)
        );
        for bad in ["", "ON", "restart", "on --x"] {
            assert_eq!(
                plan_set_hotspot(Some(&s), Some(sc), bad, false),
                Err(HotspotRefusal::BadState)
            );
        }
        assert_eq!(
            plan_set_hotspot(None, Some(sc), "on", false),
            Err(HotspotRefusal::Unavailable)
        );
        let mut bad = s.clone();
        bad.unit = "x;y.service".into();
        assert_eq!(
            plan_set_hotspot(Some(&bad), Some(sc), "on", false),
            Err(HotspotRefusal::Unavailable)
        );
    }

    #[test]
    fn hotspot_passwords() {
        let p = hotspot_password(&[0u8; 16]);
        assert_eq!(p, "aaaa-aaaa-aaaa-aaaa");
        let q = hotspot_password(&[31, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        assert_eq!(q.len(), 19);
        assert!((8..=63).contains(&q.len()));
        assert!(
            q.chars()
                .all(|c| c == '-' || c.is_ascii_lowercase() || c.is_ascii_digit()),
            "{q}"
        );
        assert!(!q.contains(['0', '1', 'l', 'i', 'o', '#']), "{q}");
    }
}
