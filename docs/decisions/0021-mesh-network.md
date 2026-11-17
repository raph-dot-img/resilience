# ADR 0021: A Wi-Fi mesh for local messaging without a router (BATMAN-adv)

Status: accepted (2026-11-17).

## Context

Local messaging (ADR 0018 to 0020) needs the computers to share a local
network: an access point, a router or a cable. After a disaster there is
often none: no power for the box, or no box. Laptops all have Wi-Fi
cards, which can talk to each other directly, and a computer in the
middle can relay between two that are out of each other's range.

The roadmap listed two candidates, Yggdrasil and BATMAN-adv. Checked on
2026-11-17 (pinned nixpkgs, kernel 6.18.63, the development host):

- **BATMAN-adv** is in the kernel (`CONFIG_BATMAN_ADV=m`, BATMAN IV and
  V); `batctl` 2026.4 is in nixpkgs. It makes the computers of a mesh one
  Ethernet segment (`bat0`), each relaying frames for the others. To
  Reticulum's AutoInterface, which uses IPv6 link-local multicast,
  computers two or three hops away are then on the same link: local
  messaging works across the mesh unchanged, without Reticulum transport.
- The link between Wi-Fi cards: **802.11s** ("mesh point") would be the
  modern choice, but the host's Intel Wi-Fi 6 AX200 (`iwlwifi`), like
  most laptop cards, does not offer it (`iw list`: IBSS, managed, AP,
  monitor, P2P only). **IBSS** (ad hoc) is offered by `iwlwifi` and by
  nearly every other driver. `mac80211_hwsim` supports it for tests.
- Reticulum's AutoInterface lists the network interfaces when it starts
  (RNS 1.2.9, `AutoInterface.__init__`); an interface that appears later
  is used only after a restart of the instance.
- **Yggdrasil** 0.5.13 (a NixOS module exists) is an encrypted IPv6
  overlay that routes between its nodes over any links, including
  across the Internet through public peers. It needs an existing link
  between neighbours (it does not make one), and nothing in Resilience
  uses IPv6 addresses beyond the link: Reticulum already routes and
  encrypts on its own.

## Decision

1. **The mesh is BATMAN-adv (BATMAN IV) over Wi-Fi ad hoc (IBSS).** All
   Resilience machines use the same cell so that they find each other
   with no setting: name `resilience-mesh`, 2412 MHz (channel 1, allowed
   in every regulatory domain), fixed cell id `02:52:45:53:4d:31` (so
   that cells formed separately merge). Options under `resilience.mesh`:
   `enable`, `name`, `frequency`, `cellId`, `devices` (empty: the first
   Wi-Fi card; Ethernet cards can be listed to join a mesh by cable),
   `autoStart`.
2. **Available wherever local messaging is** (`resilience.mesh.enable`
   defaults to `resilience.comms.enable`: on in the standard edition, off
   in minimal), but **off until someone turns it on**, unless `autoStart`
   (for a relay computer left on in a shelter). Not remembered across
   reboots otherwise.
3. **Turning it on and off**: a root service `resilience-mesh.service`
   (oneshot, kept active) does the work; the privileged helper gains
   `set-mesh on|off` (ADR 0007), which only starts or stops that unit,
   audited. Its polkit action `org.resilience.admin.set-mesh` is allowed
   **without a password to the active local session**, like joining a
   Wi-Fi network (ADR 0010) and like `set-keymap` (ADR 0017): on the live
   system there is no administrator, and the mesh is for exactly then.
4. **What the service does**: the card leaves NetworkManager (`managed
   no`), becomes an IBSS station in the cell and the only member of
   `bat0`; `bat0` gets **only its IPv6 link-local address**: router
   advertisements and address autoconfiguration are off on it, there is
   no IPv4, no DHCP, no default route and no forwarding to other
   interfaces. NetworkManager never manages `bat0`. Stopping undoes all
   of it and gives the card back to NetworkManager. Both restart the
   Reticulum instance, so that it uses (or stops using) `bat0`; the
   store and the accounts' programs reattach by themselves.
5. **No Wi-Fi encryption.** IBSS encryption (RSN in IBSS) is not
   supported by every driver, and a shared password would have to be
   known to everyone anyway. What travels on the mesh is Reticulum
   traffic, which is encrypted and signed end to end (ADR 0018). The
   firewall stays as it is: only Reticulum's two UDP ports are open.
6. **Yggdrasil is not adopted** (see below).

## Alternatives considered

- **802.11s with BATMAN-adv or its own routing (HWMP)**: better suited,
  but not available on the Intel cards of most laptops; a mesh that half
  of the computers cannot join does not help.
- **Reticulum transport nodes over IBSS without BATMAN-adv**: Reticulum
  can route itself, but every relay would have to be configured as a
  transport node, and AutoInterface still needs a shared link per hop.
  BATMAN-adv makes every computer a relay with no configuration.
- **Yggdrasil**: an overlay for IPv6 programs, which we do not have;
  it does not create the radio link that is missing; its main strength
  is peering across the Internet, against the offline principle; it
  would be one more resident, network-facing service. To revisit if
  Resilience gains IPv6 programs that need routing between networks.
- **Babel or OLSR (IP routing)**: need IP addresses to be assigned to
  every machine without a server; BATMAN-adv works below IP and needs
  none.
- **A password-protected mesh**: see 5; it would also leave people who
  do not know the password unable to reach anyone.
- **Turning the mesh on with an administrator's password**: impossible
  on the live system, which has no administrator.

## Consequences

- While the mesh is on, the Wi-Fi card cannot join an ordinary Wi-Fi
  network (one card, one mode); Ethernet keeps working. The interface
  says so before turning it on.
- A computer in the mesh relays other people's (encrypted) traffic and
  sends BATMAN-adv messages about every second: more radio use and
  battery drain while it is on.
- Anyone in radio range can join the mesh and see that the computer is
  there (its Reticulum announces, its MAC address). Hostile frames are
  parsed by the kernel's `batman_adv` and `mac80211` code.
- IBSS speeds are modest (tens of Mbit/s at best, less over several
  hops): enough for messages and files of up to 900 KB (ADR 0020).
- Real hardware is untested: `scripts/hw-report.sh` now records whether
  the Wi-Fi card offers IBSS.

Tested by: `vm-mesh` (three machines, the middle one relaying by cable
between two networks that do not see each other: a message goes from the
first to the third only while the mesh is on; `bat0` has no global
address, no default route and ignores router advertisements; the helper
and its polkit action; a fourth machine with two simulated Wi-Fi radios
forms an IBSS cell and a BATMAN-adv neighbourship, then gives the card
back to NetworkManager).
