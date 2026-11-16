# ADR 0018: Local messaging with Reticulum and LXMF

Status: accepted (2026-11-16). The network service, the per-user
messaging program and the interface (TUI) are implemented.

## Context

The specification (§14 B, §15) asks for messaging between nearby
Resilience machines without the Internet: discovery on the local network,
messages kept when the network disappears, contact identification,
integrity, and delivery states that never claim more than what happened
(queued, sent, received, delivered, failed). It asks to use an
established protocol and not to invent one.

Candidates in the pinned nixpkgs (checked 2026-11-16):

| Software | Version | Licence | Fit |
|---|---|---|---|
| Reticulum (`rns`) + LXMF (`lxmf`) | 1.2.9, 0.9.8 | **Reticulum License** (nixpkgs: unfree) | network stack for local links, radio and TCP with its own addressing and encryption; LXMF adds signed messages, delivery proofs and store-and-forward nodes |
| NomadNet (`python3Packages.nomadnet`) | 1.1.0 | GPL-3.0 | a terminal messenger for LXMF; also a page browser and node server |
| Tox (`toxic`) | 0.16.1 | GPL-3.0 | finds peers on a LAN, but both ends must be online: no store-and-forward |
| Yggdrasil | 0.5.13 | LGPL-3.0 | an IPv6 overlay network, not a messenger |
| BATMAN-adv (`batctl`) | 2026.4 | GPL-2.0 | layer-2 mesh, not a messenger |
| SimpleX Chat | absent | | |

**Licence.** Since 1.0, Reticulum and LXMF are under the Reticulum
License: the MIT text plus two conditions. The software may not be used
"in any kind of system which includes amongst its functions the ability
to purposefully do harm to human beings", nor "in the creation of an
artificial intelligence, machine learning or language model training
dataset". This is not a free software licence (it restricts fields of
use), and nixpkgs marks both packages unfree. The project owner decided
on 2026-11-16 to ship them **as an opt-in module**: a system without the
module contains none of this code. (Resilience's own code has since been
put under the Hippocratic License 2.1, which also restricts uses; the two
licences are different texts with different conditions, and both apply to
a system that has the module.)

**Experiments** (dev host, throwaway scripts, 2026-11-16, rns 1.2.9 and
lxmf 0.9.8 from the pinned nixpkgs):

- Two Reticulum instances joined by a local TCP link, one LXMF program on
  each: a message is delivered and its delivery proof is back in 1.5 s
  (direct link and single-packet "opportunistic" delivery alike). The
  sender's state goes `outbound` → `delivered`.
- Recipient absent: the state stays `outbound` and becomes `failed` after
  64 s. It is never `delivered`.
- A message from a sender whose announce was never heard arrives with
  `signature_validated = False` (the public key is unknown). Once the
  sender has announced, it is `True`. The interface must show the
  difference.
- **One shared instance, several programs**: one `rnsd` and two programs
  with their own configuration directories, attached through the
  instance's local socket (`@rns/NAME`, an abstract Unix socket). With
  different RPC keys the programs fail ("digest sent was rejected") and
  no message is delivered. With the same `rpc_key` in every
  configuration, messages are delivered both ways in about 1 s with valid
  signatures.
- `rnsd` uses 38 MiB of memory (RSS) when idle. The Python environment
  with `rns` and `lxmf` adds about 240 MiB to a system that already has
  Python (370 MiB closure in all, *measured with NomadNet included*).

## Decision

1. **Reticulum with LXMF is the local messaging stack.** No protocol or
   cryptography of our own (CLAUDE.md §9).
2. **Opt-in**: `resilience.comms.enable`, false by default. Only `rns`
   and `lxmf` are allowed as unfree, in the one package that holds them
   (`nix/packages/resilience-reticulum`), which imports nixpkgs again
   with a predicate for these two names. Modules never set
   `nixpkgs.config` (it does nothing with read-only `pkgs`). The licence
   is stated in docs/COMPONENTS.md, in the handbook and in the option's
   description.
3. **One Reticulum instance per machine**, as a system service
   (`resilience-reticulum`: `rnsd` under a dynamic unprivileged user,
   hardened). Two programs cannot both own the local-network ports, and
   several accounts can be logged in at once.
   - Interface: `AutoInterface` only (IPv6 link-local multicast on the
     local network: discovery UDP 29716, data UDP 42671, opened in the
     firewall when the module is on). No TCP, no Internet hubs, no radio
     in this slice.
   - `enable_transport = No`: the machine does not route for others.
   - Remote management, `rnsh` and `rnx` (remote shells and commands) are
     not configured, and the environment's programs are not in `PATH`.
4. **Programs of users attach to the instance** with a client
   configuration written by the service at
   `/run/resilience-reticulum/client/config` (instance name and RPC key).
   The key is made at the first start and kept in the service's state
   directory; it is **readable by every local account** on purpose: all
   accounts of a machine may use its messaging network. With it a local
   user can also disturb the instance (drop paths). That is a
   denial of service between people sharing a computer, on messaging
   only; it gives no access to anyone's messages, which are encrypted to
   each identity. The key is never in the Nix store.
5. **Each account has its own identity and messages** (next slice):
   - a per-user program `resilience-messages` (Python, because LXMF is a
     Python library; it is glue around `LXMF.LXMRouter`, with no parsing
     or cryptography of its own) runs while the user is logged in;
   - identity, contacts and messages in
     `~/.local/share/resilience/messages`, mode 0700;
   - the TUI and the program exchange **files** (an outbox directory, one
     JSON file per message with its state), so the Rust side parses
     simple files with a bounded size and never talks to the network;
   - states shown: **Queued** (no path to the recipient yet; kept and
     retried when the recipient is heard again), **Sent** (handed to the
     network, no proof yet), **Delivered** (the recipient's program
     returned LXMF's delivery proof), **Failed** (given up, with the
     reason), **Received** (incoming, with "sender verified" or not);
   - contacts are LXMF addresses (the hash of a public key). Names heard
     on the network are claims: the interface says so until the user has
     compared the address with the person.
6. **Later, each with its own decision**: a propagation node (LXMF's
   store-and-forward for recipients that are off when the sender is on),
   file exchange, radio interfaces (RNode/LoRa), links over the Internet,
   Yggdrasil and BATMAN-adv profiles, Tox and signal-cli as Internet
   messengers.

## Decided while writing the messaging program (2026-11-16)

- **Files** (all under `~/.local/share/resilience/messages`, 0700):
  `outbox/ID.json` (`{"to", "text"}`, written with a temporary name and
  renamed) and `cancel/ID` are written by the interface; `messages/ID.json`
  (one record per message, with `state`, `detail`, `verified`),
  `peers.json`, `address` and `status.json` are written by the program
  only. Text is at most 8 KiB; a request file at most 64 KiB.
- **Queued means kept and retried**: after 30 s, 1, 5 and 15 minutes,
  then every hour, and at once when the recipient is heard on the
  network. A message is given up (`failed`) after 7 days, when cancelled,
  or when the request is invalid. LXMF's own "failed" after its attempts
  only puts the message back in the queue.
- **Signatures**: a recipient that has never heard the sender cannot
  check the signature on arrival (seen between two machines in
  `vm-comms`). The sender announces itself before sending (when its last
  announce is older than a minute, or when a waiting recipient appears),
  and the recipient asks the network for an unknown sender and checks the
  kept message again when the key arrives (for 10 minutes). `verified`
  only ever goes from false to true.
- **It runs as a systemd user service** while the account is logged in,
  never as root, with the same memory limit as the instance.
- **Names are claims**: `peers.json` records the name each address
  announces, cleaned to one printable line of 64 bytes.

## Decided while writing the interface (2026-11-16)

- **Shown only where messaging exists**: the Communications section has
  "Local messages" when `/run/resilience-reticulum/client/config` exists.
- **Words on the screen**: `waiting` (queued), `sent`, `delivered`,
  `FAILED`, and for incoming messages `from them` or `from them, NOT
  VERIFIED`. Saving a message says "Message saved", never "sent".
- **People**: an address the user named shows that name; otherwise the
  name the address announces, in quotes with a question mark
  (`"marie"?`); otherwise the first 8 characters of the address. The
  user's names are in `contacts.json`, written by the interface.
- **Addresses are typed as read aloud**: groups, spaces, dashes and upper
  case are accepted.
- **The interface never talks to the network or to the program**: it
  reads the store every 2 seconds while a messages screen is shown and
  writes `outbox/`, `cancel/` and `contacts.json`. Texts and names from
  the network are cleaned before drawing (control characters, escape
  sequences, direction overrides).
- **One-line messages** of at most 1,000 characters for now.

## Editions (owner's decision, 2026-11-16)

Local messaging is **on by default in the standard edition**
(`nix/profiles/standard.nix`, with `mkDefault`, so one line turns it
off) and absent from the minimal edition. A standard system therefore
contains Reticulum and LXMF under their licence, and announces itself on
its local network.

## Alternatives considered

- **NomadNet as the interface**: maintained and complete, but it is also
  a page browser with its own markup, a node server and an editor
  launcher. Restricting it for Ultra-Stable accounts would mean patching
  it. Its state names are not the specification's. Kept as a possible
  optional application for Stable and Advanced accounts.
- **Free software only (XMPP on the LAN with prosody and profanity, or
  Tox)**: XMPP needs a server that both people can reach, so the
  "server" machine must be on; Tox has no store-and-forward. Neither has
  a path to radio links. The owner chose Reticulum.
- **The last MIT-licensed Reticulum (0.9.x)**: unmaintained.
- **Each user program with its own Reticulum instance**: two instances
  cannot share the AutoInterface ports on one machine.
- **A system-wide messaging daemon holding everyone's identities**: one
  process would hold every account's private keys.

## Consequences

- With the module on, the machine sends multicast packets on its local
  network (discovery every few seconds). This is local traffic, never to
  the Internet, and exists only when the module is enabled; it is the
  one exception to "no background network traffic" and is documented in
  docs/OFFLINE_OPERATIONS.md.
- A system with the module carries the Reticulum License's conditions in
  addition to those of Resilience's own licence. A licence audit
  (Phase 9) must list `rns` and `lxmf` apart.
- Python code enters the project (the per-user program). It is kept
  small and tested with the real libraries.
- Not tested yet: more than two machines, Wi-Fi with client isolation
  (multicast is often blocked there), links that drop mid-message, large
  messages, hostile peers.

Tested by: `vm-comms` (two machines on an isolated network: the service
starts, each sees the other, a signed LXMF message is delivered with its
proof through the shared instances, then through each account's
program, including a recipient that is away and comes back, then through
the interface on both consoles) and `comms-messages` (the program with
the real libraries, no VM).
