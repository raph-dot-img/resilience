"""Test program for vm-comms (ADR 0018): one LXMF identity attached to the
machine's shared Reticulum instance.

  comms-node.py NAME DIR listen SECONDS
  comms-node.py NAME DIR send PEER_ADDRESS_HEX SECONDS

DIR holds a copy of /run/resilience-reticulum/client/config. It prints its
address, every message received and, when sending, each state change.
"""
import os
import sys
import time

import LXMF
import RNS

name, cfg, mode = sys.argv[1], sys.argv[2], sys.argv[3]
RNS.Reticulum(configdir=cfg)
idp = os.path.join(cfg, "id")
ident = RNS.Identity.from_file(idp) if os.path.exists(idp) else RNS.Identity()
ident.to_file(idp)
router = LXMF.LXMRouter(identity=ident, storagepath=os.path.join(cfg, "lxmf"))
dest = router.register_delivery_identity(ident, display_name=name)


def received(m):
    print(
        f"[{name}] RECEIVED from={m.source_hash.hex()} verified={m.signature_validated} "
        f"content={m.content_as_string()!r}",
        flush=True,
    )


router.register_delivery_callback(received)
with open(os.path.join(cfg, "addr"), "w") as f:
    f.write(dest.hash.hex())
print(f"[{name}] address {dest.hash.hex()}", flush=True)
router.announce(dest.hash)

STATES = {
    LXMF.LXMessage.GENERATING: "generating",
    LXMF.LXMessage.OUTBOUND: "outbound",
    LXMF.LXMessage.SENDING: "sending",
    LXMF.LXMessage.SENT: "sent",
    LXMF.LXMessage.DELIVERED: "delivered",
    LXMF.LXMessage.FAILED: "failed",
}

if mode == "listen":
    end = time.time() + float(sys.argv[4])
    while time.time() < end:
        time.sleep(5)
        router.announce(dest.hash)
else:
    peer = bytes.fromhex(sys.argv[4])
    limit = float(sys.argv[5])
    t0 = time.time()
    # Let our announce reach the peer, so that it can check our signature.
    time.sleep(5)
    while RNS.Identity.recall(peer) is None and time.time() - t0 < limit:
        if not RNS.Transport.has_path(peer):
            RNS.Transport.request_path(peer)
        time.sleep(0.5)
    known = RNS.Identity.recall(peer)
    print(f"[{name}] peer known={known is not None} after {time.time() - t0:.1f}s", flush=True)
    if known is None:
        sys.exit(3)
    out = RNS.Destination(known, RNS.Destination.OUT, RNS.Destination.SINGLE, "lxmf", "delivery")
    m = LXMF.LXMessage(out, dest, f"hello from {name}", desired_method=LXMF.LXMessage.DIRECT)
    router.handle_outbound(m)
    last = None
    while time.time() - t0 < limit:
        if m.state != last:
            last = m.state
            print(f"[{name}] state {STATES.get(m.state, m.state)} after {time.time() - t0:.1f}s", flush=True)
        if m.state in (LXMF.LXMessage.DELIVERED, LXMF.LXMessage.FAILED):
            break
        time.sleep(0.1)
    ok = m.state == LXMF.LXMessage.DELIVERED
    print(f"[{name}] RESULT {'delivered' if ok else 'not delivered'}", flush=True)
    # Not sys.exit: Reticulum's own exit handlers may replace the status.
    os._exit(0 if ok else 4)
