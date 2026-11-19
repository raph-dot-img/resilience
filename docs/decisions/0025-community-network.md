# ADR 0025: Community network (sharing offline resources with nearby machines)

Status: accepted (2026-11-19, by the owner). Nothing is implemented yet; the slices are
listed at the end.

## Context

Phase 9 (ROADMAP.md, requested 2026-11-18) asks for an assisted tool to
join or create a network of Resilience machines and share offline
resources on it: packs, the library, maps, a folder, simple web pages.
After a disaster one well-stocked computer in a shelter could then serve
a whole street.

This is the first time a Resilience service would **accept requests from
the network**. Today nothing listens beyond `127.0.0.1` except Reticulum's
two UDP ports (ADR 0018), and the firewall allows nothing else (T9, T43).
This ADR settles which services exist, how they are reached, how they are
found and who may turn them on.

What already exists:

- **Networks to share on.** An ordinary Wi-Fi network or Ethernet (ADR
  0010). The mesh (ADR 0021), where `bat0` has only an IPv6 link-local
  address and no IPv4. No hotspot: our polkit rule denies
  NetworkManager's `wifi.share.*` to everyone outside `networkmanager`
  and `wheel` (T27).
- **Discovery.** Reticulum runs on every machine with messaging (on by
  default in the standard edition). Its announces are signed by the
  sending identity and already reach every machine on the LAN or the
  mesh (ADR 0018, 0021).
- **Content with trusted hashes.** Resource packs and map packs are
  pinned by size and SHA-256 in the root-owned catalog. The store accepts
  only bytes with exactly that hash (ADR 0013), whatever they come from.
- **Reading.** `resilience-library open` runs kiwix-serve on
  `127.0.0.1` and lynx with every restriction and `-localhost`, so lynx
  cannot reach any remote host (T33).

Checked in the pinned nixpkgs (2026-11-19):

- nginx 1.30.6 (BSD-2-Clause, NixOS module); darkhttpd 1.17 (static only,
  no proxying); lighttpd 1.4.85; static-web-server 2.42.0.
- kiwix-serve 3.8.2: `--address`, `--port`, `--urlRootLocation`,
  `--ipConnectionLimit`, `--blockexternal`, several ZIM files at once.
- avahi 0.8 (mDNS/DNS-SD daemon). systemd 260.5: systemd-resolved has an
  mDNS responder, publishes DNS-SD services from `.dnssd` files and can
  browse them (`io.systemd.Resolve.BrowseServices`), but it would replace
  the DNS setup of every machine.
- dnsmasq 2.93. NetworkManager's hotspot ("shared" IPv4 method) already
  starts its own dnsmasq for DHCP and DNS on the hotspot, reads extra
  options from `/etc/NetworkManager/dnsmasq-shared.d/`, and NATs the
  hotspot to the machine's default route.
- The firewall is the iptables backend (`networking.nftables.enable` is
  false).

## Decision

### 1. The network is open, and the interface says so

Anyone on the network can join it and read whatever is shared. Before
anything is shared, the TUI says so in plain words: "Everything you
share can be read by anyone on this network. Share only what you would
put on a public notice board." Nothing shared is ever private.

Integrity never depends on trusting the host. Packs are checked against
the catalog's hashes on the receiving machine. Everything else (pages
read in the library, files in a shared folder) is shown as coming from
"a computer that says it is NAME", like names in local messages
(ADR 0018).

### 2. One web server, read-only, started only while sharing

Every service is **HTTP on TCP port 80**, served by **one nginx**
(`resilience-share.service`):

- not started at boot unless the administrator set
  `resilience.share.autoStart` in the configuration (for a computer left
  on in a shelter, like the mesh's `autoStart`);
- `DynamicUser`, `CAP_NET_BIND_SERVICE` only, the usual systemd hardening
  (read-only file system, no new privileges, `MemoryMax`, `TasksMax`,
  address families `AF_INET AF_INET6 AF_UNIX`);
- **only `GET` and `HEAD`**. No uploads, no CGI, no scripts, no
  `autoindex` outside the shared folder, `disable_symlinks on`, files
  starting with `.` refused, small request headers and body size 0;
- **limits against an open network**: a few connections per address
  (`limit_conn`), requests per second per address (`limit_req`), short
  timeouts, a bounded total of connections;
- **no access log** (it would record who read what); errors go to the
  journal.

nginx is chosen because it is the most used and reviewed of the
candidates, and because it can both serve files and stand in front of
kiwix-serve with those limits. darkhttpd cannot proxy.

Paths on the server:

| Path | Service | Served from |
|---|---|---|
| `/` | A plain index page listing what is shared, in text that reads well in lynx | generated at start |
| `/packs/` | Share my packs (ZIM and MBTiles) | `/var/lib/resilience/packs` (read-only) |
| `/library/` | Share my library | kiwix-serve on `127.0.0.1`, behind nginx |
| `/files/` | Shared folder | `/var/lib/resilience/shared/files` |
| `/pages/` | Simple web pages | `/var/lib/resilience/shared/pages` |

Only the services that were turned on are reachable: the others return
404 and are absent from the index page. `/packs/` lists only files
in the catalog (names and sizes from `packs.toml`, written into the index
at start). Built-in packs and map packs are packs: "share my library and
maps" means the library is readable at `/library/` and the map files can
be downloaded at `/packs/`. Map tiles are not served one by one, because
the viewer reads local MBTiles files (ADR 0016).

kiwix-serve for `/library/` is its own unit (`DynamicUser`, hardened,
`--address 127.0.0.1`, `--urlRootLocation /library`, `--blockexternal`,
`--ipConnectionLimit`, `MemoryMax`), serving every installed ZIM pack and
the built-in documentation.

The shared folders `/var/lib/resilience/shared/{files,pages}` belong to
root, group `wheel`, mode `2775`: Advanced accounts put files there
themselves, and a later TUI action copies a file there for them. nginx runs
as a dynamic user, so it serves **only world-readable files**. A file
left private (0600) is not served. The TUI lists exactly what is served.

### 3. Only an administrator shares, every time, and only on one network

The privileged helper (ADR 0007) gets one operation, `set-share`:

```
resilience-admin set-share on NETWORK SERVICE... --name NAME
resilience-admin set-share off
```

- `NETWORK` is one network interface that is up: the Wi-Fi or Ethernet
  connection in use, the hotspot or `bat0`. **Never several and never
  "all"**: the firewall opens TCP 80 only on that interface. The rule is
  in a chain of our own (`resilience-share`, created empty by the
  firewall's `extraCommands`), filled when sharing starts and flushed
  when it stops or the unit fails. Nothing else in the firewall changes.
- `SERVICE` is among `packs library files pages`.
- `NAME` is the name other machines see, `[A-Za-z0-9 -]{1,32}`. Every
  installed machine has the host name `resilience` (ADR 0022), so the
  host name cannot be used.
- Polkit action `org.resilience.admin.set-share`: **`auth_admin` every
  time** (any, inactive, active), never `_keep`. Turning sharing off
  uses the same action, so a stranger at an unattended console cannot
  stop a shelter's server either; the console shows how to stop it.
- The helper writes the choice to `/run/resilience-share/state.toml`
  (root, 0644), starts or stops the units and audits every outcome.
  **Not kept across reboots**, except with `autoStart` (then the network,
  services and name come from the Nix configuration). A laptop moved to
  another network does not keep sharing there.
- The helper also records the NetworkManager connection (UUID) active on
  that interface. Sharing stops if the interface disappears (the units
  are bound to its device unit, `BindsTo=`) or if that connection goes
  down or another one takes the interface (a NetworkManager dispatcher
  script runs `systemctl stop`). The TUI says so. The mesh's `bat0` is
  not managed by NetworkManager: it is covered by the device binding.

The TUI always shows, while sharing is on, a line at the top of the main
screen: "Sharing library, packs on Wi-Fi 'Shelter' as 'School library'.
Stop: Communications → Community network."

### 4. Finding machines: Reticulum announces; typing an address as a fallback

Default naming is Resilience's own, with **no DNS server and no mDNS
daemon**:

- While sharing, a small unprivileged service
  (`resilience-share-announce`, part of the existing
  `resilience-messages` program, `DynamicUser`, attached to the machine's
  Reticulum instance) announces a destination with the aspect
  `resilience.share`. Its app data (msgpack, at most 400 bytes): format
  version, name, services, port and the machine's addresses on the shared
  interface (IPv4, and the IPv6 link-local address on the mesh). It
  announces at start, when an address changes, and every 10 minutes
  while sharing. **Nothing is announced while not sharing** (no
  background traffic, T9).
- Each account's messaging program already hears announces. It records
  the `resilience.share` ones, bounded (at most 64 machines, at most 1
  hour old) in `~/.local/share/resilience/messages/shares/`. The TUI
  reads them like the other message files. Names and addresses are
  claims: shown as `"School library"?`, with the short form of the
  announcing identity.
- On a machine without messaging (minimal edition), or when an
  announce does not arrive, the user can **type an address** (IPv4 or
  `fe80::…` with the interface chosen from a list).

Every address a client uses, announced or typed, must be **on the local
network**: IPv4 private ranges (`10/8`, `172.16/12`, `192.168/16`) and
link-local (`169.254/16`), IPv6 link-local (`fe80::/10`) and unique-local
(`fc00::/7`), on an interface whose network contains it. Anything else is
refused. So this feature can never be used to reach the Internet over
plain HTTP, and a hostile announce cannot point clients at a remote host.

mDNS (avahi, or systemd-resolved) is not used by default (see
Alternatives).

### 5. Joining: everything as the user

Nothing on the receiving side is privileged, except installing a pack,
which goes through the existing store with an administrator's password
(ADR 0013).

- **Packs**: `resilience-library change --from-peer URL --get ID...`
  downloads from `/packs/` with curl and streams the bytes into the
  store exactly as for a download from the Internet. **This is the one
  exception to "HTTPS only"** (ADR 0013), for local addresses only (§4).
  It does not weaken integrity, because the store checks the catalog
  hash. A host that sends other bytes wastes a download, nothing else.
  Resuming works the same way (HTTP ranges).
- **Library and pages**: `resilience-library open-remote URL` starts a
  **relay on `127.0.0.1`** owned by the user (a TCP relay of our own in
  Rust, fixed destination, bounded connections and timeouts, it parses
  nothing) and runs lynx with the same restrictions and `-localhost` as
  for local packs. lynx itself still never reaches a remote host, and the
  existing port-owner check (ADR 0013) applies to the relay's port. IPv6
  link-local addresses need an interface (`%bat0`), which lynx does not
  handle: the relay does.
- **Shared folder**: listed in the TUI from nginx's index (parsed by our
  code with a size cap). A file is downloaded with curl as the user
  into `~/Received/` with the rules of ADR 0020: cleaned name,
  `RENAME_NOREPLACE`, `.partial` until complete, never executable, and a
  size limit shown before downloading. It is then viewed only with the
  read-only viewers of ADR 0014.

### 6. Creating a network: the mesh, or a hotspot

- **The mesh** (ADR 0021) as it is: allowed to the person at the console
  without a password. Sharing on `bat0` still needs an administrator.
- **A hotspot** (new): the helper gains `set-hotspot on|off`, with its
  own polkit action, **`auth_admin`**. A hotspot is only useful to share
  something, which needs an administrator anyway, and it starts a DHCP
  and DNS server (NetworkManager's dnsmasq) on the radio. The helper
  makes it with NetworkManager (a profile it owns, `802-11-wireless.mode
  ap`, `ipv4.method shared`, IPv6 disabled):
  - **WPA2 with a random password** shown on the console (made by the TUI
    as for random passwords, ADR 0015, given to the helper on stdin, kept
    by NetworkManager root 0600). It keeps passers-by off the radio
    and encrypts the link. The network is still treated as open (§1):
    anyone told the password is on it.
  - **No routing to other networks.** NetworkManager's shared mode NATs
    the hotspot to the default route; our firewall drops every packet
    forwarded from the hotspot interface (a `FORWARD` rule in the
    `resilience-share` chain set). A hotspot never gives anyone Internet
    access through this computer.
  - Like the mesh, the Wi-Fi card cannot be on another Wi-Fi network
    meanwhile; Ethernet keeps working. Not kept across reboots.

### 7. Expert option: names in the hotspot's DNS

For experienced users, and for devices that are not Resilience machines
(phones, other laptops), **the hotspot's own DNS** (the dnsmasq that
NetworkManager already runs for it) answers names in **`home.arpa`**
(RFC 8375, reserved for home networks):

- `resilience.home.arpa` → the hotspot's address, always, so a phone on
  the hotspot can open `http://resilience.home.arpa/library/`;
- more records declared in the configuration
  (`resilience.share.hotspot.dnsRecords`, names checked at build time),
  written to `/etc/NetworkManager/dnsmasq-shared.d/resilience.conf`
  together with `local=/home.arpa/` (never forwarded upstream).

It is an option in the configuration, behind a clearly "advanced"
handbook section, not a screen in the TUI. No DNS server is started on
networks Resilience does not run: on someone else's network nobody would
ask it, and it would be one more service parsing hostile packets.

### 8. In the interface

Communications → **Community network**, an assistant with three choices:

- **Join a network**: networks nearby (Wi-Fi from the existing list, the
  mesh), then the machines heard on the current one with what they
  share; Enter opens the library or pages, `p` lists packs to get
  (ticked as in Resource packs), `f` lists the shared folder, `a` types an
  address.
- **Create a network**: the mesh or a hotspot, with what each means
  (range, the card leaves its Wi-Fi network, the hotspot's password).
- **Share something on this network**: the network in use, ticks for the
  four services with what each exposes (for the folder, its file count
  and size), the name, the warning of §1, then the administrator's
  password. While on, the same screen shows what is served, on which
  network, how many connections are open now (from nginx's status on
  `127.0.0.1` only) and a Stop button.

Shown only where `resilience.share.enable` is on (default: on in the
standard edition, off in minimal, mkDefault).

## Alternatives considered

- **Serving over Reticulum** (as NomadNet serves pages): no new port and
  end-to-end authentication. But it is a protocol and page format of
  its own that no other device reads, it is far too slow for multi-GiB
  packs, and kiwix would need a translator. Reticulum stays for messages
  and discovery.
- **mDNS/DNS-SD with avahi or systemd-resolved** for discovery: the
  standard, and phones would see the services. It needs a resident daemon
  that answers anyone on UDP 5353, or replaces the DNS setup of every
  machine (resolved), and its names are as unauthenticated as an
  announce, without the signed identity. The hotspot's DNS (§7) covers
  phones where it matters: on a network Resilience runs. To revisit if
  people need phones to find Resilience machines on networks they do not
  run.
- **kiwix-serve directly on the network**: one process fewer, but no
  rate limits beyond connections per address, and its URL and search
  code would face every request. nginx in front costs little.
- **One server per service** (kiwix-serve, darkhttpd for files): more
  ports, more processes, more firewall rules.
- **Sharing allowed to the person at the console without a password**
  (like the mesh): sharing exposes files to strangers and runs servers
  for everyone in range. It is the decision of the machine's
  administrator. The live system can join, not share.
- **Remembering sharing across reboots**: a laptop would start serving on
  whatever network it meets next. `autoStart` covers the shelter server,
  as a decision written in the configuration.
- **An open hotspot**: easier to join, but everyone in range would be on
  it, and the link would be readable. A password on the screen costs one
  line.
- **HTTPS between machines**: certificates would be self-signed and
  unverifiable on first contact (no CA offline), so it would protect
  nothing against the attacker who matters here (someone on the same
  open network who answers first). Integrity comes from the catalog
  hashes. Confidentiality is not a goal: the content is public.
- **Uploads into the shared folder**: deferred, as the roadmap says, to
  their own review (disk filling, hostile content, names).

## Consequences

- A Resilience machine can run servers for strangers. That is what this
  is for, but it is a new kind of exposure: hostile requests are parsed
  by nginx and, through it, by kiwix-serve (for library pages and
  search), on one interface and only while an administrator wants it.
  THREAT_MODEL.md gets a section on serving to the local network
  (T46 to T50).
- Without messaging (minimal), machines are found only by typing an
  address.
- On a big shared LAN (a café, a campus) anyone on it can read what is
  shared. The interface names the network before sharing.
- Packs from a peer are as trusted as packs from Kiwix: exactly as
  trusted as the catalog. A peer can share only packs that are in the
  receiver's catalog. Different releases have different catalogs, so
  some packs may not be accepted between them.
- The shared folder is served as is: an administrator who puts a private
  file there and makes it world-readable publishes it. The listing on the
  Share screen is the guard.
- Bandwidth: one machine serving several large downloads at once is
  slow, and limits per address keep one person from taking all of it.
  Over the mesh (tens of Mbit/s at best) a multi-GiB pack takes hours;
  the handbook says so and recommends a stick for big packs.
- Ports and services: TCP 80 on one interface while sharing; the
  hotspot's DHCP (UDP 67) and DNS (UDP/TCP 53) on the hotspot only while
  it is on. Reticulum's two UDP ports as before.

## Slices

1. This ADR and the threat model section (no code).
2. Server side: `nix/modules/share.nix` (options, nginx and kiwix-serve
   units, firewall chain), `set-share` in the helper (pure rules in
   `resilience-core/src/admin.rs`, polkit action, audit),
   `vm-share` part 1 (on/off, only the chosen interface, port closed
   after stop, only GET/HEAD, private files not served, a
   non-administrator refused).
3. Discovery: `resilience-share-announce`, shares recorded by the
   per-user program, `resilience_core::share` parsers, `comms-messages`
   extended.
4. Joining: `--from-peer` and `open-remote` with the relay in
   `resilience-library`, the local-address rules, the TUI's Community
   network screens, `vm-share` part 2 (two machines: a pack installed
   from a peer, a tampered pack refused, the library read through the
   relay).
5. Shared folder and pages (TUI listing, download into ~/Received).
6. Hotspot (`set-hotspot`, no forwarding, WPA2 password) and the expert
   DNS, `vm-share` part 3 (a hotspot with `mac80211_hwsim`, a client
   resolves `resilience.home.arpa`, no forwarding to another network).
7. Handbook page, docs, a review of the server configuration.
