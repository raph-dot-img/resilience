"""resilience-messages: the per-user local messaging program (ADR 0018 §5).

It is glue around LXMF (`LXMF.LXMRouter`) and Reticulum, which do all the
networking, cryptography and message formats. It runs as the user while
they are logged in, attached to the machine's Reticulum instance, and
talks to the interface through files only:

  ~/.local/share/resilience/messages/      (0700)
    identity        the account's Reticulum identity (private key, 0600)
    address         its LXMF address (32 hex digits), written here
    name            optional display name, written by the interface
    status.json     written here every few seconds: running, address, time
    outbox/ID.json  written by the interface: {"to": ADDRESS, "text": TEXT}
    cancel/ID       written by the interface: give up on message ID
    messages/ID.json  written here only, one per message:
        {"id", "direction": "out"|"in", "peer", "text", "created",
         "updated", "state", "verified", "detail", "attempts"}
    peers.json      written here: addresses heard on the network, with the
                    name each one claims and when it was last heard

States (the specification's, never more than what happened):
  queued     kept here; no confirmed delivery yet, it will be tried again
  sent       handed to the network, no proof of delivery yet
  delivered  the recipient's program returned LXMF's delivery proof
  failed     given up (cancelled, too old, or refused), see "detail"
  received   an incoming message; "verified" says whether its signature
             was checked against the sender's announced key (it becomes
             true later if the sender is only identified afterwards)

Usage: resilience-messages run [--dir DIR] [--client-config FILE]
"""

import json
import os
import re
import shutil
import signal
import sys
import threading
import time

CLIENT_CONFIG = "/run/resilience-reticulum/client/config"
ADDRESS = re.compile(r"[0-9a-f]{32}")
MESSAGE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}")
# Limits on what is accepted from the interface and from the network.
MAX_TEXT_BYTES = 8 * 1024
MAX_OUTBOX_FILE = 64 * 1024
MAX_NAME_BYTES = 64
MAX_PEERS = 500
MAX_STORED = 5000
# A queued message is tried again after these delays, then every hour,
# and at once when its recipient is heard. It is given up after a week.
RETRY_DELAYS = [30, 60, 300, 900, 3600]
GIVE_UP_AFTER = 7 * 24 * 3600
ANNOUNCE_EVERY = 600
ANNOUNCE_BEFORE_SENDING = 60
# Unverified messages are checked again for this long, at most this many.
RECHECK_FOR = 600
MAX_UNVERIFIED = 200
TICK = 1.0


def write_json(path, value, mode=0o600):
    """Atomically replace `path` (a reader never sees half a file)."""
    tmp = f"{path}.{os.getpid()}.tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, mode)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(value, f, ensure_ascii=False, sort_keys=True)
        f.write("\n")
    os.replace(tmp, path)


def read_json(path, limit):
    """The JSON value of a regular file of at most `limit` bytes, or None."""
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    except OSError:
        return None
    with os.fdopen(fd, "rb") as f:
        data = f.read(limit + 1)
    if len(data) > limit:
        return None
    try:
        return json.loads(data.decode("utf-8"))
    except (ValueError, UnicodeDecodeError):
        return None


def valid_request(req):
    """Why an outbox request is refused, or None."""
    if not isinstance(req, dict):
        return "not a message request"
    to, text = req.get("to"), req.get("text")
    if not isinstance(to, str) or not ADDRESS.fullmatch(to):
        return "the address is not 32 hexadecimal digits"
    if not isinstance(text, str) or not text.strip():
        return "the message is empty"
    if len(text.encode("utf-8")) > MAX_TEXT_BYTES:
        return "the message is too long"
    return None


def retry_delay(attempts):
    return RETRY_DELAYS[min(max(attempts, 1), len(RETRY_DELAYS)) - 1]


def clean_name(raw):
    """A display name claimed on the network: printable, one line, short."""
    if not isinstance(raw, str):
        return ""
    name = "".join(c for c in raw if c.isprintable()).strip()
    while len(name.encode("utf-8")) > MAX_NAME_BYTES:
        name = name[:-1]
    return name


class Store:
    """The files of one account."""

    def __init__(self, root):
        self.root = root
        for d in ("", "outbox", "cancel", "messages", "reticulum", "lxmf"):
            os.makedirs(os.path.join(root, d), mode=0o700, exist_ok=True)
        os.chmod(root, 0o700)

    def path(self, *parts):
        return os.path.join(self.root, *parts)

    def message_path(self, mid):
        return self.path("messages", mid + ".json")

    def save(self, rec):
        rec["updated"] = int(time.time())
        write_json(self.message_path(rec["id"]), rec)

    def load_all(self):
        out = {}
        for name in sorted(os.listdir(self.path("messages"))):
            if not name.endswith(".json") or not MESSAGE_ID.fullmatch(name[:-5]):
                continue
            rec = read_json(self.path("messages", name), 4 * MAX_OUTBOX_FILE)
            if isinstance(rec, dict) and rec.get("id") == name[:-5]:
                out[rec["id"]] = rec
        return out

    def take_outbox(self):
        """New requests, oldest first: (id, request or None). Files are removed."""
        box = self.path("outbox")
        found = []
        for name in sorted(os.listdir(box)):
            full = os.path.join(box, name)
            if name.endswith(".json") and MESSAGE_ID.fullmatch(name[:-5]):
                found.append((name[:-5], read_json(full, MAX_OUTBOX_FILE)))
            elif name.endswith(".tmp"):
                continue  # being written
            try:
                os.unlink(full)
            except OSError:
                pass
        return found

    def take_cancels(self):
        box = self.path("cancel")
        ids = []
        for name in sorted(os.listdir(box)):
            if MESSAGE_ID.fullmatch(name):
                ids.append(name)
            try:
                os.unlink(os.path.join(box, name))
            except OSError:
                pass
        return ids

    def display_name(self, fallback):
        try:
            with open(self.path("name"), encoding="utf-8") as f:
                return clean_name(f.read(4 * MAX_NAME_BYTES)) or fallback
        except (OSError, UnicodeDecodeError):
            return fallback


class Messenger:
    def __init__(self, store, client_config):
        import LXMF
        import RNS

        self.LXMF, self.RNS = LXMF, RNS
        self.store = store
        self.lock = threading.RLock()
        self.stop = threading.Event()
        # The instance's name and key may change when the service restarts.
        shutil.copyfile(client_config, store.path("reticulum", "config"))
        os.chmod(store.path("reticulum", "config"), 0o600)
        RNS.Reticulum(configdir=store.path("reticulum"), loglevel=RNS.LOG_WARNING)
        idp = store.path("identity")
        if os.path.exists(idp):
            self.identity = RNS.Identity.from_file(idp)
        else:
            self.identity = RNS.Identity()
            old = os.umask(0o077)
            try:
                self.identity.to_file(idp)
            finally:
                os.umask(old)
        if self.identity is None:
            raise SystemExit("the identity file is damaged: " + idp)
        self.router = LXMF.LXMRouter(identity=self.identity, storagepath=store.path("lxmf"))
        self.name = store.display_name(os.environ.get("USER", "user"))
        self.dest = self.router.register_delivery_identity(self.identity, display_name=self.name)
        self.router.register_delivery_callback(self.on_received)
        self.address = self.dest.hash.hex()
        with open(store.path("address"), "w", encoding="ascii") as f:
            f.write(self.address + "\n")
        self.records = store.load_all()
        self.in_flight = {}
        # Received messages whose signature could not be checked yet.
        self.unverified = {}
        self.peers = read_json(store.path("peers.json"), 1 << 20)
        if not isinstance(self.peers, dict):
            self.peers = {}
        self.peers_dirty = False
        RNS.Transport.register_announce_handler(self)
        # After a restart, what was on its way is simply queued again.
        for rec in self.records.values():
            if rec.get("direction") == "out" and rec.get("state") == "sent":
                rec["state"] = "queued"
                rec["next_try"] = 0
                store.save(rec)
        self.last_announce = 0.0

    # --- Reticulum announce handler -------------------------------------
    aspect_filter = "lxmf.delivery"
    receive_path_responses = True

    def received_announce(self, destination_hash, announced_identity, app_data):
        try:
            name = self.LXMF.display_name_from_app_data(app_data)
        except Exception:
            name = None
        addr = destination_hash.hex()
        if addr == self.address:
            return
        with self.lock:
            if addr not in self.peers and len(self.peers) >= MAX_PEERS:
                oldest = min(self.peers, key=lambda a: self.peers[a].get("heard", 0))
                del self.peers[oldest]
            self.peers[addr] = {"name": clean_name(name), "heard": int(time.time())}
            self.peers_dirty = True
            # Its recipient is here: try what waits for it now.
            for rec in self.records.values():
                if rec.get("state") == "queued" and rec.get("peer") == addr:
                    rec["next_try"] = 0
                    # It may have just started and never heard us: announce
                    # again first, so that it can check our signature.
                    self.last_announce = 0.0

    # --- incoming --------------------------------------------------------
    def on_received(self, m):
        try:
            text = m.content_as_string() or ""
        except Exception:
            text = ""
        raw = text.encode("utf-8", "replace")[:MAX_TEXT_BYTES]
        mid = "in-" + m.hash.hex()[:40]
        if not m.signature_validated:
            # The sender's key is not known here yet: ask the network who
            # this is, and check the signature again once it answers.
            self.RNS.Transport.request_path(m.source_hash)
            with self.lock:
                if len(self.unverified) < MAX_UNVERIFIED and m.packed:
                    self.unverified[mid] = (m.source_hash, bytes(m.packed), time.time())
        with self.lock:
            if mid in self.records:
                return
            rec = {
                "id": mid,
                "direction": "in",
                "peer": m.source_hash.hex(),
                "text": raw.decode("utf-8", "ignore"),
                "created": int(m.timestamp) if m.timestamp else int(time.time()),
                "state": "received",
                "verified": bool(m.signature_validated),
                "detail": "",
                "attempts": 0,
            }
            self.records[mid] = rec
            self.store.save(rec)

    def recheck_signatures(self):
        """Check again the messages whose sender has become known."""
        now = time.time()
        for mid, (source, packed, since) in list(self.unverified.items()):
            if now - since > RECHECK_FOR:
                del self.unverified[mid]
            elif self.RNS.Identity.recall(source) is not None:
                del self.unverified[mid]
                try:
                    again = self.LXMF.LXMessage.unpack_from_bytes(packed)
                except Exception:
                    continue
                rec = self.records.get(mid)
                if rec and again.signature_validated and again.source_hash == source:
                    rec["verified"] = True
                    self.store.save(rec)

    # --- outgoing --------------------------------------------------------
    def accept_outbox(self):
        for mid, req in self.store.take_outbox():
            with self.lock:
                if mid in self.records:
                    continue
                problem = valid_request(req)
                rec = {
                    "id": mid,
                    "direction": "out",
                    "peer": req.get("to", "") if isinstance(req, dict) else "",
                    "text": req.get("text", "") if problem is None else "",
                    "created": int(time.time()),
                    "state": "queued" if problem is None else "failed",
                    "verified": False,
                    "detail": problem or "waiting for the recipient",
                    "attempts": 0,
                    "next_try": 0,
                }
                if not isinstance(rec["peer"], str) or not ADDRESS.fullmatch(rec["peer"]):
                    rec["peer"] = ""
                self.records[mid] = rec
                self.store.save(rec)

    def apply_cancels(self):
        for mid in self.store.take_cancels():
            with self.lock:
                rec = self.records.get(mid)
                if rec and rec.get("direction") == "out" and rec.get("state") in ("queued", "sent"):
                    rec["state"] = "failed"
                    rec["detail"] = "cancelled"
                    self.in_flight.pop(mid, None)
                    self.store.save(rec)

    def try_send(self, rec):
        RNS, LXMF = self.RNS, self.LXMF
        peer = bytes.fromhex(rec["peer"])
        now = time.time()
        if now - rec["created"] > GIVE_UP_AFTER:
            rec["state"] = "failed"
            rec["detail"] = "not delivered after 7 days"
            self.store.save(rec)
            return
        recipient = RNS.Identity.recall(peer)
        if recipient is None:
            # Never heard: ask the network who this is, and wait.
            RNS.Transport.request_path(peer)
            rec["attempts"] += 1
            rec["next_try"] = now + retry_delay(rec["attempts"])
            rec["detail"] = "the recipient has not been heard on the network"
            self.store.save(rec)
            return
        if now - self.last_announce > ANNOUNCE_BEFORE_SENDING:
            # The recipient checks our signature with the key of our
            # announce: make sure it has heard a recent one first.
            self.router.announce(self.dest.hash)
            self.last_announce = now
            rec["next_try"] = now + 2
            return
        out = RNS.Destination(recipient, RNS.Destination.OUT, RNS.Destination.SINGLE, "lxmf", "delivery")
        m = LXMF.LXMessage(out, self.dest, rec["text"], desired_method=LXMF.LXMessage.DIRECT)
        rec["attempts"] += 1
        rec["detail"] = "trying to reach the recipient"
        self.in_flight[rec["id"]] = m
        self.store.save(rec)
        self.router.handle_outbound(m)

    def follow(self, rec, m):
        """Copy the state of an LXMF message in flight into its record."""
        LXMF = self.LXMF
        if m.state == LXMF.LXMessage.DELIVERED:
            rec["state"], rec["detail"] = "delivered", ""
            del self.in_flight[rec["id"]]
        elif m.state == LXMF.LXMessage.SENT and rec["state"] != "sent":
            rec["state"], rec["detail"] = "sent", "no proof of delivery yet"
        elif m.state in (LXMF.LXMessage.FAILED, getattr(LXMF.LXMessage, "REJECTED", -1)):
            del self.in_flight[rec["id"]]
            rec["state"] = "queued"
            rec["next_try"] = time.time() + retry_delay(rec["attempts"])
            rec["detail"] = "the recipient could not be reached; it will be tried again"
        else:
            return
        self.store.save(rec)

    def tick(self):
        self.accept_outbox()
        self.apply_cancels()
        now = time.time()
        with self.lock:
            for rec in list(self.records.values()):
                if rec.get("direction") != "out":
                    continue
                m = self.in_flight.get(rec["id"])
                if m is not None:
                    self.follow(rec, m)
                elif rec.get("state") == "queued" and rec.get("next_try", 0) <= now:
                    self.try_send(rec)
            self.recheck_signatures()
            if self.peers_dirty:
                write_json(self.store.path("peers.json"), self.peers)
                self.peers_dirty = False
            self.prune()
        if now - self.last_announce > ANNOUNCE_EVERY:
            self.router.announce(self.dest.hash)
            self.last_announce = now
        write_json(
            self.store.path("status.json"),
            {"address": self.address, "name": self.name, "pid": os.getpid(), "time": int(now)},
        )

    def prune(self):
        """Keep at most MAX_STORED finished messages (the oldest go)."""
        if len(self.records) <= MAX_STORED:
            return
        done = sorted(
            (r for r in self.records.values() if r.get("state") not in ("queued", "sent")),
            key=lambda r: r.get("created", 0),
        )
        for rec in done[: len(self.records) - MAX_STORED]:
            del self.records[rec["id"]]
            try:
                os.unlink(self.store.message_path(rec["id"]))
            except OSError:
                pass

    def run(self):
        while not self.stop.is_set():
            self.tick()
            self.stop.wait(TICK)
        try:
            os.unlink(self.store.path("status.json"))
        except OSError:
            pass


def default_dir():
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, "resilience", "messages")


def main(argv):
    if len(argv) < 2 or argv[1] != "run":
        sys.stderr.write(__doc__.strip().splitlines()[-1] + "\n")
        return 64
    root, client = default_dir(), CLIENT_CONFIG
    args = argv[2:]
    while args:
        if args[0] == "--dir" and len(args) > 1:
            root, args = args[1], args[2:]
        elif args[0] == "--client-config" and len(args) > 1:
            client, args = args[1], args[2:]
        else:
            sys.stderr.write(f"unknown argument {args[0]!r}\n")
            return 64
    if os.geteuid() == 0:
        sys.stderr.write("resilience-messages must not run as root\n")
        return 1
    if not os.path.isfile(client):
        sys.stderr.write(f"local messaging is not enabled on this computer ({client} is missing)\n")
        return 1
    os.umask(0o077)
    messenger = Messenger(Store(root), client)
    signal.signal(signal.SIGTERM, lambda *_: messenger.stop.set())
    signal.signal(signal.SIGINT, lambda *_: messenger.stop.set())
    messenger.run()
    # Reticulum's exit handlers may hang on the instance's socket.
    sys.stdout.flush()
    os._exit(0)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
