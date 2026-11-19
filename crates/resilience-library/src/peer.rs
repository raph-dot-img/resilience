//! Nearby machines that share (ADR 0025 §5): checking an address, curl's
//! arguments for plain HTTP to a peer, and the loopback relay through
//! which the text browser reads a peer's pages.
//!
//! The relay copies bytes between a connection on `127.0.0.1` and the
//! peer; it parses nothing. lynx keeps `-localhost`, so it never reaches a
//! remote host itself, and a link-local address (which needs an
//! interface, `%bat0`, that lynx cannot handle) works the same way.

use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, SocketAddrV6, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use resilience_core::share::{self, Peer};

/// At most this many connections through the relay at once.
const MAX_CONNECTIONS: usize = 16;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// A peer address given on the command line, accepted only if it is a
/// local address on one of this machine's own networks.
pub fn resolve(arg: &str) -> Result<Peer, String> {
    let p = Peer::parse(arg).ok_or_else(|| {
        format!(
            "{} is not the address of a computer on a local network.",
            resilience_core::printable(arg)
        )
    })?;
    if let Some(i) = &p.interface
        && !Path::new("/sys/class/net").join(i).exists()
    {
        return Err(format!("This computer has no network interface {i}."));
    }
    let r4 = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    let r6 = std::fs::read_to_string("/proc/net/ipv6_route").unwrap_or_default();
    if !share::on_link(&p, &r4, &r6) {
        return Err(format!(
            "{} is not on a network this computer is connected to.",
            p.display()
        ));
    }
    Ok(p)
}

/// The socket address to connect to (the interface's index for a
/// link-local address).
pub fn socket_addr(p: &Peer) -> Result<SocketAddr, String> {
    match (p.address, &p.interface) {
        (IpAddr::V6(a), Some(i)) => {
            let name = std::ffi::CString::new(i.as_str()).map_err(|_| "bad interface name")?;
            // SAFETY: `name` is a valid NUL-terminated string that outlives the call.
            let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
            if index == 0 {
                return Err(format!("This computer has no network interface {i}."));
            }
            Ok(SocketAddr::V6(SocketAddrV6::new(a, p.port, 0, index)))
        }
        (a, _) => Ok(SocketAddr::new(a, p.port)),
    }
}

/// curl's arguments to fetch `path` from a peer: plain HTTP only, no
/// redirects, no proxy, `~/.curlrc` ignored (`-q` first), the URL taken
/// literally (`--globoff`: IPv6 addresses have brackets).
pub fn curl_args(p: &Peer, path: &str, from: u64) -> Vec<String> {
    let mut a: Vec<String> = [
        "-q",
        "--fail",
        "--globoff",
        "--noproxy",
        "*",
        "--proto",
        "=http",
        "--max-redirs",
        "0",
        "--silent",
        "--show-error",
        "--connect-timeout",
        "10",
        "--speed-limit",
        "1024",
        "--speed-time",
        "60",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if from > 0 {
        a.extend(["--continue-at".into(), from.to_string()]);
    }
    a.extend(["--output".into(), "-".into(), "--".into(), p.url(path)]);
    a
}

/// A relay from `127.0.0.1:port` to a peer, until dropped.
pub struct Relay {
    pub port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Relay {
    pub fn start(target: SocketAddr) -> io::Result<Relay> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let s = stop.clone();
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((client, _)) => {
                        if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                            continue; // dropped: the browser tries again
                        }
                        active.fetch_add(1, Ordering::Relaxed);
                        let a = active.clone();
                        std::thread::spawn(move || {
                            let _ = carry(client, target);
                            a.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
        });
        Ok(Relay {
            port,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Copy both ways between the browser's connection and the peer.
fn carry(client: TcpStream, target: SocketAddr) -> io::Result<()> {
    client.set_nonblocking(false)?;
    let peer = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT)?;
    for s in [&client, &peer] {
        s.set_read_timeout(Some(IDLE_TIMEOUT))?;
        s.set_write_timeout(Some(IDLE_TIMEOUT))?;
    }
    let (mut c_in, mut p_out) = (client.try_clone()?, peer.try_clone()?);
    let up = std::thread::spawn(move || {
        let _ = copy(&mut c_in, &mut p_out);
        let _ = p_out.shutdown(Shutdown::Write);
    });
    let (mut p_in, mut c_out) = (peer, client);
    let _ = copy(&mut p_in, &mut c_out);
    let _ = c_out.shutdown(Shutdown::Both);
    let _ = p_in.shutdown(Shutdown::Both);
    let _ = up.join();
    Ok(())
}

fn copy(r: &mut impl Read, w: &mut impl Write) -> io::Result<u64> {
    let mut buf = [0u8; 16 * 1024];
    let mut total = 0;
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => return Ok(total),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        w.write_all(&buf[..n])?;
        total += n as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curl_for_a_peer_is_plain_http_without_redirects_or_proxy() {
        let p = Peer::parse("[fe80::1%eth1]:80").expect("peer");
        let a = curl_args(&p, "/packs/a.zim", 5);
        assert_eq!(a[0], "-q");
        let joined = a.join(" ");
        for want in [
            "--globoff",
            "--noproxy *",
            "--proto =http",
            "--max-redirs 0",
            "--continue-at 5",
        ] {
            assert!(joined.contains(want), "{want}: {joined}");
        }
        assert!(!joined.contains("--location"));
        assert_eq!(
            a[a.len() - 2..],
            [
                "--".to_string(),
                "http://[fe80::1%25eth1]:80/packs/a.zim".to_string()
            ]
        );
        assert!(!curl_args(&p, "/", 0).iter().any(|x| x == "--continue-at"));
    }

    #[test]
    fn the_relay_carries_bytes_both_ways() {
        let server = TcpListener::bind("127.0.0.1:0").expect("bind");
        let target = server.local_addr().expect("addr");
        let echo = std::thread::spawn(move || {
            let (mut s, _) = server.accept().expect("accept");
            let mut buf = [0u8; 5];
            s.read_exact(&mut buf).expect("read");
            s.write_all(b"pong:").expect("w");
            s.write_all(&buf).expect("w");
        });
        let relay = Relay::start(target).expect("relay");
        let mut c = TcpStream::connect(("127.0.0.1", relay.port)).expect("connect");
        c.write_all(b"hello").expect("w");
        let mut got = String::new();
        c.read_to_string(&mut got).expect("read");
        assert_eq!(got, "pong:hello");
        echo.join().expect("echo");
        drop(relay);
    }
}
