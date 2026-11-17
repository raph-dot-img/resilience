# ADR 0019: Store-and-forward for local messages (LXMF propagation nodes)

Status: accepted (2026-11-17).

## Context

With ADR 0018, a message waits on the sender's computer and is delivered
only when both computers are on, on the same network, at the same time
within 7 days. In a shelter or a neighbourhood that is the normal case
to fail: laptops are closed, accounts are logged out, batteries are
saved. LXMF, which we already use, has a store-and-forward mechanism:
**propagation nodes** keep messages, encrypted for their recipients,
until the recipients' programs collect them, and nodes that hear each
other exchange what they hold.

Read in the pinned LXMF (0.9.8) and measured on the development host
(Ryzen 5 PRO 4650U), 2026-11-17:

- A node needs no address of its own (`LXMRouter.enable_propagation`
  without `register_delivery_identity`). Upstream's daemon `lxmd`
  *does* register one and writes every message sent to it into a
  directory with no size limit; anyone on the network could fill the
  disk that way.
- Senders must attach a proof of work ("stamp") whose cost the node
  announces. LXMF's default is 16, its minimum 13: about 1 s and 0.2 s
  per message here (all cores), several times that on a slow dual-core.
  Checking a stamp costs a node about 40 ms.
- Nodes peer automatically with the nodes whose announces they hear
  (`autopeer`, within a number of hops) and synchronise about every 24 s.
  A peering costs a one-time proof of work (18, well under a second here).
- A client collects with `request_messages_from_propagation_node`; the
  node deletes what the client reports having.
- The LXMF hash of a message covers its timestamp, not the delivery
  method: two copies built with the same timestamp have the same hash,
  and the recipient keeps one.
- Found on the way: a Reticulum program configured as a client of a
  shared instance that starts **before** the instance becomes the shared
  instance itself and takes its socket name, unless it passes
  `require_shared_instance=True`. The per-user program of ADR 0018 had
  this race (autologin on tty1 can start before `rnsd`).
- Reticulum's clients never save the public keys they hear
  (`known_destinations`) themselves, and our program exits with
  `os._exit`: after a restart, a message from someone not heard again
  could not be checked.

## Decision

1. **Every machine with local messaging runs a message store by
   default**: `resilience.comms.store.enable` (default true, so on in the
   standard edition), `resilience.comms.store.limitMB` (default 32).
   Store-and-forward is only useful if some computer that stays on has
   one, and the live system has no administrator to turn it on later.
   Each store holds what passes on its network, so any machine that is
   on can hand a message over.
2. **The store is our program, not `lxmd`**: `resilience-messages node`,
   a few lines around `LXMRouter` with propagation enabled and **no
   delivery address** (it never receives messages for itself), a size
   limit, LXMF's **minimum stamp cost (13)** so slow computers can leave
   messages, and peering only with stores **one hop away** (the local
   network). Service `resilience-messages-node`: dynamic unprivileged
   user, the same hardening as the instance, **AF_UNIX only** (it talks
   only to the machine's instance), state 0700, its address in
   `/run/resilience-messages-node/address` (0644) for the accounts'
   programs. Started, stopped and restarted with the instance.
3. **The accounts' programs** (`resilience-messages run`):
   - use the machine's own store when it has one, otherwise the nearest
     store heard in the last hour (fewest hops, then most recent),
     ignoring stores that announce a stamp cost above 16;
   - try **direct delivery first**, as before. A message still not
     delivered after **2 minutes** is also handed to the store (once;
     again after 5 minutes if that failed). Direct attempts go on: a
     delivery proof can still come, and both copies have the **same
     hash** (the record keeps LXMF's timestamp, `ts`), so the recipient
     keeps one;
   - state: `sent` with `"stored": true` once the store has accepted the
     copy ("sent, kept on the network" on screen). It becomes
     `delivered` only with a delivery proof, which only direct delivery
     gives. After 7 days, or when cancelled, a stored message is `failed`
     but shown as "stopped, may still arrive": the copy cannot be called
     back;
   - collect waiting messages from the store 20 s after starting, then
     every 5 minutes;
   - attach with `require_shared_instance=True` (exit 75 and a restart
     by systemd when the instance is not up);
   - save the public keys heard every 10 minutes and when stopping, and
     check an unverified message again for up to 7 days (was 10
     minutes), since a message from a store usually comes from someone
     who is away.
4. **The interface** shows whether a store is in use ("Messages for
   people who are away wait in a message store on this network…" or "No
   message store on this network: a message arrives only when both
   computers are on at the same time."), from `"store"` in
   `status.json`.

## Alternatives considered

- **`lxmd`**: upstream's daemon, but it registers a delivery address
  whose messages are written to disk without a limit, and it can run a
  command on every message (`on_inbound`). Our node is shorter than its
  configuration file.
- **A store only on machines an administrator chooses** (off by
  default): cheaper, but nobody can turn it on on the live system, and a
  network without one has no store-and-forward at all.
- **Hand every message to the store at once**: a recipient that is there
  would get it later (at its next collection) and the sender would never
  see "delivered" for messages that could have been delivered directly.
- **Stop direct attempts once a copy is stored**: simpler, but the
  sender would never get a delivery proof for those messages.
- **The default stamp cost (16)**: protects the store better against
  floods, but a slow computer would spend many seconds per message. The
  store is bounded by its size limit in any case (LXMF drops the oldest
  and lowest-value messages first).

## Consequences

- About 40 MB more memory per machine (measured on the live ISO with
  messaging on: 330 MiB used after boot at 1 GiB, ADR 0018's
  measurements included), up to 32 MB of disk for messages of others,
  and more local traffic: stores announce themselves every 10 minutes
  and synchronise while they have something to exchange.
- Metadata: a store learns which addresses receive messages and when
  (not who sent them, nor the text, which are encrypted for the
  recipient). On the live system the store is in memory and disappears
  at shutdown.
- A hostile machine on the network can fill the stores up to their
  limit (pushing older messages out) or announce a store that accepts
  and drops messages; a stored message is never shown as delivered, so
  the sender is not misled about it.
- A message reaches a person who is away only if some computer that
  holds a copy is on, on the same network, when that person's computer
  collects. Two computers alone, one of them off, still cannot exchange
  messages.

Tested by: `comms-messages` (real libraries, no VM: a store; Bob's
program stopped; Ann's message is `sent` and stored after 2 minutes;
Ann stops; Bob collects it once, verified) and `vm-comms` (two machines:
the store is unprivileged with no network socket; an account logged out
on beta; alpha's message is stored, copied to beta's store, shown as
"sent, kept on the network"; alpha is switched off; beta's account
collects it, verified, shown "from them").
