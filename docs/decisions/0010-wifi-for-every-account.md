# ADR 0010: Wi-Fi for every account, and NetworkManager's polkit defaults

Status: accepted (2026-11-14).

## Context

Joining a Wi-Fi network is an everyday task. Until now only Stable and
Advanced accounts could do it, with `nmtui`. Ultra-Stable accounts had no
way to join from the interface.

Exploring this in a VM showed a problem in what we had **claimed**. Upstream
NetworkManager's polkit defaults give a lot to *any active local session*.
No group and no password are needed for:

- `enable-disable-network`: switch all networking off;
- `enable-disable-wifi`, `-wwan`, `-wimax`, `-statistics`;
- `enable-disable-connectivity-check`: would start background traffic,
  against ADR 0004;
- `wifi.share.open` and `wifi.share.protected`: hotspots;
- `network-control`, `wifi.scan`, `settings.modify.own`.

Ultra-Stable accounts are active local sessions on tty1. `vm-modes` had
recorded "Ultra-Stable cannot change networking" as PASS, but it ran
`nmcli networking off` through `runuser`, which has no session. polkit then
answers "no" for reasons that have nothing to do with the mode. From the
real console, the same command succeeded (`nmcli networking off`, exit 0, in
the exploration VM).

Without the group, some actions already needed an administrator's password
(`auth_admin_keep`). Examples are `settings.modify.system` and setting the
hostname, and NetworkManager also refused to create a hotspot.

## Decision

### Policy: an allowlist for accounts without the `networkmanager` group

A polkit rule (`nix/modules/networking.nix`) applies to accounts in neither
`networkmanager` nor `wheel`, which in practice means Ultra-Stable accounts.
It returns **NO** for every `org.freedesktop.NetworkManager.*` action except
these four:

| Action | Why it is kept |
|---|---|
| `wifi.scan` | list networks |
| `network-control` | connect and disconnect |
| `settings.modify.own` | profiles restricted to the account (`connection.permissions user:NAME`) |
| `enable-disable-wifi` | switch the Wi-Fi radio back on |

The rule only removes rights and never grants any. Actions added in future
NetworkManager versions are denied by default. Stable and Advanced accounts
keep NixOS's existing rule for the `networkmanager` group (everything
allowed), unchanged. A security invariant checks that the rule is present
whenever NetworkManager is enabled. `vm-wifi` checks every listed action
with `pkcheck` against the TUI's own process on tty1, and checks that a
Stable account keeps its rights.

### Interface: "Wi-Fi networks", as the user, through nmcli

A Networking entry, available in every mode. Nothing in it is privileged.

- **List:** `nmcli -t … device wifi list --rescan no`, parsed by pure
  functions (`resilience-core/src/wifi.rs`). The list has one row per
  network name (the strongest access point), with signal, security and
  whether the network is saved. A scan is requested in the background when
  the screen opens and on `r`. The list refreshes every 5 seconds.
- **Join a new network:**
  1. The TUI's own dialog asks for the password and shows it as stars. It is
     checked: 8–63 printable ASCII characters, or 64 hex digits.
  2. A profile is created for this account only, **without a secret**:
     `connection.permissions user:NAME`, `psk-flags 0`, and a random UUID so
     it can be addressed exactly.
  3. `nmcli --wait 45 connection up uuid U passwd-file /dev/stdin` runs in a
     background thread. The password reaches nmcli **through a pipe**, never
     through argv, the environment, a file or a log. Our copies are
     overwritten after use (best effort).
  4. NetworkManager stores the password in its root-only profile file
     (`/etc/NetworkManager/system-connections`, root 0600).
  5. If the attempt fails, the new profile is deleted: nothing is saved.
- **Saved networks** connect without asking. If the saved password is
  refused, the password dialog opens for that profile.
- **Open networks** first show a warning, with Cancel as the default.
  **WEP** and **enterprise (802.1X)** networks are shown but not offered.
- **Forget** asks for confirmation, then deletes the profile. Profiles made
  by an administrator cannot be deleted (polkit refuses), and the screen
  explains that.

### Why not `nmcli --ask`

`nmcli --ask` was the first design: nmcli prompts for the password on the
terminal. vm-wifi showed that after a wrong password it asks again, with the
old password prefilled. Ctrl-C only redraws the prompt, so a user who does
not know the password cannot get out. The TUI dialog plus `passwd-file` on
stdin avoids this.

With a wrong password, NetworkManager retries, and nmcli's password file
answers each request with the same password. The attempt therefore ends as
"Timeout expired (45 seconds)". The TUI says the password is probably wrong
or the network is out of range, because the two cannot be told apart. After
nmcli exits, NetworkManager stops (tested).

## Alternatives considered

- **Give Ultra-Stable accounts the `networkmanager` group:** they would get
  everything, including system-wide profiles, connectivity checks and
  hotspots.
- **A privileged helper operation (`wifi-connect`) with an administrator's
  password:** that is too much friction for a daily task. Our root code
  would also have to handle network passwords.
- **Deny everything to Ultra-Stable (no Wi-Fi):** safe, but it fails the
  "useful everyday operations" requirement.
- **Talk D-Bus to NetworkManager directly:** more code than nmcli, and no
  security gain.

## Consequences

- Ultra-Stable accounts can join, leave and forget their own Wi-Fi
  networks. They can no longer switch networking off, share a hotspot or
  turn on connectivity checks, which they could before this change.
- **Remaining rights.** `network-control` lets them disconnect *any*
  connection, including Ethernet set up by an administrator.
  `settings.modify.own` lets them create profiles with any settings that
  apply to their own connections, for example a static address or a cloned
  MAC address. The TUI only creates plain Wi-Fi profiles, but an account
  with a way to run nmcli could do more. We accept this because it only
  affects networking, the user is at the console, and the same is true on
  most desktop systems. It is listed as a known gap.
- **Not supported yet:** hidden networks, enterprise (802.1X), WEP,
  passwords with leading or trailing spaces, captive portals, and choosing
  between several Wi-Fi adapters.
- On the console, the interface owns the screen while connecting. Kernel
  messages are not shown (`loglevel=3` on the ISO).

Tested by: `tests/vm/wifi.nix` (mac80211_hwsim + hostapd: policy, Esc, a
wrong password, the right password, private profile, secret absent from the
journal, process list, home folder and other accounts, reconnecting without
a password, forgetting), unit tests in `wifi.rs` (parsing, argv,
password only on stdin), TUI tests, and `security-invariants`.
