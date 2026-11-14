//! Network state detection.
//!
//! Two layers, deliberately separated (see ADR-0004):
//!
//! 1. **Passive state** — read from sysfs/procfs only. Never sends a packet.
//!    Distinguishes *offline* (no link), *local link* (link but no default
//!    route) and *routed* (a default route exists; Internet unverified).
//! 2. **Active probe** — only on explicit request. Opens a TCP connection to
//!    configured addresses and resolves one DNS name, each with a hard timeout,
//!    and is skipped entirely when there is no default route.
//!
//! The probe never retries in a loop and always terminates.

use std::ffi::CStr;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfKind {
    Loopback,
    Ethernet,
    Wireless,
    /// Bridges, tunnels, veth and other software interfaces.
    Virtual,
    Other,
}

impl IfKind {
    pub fn label(self) -> &'static str {
        match self {
            IfKind::Loopback => "loopback",
            IfKind::Ethernet => "ethernet",
            IfKind::Wireless => "wi-fi",
            IfKind::Virtual => "virtual",
            IfKind::Other => "other",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    pub kind: IfKind,
    /// Kernel operstate: up, down, dormant, lowerlayerdown, unknown...
    pub operstate: String,
    pub carrier: bool,
    pub addresses: Vec<IpAddr>,
}

impl Interface {
    /// Usable for traffic: carrier present and not administratively down.
    pub fn is_connected(&self) -> bool {
        self.kind != IfKind::Loopback
            && self.carrier
            && matches!(self.operstate.as_str(), "up" | "unknown")
    }
}

/// Passive connectivity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkLevel {
    /// No connected non-loopback interface.
    Offline,
    /// A link exists but there is no default route: local network at most.
    LocalLink,
    /// A default route exists. Internet access is *not* verified.
    Routed,
}

impl LinkLevel {
    pub fn label(self) -> &'static str {
        match self {
            LinkLevel::Offline => "OFFLINE",
            LinkLevel::LocalLink => "LOCAL LINK",
            LinkLevel::Routed => "ROUTE UP",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NetworkState {
    pub interfaces: Vec<Interface>,
    pub gateway_v4: Option<(String, Ipv4Addr)>,
    pub default_v6_iface: Option<String>,
    pub nameservers: Vec<IpAddr>,
    /// Whether the IPv4 gateway has a complete ARP entry (seen on the wire recently).
    pub gateway_arp: Option<bool>,
    pub level: LinkLevel,
}

/// Parse `/proc/net/route` for the IPv4 default gateway.
pub fn parse_default_gateway_v4(route: &str) -> Option<(String, Ipv4Addr)> {
    const RTF_UP: u32 = 0x1;
    const RTF_GATEWAY: u32 = 0x2;
    route.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 || f[1] != "00000000" || f[7] != "00000000" {
            return None;
        }
        let flags = u32::from_str_radix(f[3], 16).ok()?;
        if flags & RTF_UP == 0 || flags & RTF_GATEWAY == 0 {
            return None;
        }
        // The kernel prints the network-order address as a native-endian u32.
        let gw = u32::from_str_radix(f[2], 16).ok()?;
        Some((f[0].to_string(), Ipv4Addr::from(gw.to_ne_bytes())))
    })
}

/// Parse `/proc/net/ipv6_route` for a default route, returning its interface.
pub fn parse_default_route_v6(route: &str) -> Option<String> {
    route.lines().find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 10 || f[0] != "00000000000000000000000000000000" || f[1] != "00" {
            return None;
        }
        // Skip unreachable/reject routes (RTF_REJECT = 0x0200).
        let flags = u32::from_str_radix(f[8], 16).ok()?;
        if flags & 0x0200 != 0 || f[9] == "lo" {
            return None;
        }
        Some(f[9].to_string())
    })
}

pub fn parse_nameservers(resolv: &str) -> Vec<IpAddr> {
    resolv
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == "nameserver").then(|| it.next()?.split('%').next()?.parse().ok())?
        })
        .collect()
}

/// Whether `/proc/net/arp` holds a complete entry (flag 0x2) for `ip`.
pub fn arp_complete(arp: &str, ip: Ipv4Addr) -> bool {
    let ip = ip.to_string();
    arp.lines().skip(1).any(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        f.len() >= 3
            && f[0] == ip
            && u32::from_str_radix(f[2].trim_start_matches("0x"), 16).is_ok_and(|fl| fl & 0x2 != 0)
    })
}

pub fn classify(interfaces: &[Interface], gw4: bool, gw6: bool) -> LinkLevel {
    if !interfaces.iter().any(Interface::is_connected) {
        LinkLevel::Offline
    } else if gw4 || gw6 {
        LinkLevel::Routed
    } else {
        LinkLevel::LocalLink
    }
}

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn if_kind(dir: &Path, name: &str) -> IfKind {
    let arphrd = read_trim(&dir.join("type")).and_then(|t| t.parse::<u32>().ok());
    if name == "lo" || arphrd == Some(772) {
        IfKind::Loopback
    } else if dir.join("wireless").exists() || dir.join("phy80211").exists() {
        IfKind::Wireless
    } else if arphrd == Some(1) && dir.join("device").exists() {
        IfKind::Ethernet
    } else if !dir.join("device").exists() {
        IfKind::Virtual
    } else {
        IfKind::Other
    }
}

/// IPv4/IPv6 addresses per interface via getifaddrs(3).
fn addresses() -> Vec<(String, IpAddr)> {
    let mut out = Vec::new();
    // SAFETY: getifaddrs allocates a linked list that we only read and then
    // release with freeifaddrs. Every pointer is checked for NULL, and the
    // sockaddr casts are guarded by the reported address family.
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            if !ifa.ifa_addr.is_null() && !ifa.ifa_name.is_null() {
                let name = CStr::from_ptr(ifa.ifa_name).to_string_lossy().into_owned();
                match i32::from((*ifa.ifa_addr).sa_family) {
                    libc::AF_INET => {
                        let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                        out.push((
                            name,
                            IpAddr::V4(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr))),
                        ));
                    }
                    libc::AF_INET6 => {
                        let sin6 = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                        out.push((name, IpAddr::V6(Ipv6Addr::from(sin6.sin6_addr.s6_addr))));
                    }
                    _ => {}
                }
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(ifap);
    }
    out
}

pub fn interfaces() -> Vec<Interface> {
    let addrs = addresses();
    let Ok(entries) = fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    let mut out: Vec<Interface> = entries
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let dir = e.path();
            Interface {
                kind: if_kind(&dir, &name),
                operstate: read_trim(&dir.join("operstate")).unwrap_or_else(|| "unknown".into()),
                // Reading `carrier` fails with EINVAL on a down interface.
                carrier: read_trim(&dir.join("carrier")).as_deref() == Some("1"),
                addresses: addrs
                    .iter()
                    .filter(|(n, _)| *n == name)
                    .map(|(_, a)| *a)
                    .collect(),
                name,
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Read the passive network state. Sends no network traffic.
pub fn passive_state() -> NetworkState {
    let interfaces = interfaces();
    let gateway_v4 = fs::read_to_string("/proc/net/route")
        .ok()
        .and_then(|t| parse_default_gateway_v4(&t));
    let default_v6_iface = fs::read_to_string("/proc/net/ipv6_route")
        .ok()
        .and_then(|t| parse_default_route_v6(&t));
    let nameservers = fs::read_to_string("/etc/resolv.conf")
        .map(|t| parse_nameservers(&t))
        .unwrap_or_default();
    let gateway_arp = gateway_v4
        .as_ref()
        .map(|(_, gw)| fs::read_to_string("/proc/net/arp").is_ok_and(|t| arp_complete(&t, *gw)));
    let level = classify(
        &interfaces,
        gateway_v4.is_some(),
        default_v6_iface.is_some(),
    );
    NetworkState {
        interfaces,
        gateway_v4,
        default_v6_iface,
        nameservers,
        gateway_arp,
        level,
    }
}

/// What an active probe is allowed to contact. Comes from the system config.
#[derive(Debug, Clone)]
pub struct ProbeConfig {
    pub targets: Vec<SocketAddr>,
    pub dns_name: String,
    /// Timeout per connection attempt and for the DNS lookup.
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    Skipped(String),
    Ok { detail: String, elapsed: Duration },
    Failed(String),
}

impl ProbeOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, ProbeOutcome::Ok { .. })
    }
}

#[derive(Debug, Clone)]
pub struct ProbeReport {
    pub internet: ProbeOutcome,
    pub dns: ProbeOutcome,
    pub finished: SystemTime,
}

/// Actively test Internet reachability and DNS. Blocks for at most roughly
/// `timeout * (targets + 1)`; call it from a worker thread.
pub fn probe(cfg: &ProbeConfig, level: LinkLevel) -> ProbeReport {
    if level != LinkLevel::Routed {
        let why = format!(
            "not attempted: {} (no default route)",
            level.label().to_lowercase()
        );
        return ProbeReport {
            internet: ProbeOutcome::Skipped(why.clone()),
            dns: ProbeOutcome::Skipped(why),
            finished: SystemTime::now(),
        };
    }
    let internet = probe_tcp(&cfg.targets, cfg.timeout);
    let dns = probe_dns(&cfg.dns_name, cfg.timeout);
    ProbeReport {
        internet,
        dns,
        finished: SystemTime::now(),
    }
}

fn probe_tcp(targets: &[SocketAddr], timeout: Duration) -> ProbeOutcome {
    if targets.is_empty() {
        return ProbeOutcome::Skipped("no probe targets configured".into());
    }
    let mut errors = Vec::new();
    for t in targets {
        let start = Instant::now();
        match TcpStream::connect_timeout(t, timeout) {
            Ok(_) => {
                return ProbeOutcome::Ok {
                    detail: format!("TCP connect to {t}"),
                    elapsed: start.elapsed(),
                };
            }
            Err(e) => errors.push(format!("{t}: {e}")),
        }
    }
    ProbeOutcome::Failed(errors.join("; "))
}

fn probe_dns(name: &str, timeout: Duration) -> ProbeOutcome {
    if name.is_empty() {
        return ProbeOutcome::Skipped("no DNS probe name configured".into());
    }
    // std's resolver has no timeout parameter; run it on a detached thread and
    // stop waiting after `timeout`. The thread ends when the libc resolver
    // gives up on its own (bounded by resolv.conf timeouts).
    let (tx, rx) = mpsc::channel();
    let owned = name.to_string();
    let start = Instant::now();
    thread::spawn(move || {
        let _ = tx.send((owned.as_str(), 443).to_socket_addrs().map(|a| a.count()));
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(n)) if n > 0 => ProbeOutcome::Ok {
            detail: format!("{name} resolved ({n} addresses)"),
            elapsed: start.elapsed(),
        },
        Ok(Ok(_)) => ProbeOutcome::Failed(format!("{name}: no addresses")),
        Ok(Err(e)) => ProbeOutcome::Failed(format!("{name}: {e}")),
        Err(_) => ProbeOutcome::Failed(format!(
            "{name}: timed out after {} ms",
            timeout.as_millis()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTE: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
        eth0\t0000A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n\
        eth0\t00000000\t0102A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";

    fn iface(kind: IfKind, carrier: bool, op: &str) -> Interface {
        Interface {
            name: "x".into(),
            kind,
            operstate: op.into(),
            carrier,
            addresses: vec![],
        }
    }

    #[test]
    fn parses_ipv4_default_gateway() {
        let gw = parse_default_gateway_v4(ROUTE).expect("gateway");
        assert_eq!(gw.0, "eth0");
        if cfg!(target_endian = "little") {
            assert_eq!(gw.1, Ipv4Addr::new(192, 168, 2, 1));
        }
        let only_local = ROUTE.lines().take(2).collect::<Vec<_>>().join("\n");
        assert_eq!(parse_default_gateway_v4(&only_local), None);
        assert_eq!(parse_default_gateway_v4(""), None);
    }

    #[test]
    fn ignores_down_default_routes() {
        let down = "h\neth0\t00000000\t0102A8C0\t0002\t0\t0\t100\t00000000\t0\t0\t0\n";
        assert_eq!(parse_default_gateway_v4(down), None);
    }

    #[test]
    fn parses_ipv6_default_route() {
        let t = "00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00000003 wlan0\n";
        assert_eq!(parse_default_route_v6(t).as_deref(), Some("wlan0"));
        let reject = "00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200 lo\n";
        assert_eq!(parse_default_route_v6(reject), None);
    }

    #[test]
    fn parses_nameservers() {
        let t = "# comment\nnameserver 192.168.1.1\nnameserver fe80::1%eth0\nsearch lan\nnameserver bogus\n";
        assert_eq!(
            parse_nameservers(t),
            vec![
                "192.168.1.1".parse::<IpAddr>().expect("ip"),
                "fe80::1".parse().expect("ip")
            ]
        );
    }

    #[test]
    fn parses_arp() {
        let t = "IP address       HW type     Flags       HW address            Mask     Device\n\
                 192.168.2.1      0x1         0x2         aa:bb:cc:dd:ee:ff     *        eth0\n\
                 192.168.2.9      0x1         0x0         00:00:00:00:00:00     *        eth0\n";
        assert!(arp_complete(t, Ipv4Addr::new(192, 168, 2, 1)));
        assert!(!arp_complete(t, Ipv4Addr::new(192, 168, 2, 9)));
        assert!(!arp_complete(t, Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn classifies_levels() {
        let lo = iface(IfKind::Loopback, true, "unknown");
        let eth_down = iface(IfKind::Ethernet, false, "down");
        let eth_up = iface(IfKind::Ethernet, true, "up");
        assert_eq!(
            classify(std::slice::from_ref(&lo), true, true),
            LinkLevel::Offline
        );
        assert_eq!(
            classify(&[lo.clone(), eth_down], true, false),
            LinkLevel::Offline
        );
        assert_eq!(
            classify(&[lo.clone(), eth_up.clone()], false, false),
            LinkLevel::LocalLink
        );
        assert_eq!(classify(&[lo, eth_up], true, false), LinkLevel::Routed);
    }

    #[test]
    fn probe_is_skipped_without_route() {
        let cfg = ProbeConfig {
            targets: vec!["192.0.2.1:443".parse().expect("addr")],
            dns_name: "example.org".into(),
            timeout: Duration::from_secs(30),
        };
        let start = Instant::now();
        let r = probe(&cfg, LinkLevel::LocalLink);
        assert!(matches!(r.internet, ProbeOutcome::Skipped(_)));
        assert!(matches!(r.dns, ProbeOutcome::Skipped(_)));
        // No network activity, so it must return immediately.
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn tcp_probe_failure_is_bounded() {
        // Port 1 on localhost: refused immediately, never hangs.
        let r = probe_tcp(
            &["127.0.0.1:1".parse().expect("addr")],
            Duration::from_millis(500),
        );
        assert!(matches!(r, ProbeOutcome::Failed(_)));
        assert!(matches!(
            probe_tcp(&[], Duration::from_millis(1)),
            ProbeOutcome::Skipped(_)
        ));
    }

    #[test]
    fn tcp_probe_success_against_local_listener() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = l.local_addr().expect("addr");
        assert!(probe_tcp(&[addr], Duration::from_secs(2)).is_ok());
    }
}
