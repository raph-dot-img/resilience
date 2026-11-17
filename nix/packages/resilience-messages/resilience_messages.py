"""resilience-messages: the per-user local messaging program (ADR 0018 §5).

It is glue around LXMF (`LXMF.LXMRouter`) and Reticulum, which do all the
networking, cryptography and message formats. It runs as the user while
they are logged in, attached to the machine's Reticulum instance, and
talks to the interface through files only:

  ~/.local/share/resilience/messages/      (0700)
    identity        the account's Reticulum identity (private key, 0600)
    address         its LXMF address (32 hex digits), written here
    name            optional display name, written by the interface
    status.json     written here every second: running, address, time, and
                    "store", the message store (propagation node) in use
    outbox/ID.json  written by the interface: {"to": ADDRESS, "text": TEXT}
                    and, for a file (ADR 0020), "file": NAME, with the
                    file's bytes in outbox/ID.file written just before
    cancel/ID       written by the interface: give up on message ID
    messages/ID.json  written here only, one per message:
        {"id", "direction": "out"|"in", "peer", "text", "created",
         "updated", "state", "verified", "detail", "attempts"}
        and with a file "file": {"name", "size"} (incoming: "data", the
        file's name in files/)
    files/          written here: received files, named by message id
                    (ID.EXT), and the copies of files waiting to be sent
                    (ID.data, removed once delivered or given up)
    peers.json      written here: addresses heard on the network, with the
                    name each one claims and when it was last heard

States (the specification's, never more than what happened):
  queued     kept here; no confirmed delivery yet, it will be tried again
  sent       handed to the network, no proof of delivery yet; with
             "stored": true, a copy waits in a message store on the local
             network for the recipient to collect it (ADR 0019)
  delivered  the recipient's program returned LXMF's delivery proof
  failed     given up (cancelled, too old, or refused), see "detail"
  received   an incoming message; "verified" says whether its signature
             was checked against the sender's announced key (it becomes
             true later if the sender is only identified afterwards)

Store-and-forward (ADR 0019): a message still undelivered after two minutes
is also left with a message store, an LXMF propagation node: the
machine's own (`node` below) or one heard on the local network. Direct
attempts go on; both copies have the same LXMF hash, so the recipient
keeps one. The program collects its own waiting messages from the store
regularly.

  resilience-messages node   the machine's message store: an LXMF
      propagation node with no address of its own (it receives nothing for
      itself), a bounded store, and peering only with the stores of the
      local network. Run by the system as an unprivileged user.

Files (ADR 0020): one per message, at most 900 KB, in LXMF's
FIELD_FILE_ATTACHMENTS. Received files are kept under the message id,
never under the name that came with them, within 64 MB per account.

Usage: resilience-messages run [--dir DIR] [--client-config FILE] [--local-store FILE]
       resilience-messages node --dir DIR [--client-config FILE] [--address-file FILE] [--limit-mb N]
"""

import json
import os
import re
import shutil
import signal
import stat
import sys
import threading
import time

CLIENT_CONFIG = "/run/resilience-reticulum/client/config"
LOCAL_STORE = "/run/resilience-messages-node/address"
ADDRESS = re.compile(r"[0-9a-f]{32}")
MESSAGE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,63}")
# Limits on what is accepted from the interface and from the network.
MAX_TEXT_BYTES = 8 * 1024
# Files (ADR 0020): the whole message stays under LXMF's delivery limit
# (1000 KB); received files take at most this much room per account.
MAX_FILE_BYTES = 900 * 1000
MAX_RECEIVED_BYTES = 64 * 1024 * 1024
MAX_FILE_NAME_BYTES = 100
FILE_EXT = re.compile(r"[a-z0-9]{1,8}")
# A file copied into the outbox without its request (the interface
# stopped in between) is removed after this long.
STRAY_FILE_AFTER = 3600
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
# Unverified messages are checked again for this long, at most this many
# (a message collected from a store may come from someone who is away).
RECHECK_FOR = 7 * 24 * 3600
MAX_UNVERIFIED = 200
TICK = 1.0
# Store-and-forward (ADR 0019).
PROPAGATE_AFTER = 120
STORE_RETRY = 300
COLLECT_FIRST = 20
COLLECT_EVERY = 300
STORE_HEARD_FOR = 3600
MAX_STORES = 50
NODE_ANNOUNCE_EVERY = 600
# The lowest stamp cost LXMF accepts (LXMRouter.PROPAGATION_COST_MIN):
# slow computers must be able to leave messages.
NODE_STAMP_COST = 13
# Stores asking more than LXMF's default cost are not used.
MAX_STORE_COST = 16
NODE_STORAGE_MB = 32
# Our stores pass messages up to 1000 KB to each other (files, ADR 0020);
# LXMF's default is 256 KB, assumed for a store whose limit is not known.
NODE_TRANSFER_LIMIT_KB = 1000
DEFAULT_STORE_LIMIT_KB = 256
SAVE_KEYS_EVERY = 600


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


def valid_request(req, file_size=None):
    """Why an outbox request is refused, or None. `file_size` is the size
    of the file copied with it, None if there is none."""
    if not isinstance(req, dict):
        return "not a message request"
    to, text = req.get("to"), req.get("text", "")
    if not isinstance(to, str) or not ADDRESS.fullmatch(to):
        return "the address is not 32 hexadecimal digits"
    if not isinstance(text, str):
        return "the message is empty"
    if "file" in req:
        if not isinstance(req["file"], str) or not clean_file_name(req["file"]):
            return "the file has no name"
        if file_size is None:
            return "the file to send was not found"
        if file_size > MAX_FILE_BYTES:
            return "the file is too large (at most 900 KB)"
    elif not text.strip():
        return "the message is empty"
    if len(text.encode("utf-8")) > MAX_TEXT_BYTES:
        return "the message is too long"
    return None


def clean_file_name(raw):
    """A file name that came with a message: its last part, printable, one
    line, at most MAX_FILE_NAME_BYTES. Only ever shown, never used as a path
    here (the interface makes its own safe name when the user saves it)."""
    if isinstance(raw, bytes):
        raw = raw.decode("utf-8", "replace")
    if not isinstance(raw, str):
        return ""
    raw = raw.replace("\\", "/").rsplit("/", 1)[-1]
    name = "".join(c for c in raw if c.isprintable()).strip()
    while len(name.encode("utf-8")) > MAX_FILE_NAME_BYTES:
        name = name[:-1]
    return name


def stored_file_name(mid, name):
    """files/ name of a received file: the message id, and the name's
    extension when it is a plain one (so the viewers know the kind)."""
    ext = name.rsplit(".", 1)[-1].lower() if "." in name else ""
    # ".data" names the copies of files being sent.
    return f"{mid}.{ext}" if FILE_EXT.fullmatch(ext) and ext != "data" else mid


def attachment(fields, LXMF):
    """(name, bytes, how many others) of the first file of a message."""
    if not isinstance(fields, dict):
        return None
    files = fields.get(LXMF.FIELD_FILE_ATTACHMENTS)
    found = []
    if isinstance(files, list):
        for f in files:
            if isinstance(f, (list, tuple)) and len(f) >= 2 and isinstance(f[1], (bytes, bytearray)):
                found.append((clean_file_name(f[0]) or "file", bytes(f[1])))
    image = fields.get(LXMF.FIELD_IMAGE)
    if isinstance(image, (list, tuple)) and len(image) >= 2 and isinstance(image[1], (bytes, bytearray)):
        kind = image[0].decode("ascii", "replace") if isinstance(image[0], bytes) else str(image[0])
        kind = kind.lower().rsplit("/", 1)[-1]
        found.append(("picture." + kind if FILE_EXT.fullmatch(kind) else "picture", bytes(image[1])))
    if not found:
        return None
    return found[0][0], found[0][1], len(found) - 1


def retry_delay(attempts):
    return RETRY_DELAYS[min(max(attempts, 1), len(RETRY_DELAYS)) - 1]


def waiting_detail(rec, base):
    """What a waiting message's record says, with why no store keeps it."""
    if rec.get("too_large_for_store"):
        return base + "; too large for the message store: it arrives only when both computers are on"
    return base


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
        for d in ("", "outbox", "cancel", "messages", "files", "reticulum", "lxmf"):
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
        """New requests, oldest first: (id, request or None, size of its
        file or None). Requests are removed; their files are moved to
        files/ID.data."""
        box = self.path("outbox")
        found = []
        for name in sorted(os.listdir(box)):
            full = os.path.join(box, name)
            if name.endswith(".json") and MESSAGE_ID.fullmatch(name[:-5]):
                req = read_json(full, MAX_OUTBOX_FILE)
                size = self.take_file(name[:-5]) if isinstance(req, dict) and "file" in req else None
                found.append((name[:-5], req, size))
            elif name.endswith(".tmp"):
                continue  # being written
            elif name.endswith(".file") and MESSAGE_ID.fullmatch(name[:-5]):
                # Its request comes next (".json" sorts after ".file"), or
                # is being written; an old one alone is left over.
                try:
                    if time.time() - os.lstat(full).st_mtime < STRAY_FILE_AFTER:
                        continue
                except OSError:
                    continue
            try:
                os.unlink(full)
            except OSError:
                pass
        return found

    def take_file(self, mid):
        """Move outbox/ID.file to files/ID.data; its size, or None."""
        src = self.path("outbox", mid + ".file")
        try:
            st = os.lstat(src)
        except OSError:
            return None
        if not stat.S_ISREG(st.st_mode) or st.st_size > MAX_FILE_BYTES:
            try:
                os.unlink(src)
            except OSError:
                pass
            return MAX_FILE_BYTES + 1 if stat.S_ISREG(st.st_mode) else None
        os.replace(src, self.path("files", mid + ".data"))
        return st.st_size

    def outgoing_data(self, mid):
        return self.path("files", mid + ".data")

    def drop_outgoing(self, mid):
        try:
            os.unlink(self.outgoing_data(mid))
        except OSError:
            pass

    def received_bytes(self):
        total = 0
        for name in os.listdir(self.path("files")):
            if name.endswith(".data"):
                continue
            try:
                total += os.lstat(self.path("files", name)).st_size
            except OSError:
                pass
        return total

    def keep_received(self, mid, name, data):
        """Write a received file as files/ID.EXT; its name there, or None
        when received files are full."""
        if self.received_bytes() + len(data) > MAX_RECEIVED_BYTES:
            return None
        stored = stored_file_name(mid, name)
        tmp = self.path("files", f"{stored}.{os.getpid()}.tmp")
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as f:
            f.write(data)
        os.replace(tmp, self.path("files", stored))
        return stored

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


def read_address(path):
    """A 32-hex-digit address from a small file, or None."""
    try:
        with open(path, encoding="ascii") as f:
            text = f.read(100).strip()
    except (OSError, UnicodeDecodeError):
        return None
    return text if ADDRESS.fullmatch(text) else None


def attach(RNS, configdir, client_config):
    """Attach to the machine's Reticulum instance, never start one.

    Without require_shared_instance, a program started before the instance
    would become the shared instance itself and take its socket name.
    """
    os.makedirs(configdir, mode=0o700, exist_ok=True)
    # The instance's name and key may change when the service restarts.
    shutil.copyfile(client_config, os.path.join(configdir, "config"))
    os.chmod(os.path.join(configdir, "config"), 0o600)
    try:
        RNS.Reticulum(configdir=configdir, loglevel=RNS.LOG_WARNING, require_shared_instance=True)
    except SystemError:
        sys.stderr.write("the machine's Reticulum instance is not running yet\n")
        sys.stdout.flush()
        os._exit(75)


def load_identity(RNS, path):
    if os.path.exists(path):
        identity = RNS.Identity.from_file(path)
        if identity is None:
            raise SystemExit("the identity file is damaged: " + path)
        return identity
    identity = RNS.Identity()
    old = os.umask(0o077)
    try:
        identity.to_file(path)
    finally:
        os.umask(old)
    return identity


def save_keys(RNS):
    """Keep the public keys heard so far (Reticulum's clients never do it
    themselves): a message collected from a store while its sender is away
    can then still be checked."""
    try:
        RNS.Identity.save_known_destinations()
    except Exception:
        pass


def shut_down(RNS, router):
    """LXMF's own state (delivered message ids, a node's peers), the keys."""
    try:
        router.exit_handler()
    except Exception:
        pass
    save_keys(RNS)
    sys.stdout.flush()
    # Reticulum's exit handlers may hang on the instance's socket.
    os._exit(0)


class Stores:
    """Message stores (LXMF propagation nodes) heard on the network."""

    aspect_filter = "lxmf.propagation"
    receive_path_responses = True

    def __init__(self, LXMF):
        self.LXMF = LXMF
        self.lock = threading.Lock()
        self.heard = {}
        self.limits = {}

    def received_announce(self, destination_hash, announced_identity, app_data):
        try:
            if not self.LXMF.pn_announce_data_is_valid(app_data):
                return
            import RNS.vendor.umsgpack as msgpack

            data = msgpack.unpackb(app_data)
            if not data[2]:
                return  # a node that says it is not running
            if int(data[5][0]) > MAX_STORE_COST:
                return  # its stamps would keep this computer busy
            limit = int(data[3])
        except Exception:
            return
        with self.lock:
            if destination_hash not in self.heard and len(self.heard) >= MAX_STORES:
                del self.heard[min(self.heard, key=self.heard.get)]
                self.limits.pop(destination_hash, None)
            self.heard[destination_hash] = time.time()
            self.limits[destination_hash] = limit

    def limit_bytes(self, destination_hash):
        """The largest message a store passes on, as it announced it."""
        with self.lock:
            return 1000 * self.limits.get(destination_hash, DEFAULT_STORE_LIMIT_KB)

    def nearest(self, RNS):
        """The closest store heard in the last hour, the most recent first."""
        now = time.time()
        with self.lock:
            fresh = [(h, t) for h, t in self.heard.items() if now - t < STORE_HEARD_FOR]
        if not fresh:
            return None
        fresh.sort(key=lambda ht: (RNS.Transport.hops_to(ht[0]), -ht[1]))
        return fresh[0][0]


class Messenger:
    def __init__(self, store, client_config, local_store=LOCAL_STORE):
        import LXMF
        import RNS

        self.LXMF, self.RNS = LXMF, RNS
        self.store = store
        self.lock = threading.RLock()
        self.stop = threading.Event()
        attach(RNS, store.path("reticulum"), client_config)
        self.identity = load_identity(RNS, store.path("identity"))
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
        # After a restart, what was on its way is simply queued again (a
        # copy already in a store stays "sent").
        for rec in self.records.values():
            if rec.get("direction") == "out" and rec.get("state") == "sent":
                if not rec.get("stored"):
                    rec["state"] = "queued"
                rec["next_try"] = 0
                store.save(rec)
        # Copies of files whose message is finished (stopped in between).
        for name in os.listdir(store.path("files")):
            mid = name[:-5] if name.endswith(".data") else None
            if mid is not None and self.records.get(mid, {}).get("state") not in ("queued", "sent"):
                store.drop_outgoing(mid)
        self.last_announce = 0.0
        # Store-and-forward (ADR 0019).
        self.local_store_file = local_store
        self.stores = Stores(LXMF)
        RNS.Transport.register_announce_handler(self.stores)
        self.store_in_use = None
        self.store_is_local = False
        self.to_store = {}
        self.started = time.time()
        self.next_collect = self.started + COLLECT_FIRST
        self.last_keys_saved = self.started

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
        try:
            att = attachment(m.fields, self.LXMF)
        except Exception:
            att = None
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
            if att is not None:
                name, data, others = att
                rec["file"] = {"name": name, "size": len(data)}
                stored = self.store.keep_received(mid, name, data)
                if stored is None:
                    rec["detail"] = "file not kept: received files are full (64 MB); save or remove some"
                else:
                    rec["file"]["data"] = stored
                if others:
                    rec["detail"] = (rec["detail"] + "; " if rec["detail"] else "") + (
                        f"{others} more file(s) in this message were not kept"
                    )
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
        for mid, req, file_size in self.store.take_outbox():
            with self.lock:
                if mid in self.records:
                    continue
                problem = valid_request(req, file_size)
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
                    # LXMF's timestamp, the same for every copy: a message
                    # sent directly and through a store has one hash.
                    "ts": time.time(),
                }
                if not isinstance(rec["peer"], str) or not ADDRESS.fullmatch(rec["peer"]):
                    rec["peer"] = ""
                if isinstance(req, dict) and isinstance(req.get("file"), str):
                    rec["file"] = {"name": clean_file_name(req["file"]), "size": file_size or 0}
                if problem is not None:
                    self.store.drop_outgoing(mid)
                self.records[mid] = rec
                self.store.save(rec)

    def apply_cancels(self):
        for mid in self.store.take_cancels():
            with self.lock:
                rec = self.records.get(mid)
                if rec and rec.get("direction") == "out" and rec.get("state") in ("queued", "sent"):
                    rec["state"] = "failed"
                    rec["detail"] = "cancelled"
                    self.store.drop_outgoing(mid)
                    if rec.get("stored"):
                        rec["detail"] = "cancelled; the copy left in a message store may still arrive"
                    self.in_flight.pop(mid, None)
                    m = self.to_store.pop(mid, None)
                    if m is not None:
                        try:
                            self.router.cancel_outbound(m.message_id)
                        except Exception:
                            pass
                    self.store.save(rec)

    def try_send(self, rec):
        RNS, LXMF = self.RNS, self.LXMF
        peer = bytes.fromhex(rec["peer"])
        now = time.time()
        if now - rec["created"] > GIVE_UP_AFTER:
            rec["state"] = "failed"
            rec["detail"] = "not delivered after 7 days"
            if rec.get("stored"):
                rec["detail"] += "; the copy left in a message store may still arrive"
            self.store.drop_outgoing(rec["id"])
            self.store.save(rec)
            return
        recipient = RNS.Identity.recall(peer)
        if recipient is None:
            # Never heard: ask the network who this is, and wait.
            RNS.Transport.request_path(peer)
            rec["attempts"] += 1
            rec["next_try"] = now + retry_delay(rec["attempts"])
            if not rec.get("stored"):
                rec["detail"] = waiting_detail(rec, "the recipient has not been heard on the network")
            self.store.save(rec)
            return
        if now - self.last_announce > ANNOUNCE_BEFORE_SENDING:
            # The recipient checks our signature with the key of our
            # announce: make sure it has heard a recent one first.
            self.router.announce(self.dest.hash)
            self.last_announce = now
            rec["next_try"] = now + 2
            return
        m = self.lxmessage(rec, recipient, LXMF.LXMessage.DIRECT)
        if m is None:
            return
        rec["attempts"] += 1
        if not rec.get("stored"):
            rec["detail"] = waiting_detail(rec, "trying to reach the recipient")
        self.in_flight[rec["id"]] = m
        self.store.save(rec)
        self.router.handle_outbound(m)

    def lxmessage(self, rec, recipient, method):
        RNS, LXMF = self.RNS, self.LXMF
        out = RNS.Destination(recipient, RNS.Destination.OUT, RNS.Destination.SINGLE, "lxmf", "delivery")
        fields = None
        if isinstance(rec.get("file"), dict):
            try:
                with open(self.store.outgoing_data(rec["id"]), "rb") as f:
                    data = f.read(MAX_FILE_BYTES + 1)
            except OSError:
                data = None
            if data is None or len(data) > MAX_FILE_BYTES:
                rec["state"], rec["detail"] = "failed", "the copy of the file to send is missing"
                self.store.drop_outgoing(rec["id"])
                self.store.save(rec)
                return None
            fields = {LXMF.FIELD_FILE_ATTACHMENTS: [[rec["file"]["name"], data]]}
        m = LXMF.LXMessage(out, self.dest, rec["text"], fields=fields, desired_method=method)
        # The same timestamp, so the same hash, for every copy (older
        # records have none: their creation time).
        ts = rec.get("ts")
        m.timestamp = float(ts) if isinstance(ts, (int, float)) else float(rec["created"])
        return m

    # --- store-and-forward (ADR 0019) -------------------------------------
    def choose_store(self):
        """The machine's own store when it runs one, else the nearest heard."""
        local = read_address(self.local_store_file)
        choice = bytes.fromhex(local) if local else self.stores.nearest(self.RNS)
        # The machine's own store is ours: its limit is known.
        self.store_is_local = bool(local)
        if choice != self.store_in_use:
            self.store_in_use = choice
            if choice is not None:
                self.router.set_outbound_propagation_node(choice)
                self.next_collect = min(self.next_collect, time.time() + 2)

    def leave_in_store(self, rec, now):
        """Hand a copy of a message that waits to the message store."""
        if self.store_in_use is None or rec["id"] in self.to_store:
            return
        if rec.get("stored") or now - rec["created"] < PROPAGATE_AFTER:
            return
        if now < rec.get("next_store_try", 0):
            return
        recipient = self.RNS.Identity.recall(bytes.fromhex(rec["peer"]))
        if recipient is None:
            # The store cannot encrypt for a key nobody here has heard.
            return
        size = len(rec.get("text", "").encode("utf-8"))
        if isinstance(rec.get("file"), dict):
            size += int(rec["file"].get("size", 0))
        # Room for LXMF's own fields, signature and encryption.
        limit = NODE_TRANSFER_LIMIT_KB * 1000 if self.store_is_local else self.stores.limit_bytes(self.store_in_use)
        too_large = size + 2048 > limit
        if too_large != bool(rec.get("too_large_for_store")):
            rec["too_large_for_store"] = too_large
            if rec["state"] == "queued":
                rec["detail"] = waiting_detail(rec, "waiting for the recipient")
            self.store.save(rec)
        if too_large:
            return
        rec["next_store_try"] = now + STORE_RETRY
        m = self.lxmessage(rec, recipient, self.LXMF.LXMessage.PROPAGATED)
        if m is None:
            return
        self.to_store[rec["id"]] = m
        self.router.handle_outbound(m)

    def follow_store(self, rec, m):
        LXMF = self.LXMF
        if m.state == LXMF.LXMessage.SENT:
            del self.to_store[rec["id"]]
            rec["stored"] = True
            if rec["state"] == "queued":
                rec["state"] = "sent"
            if rec["state"] == "sent":
                rec["detail"] = "a copy waits in a message store on the local network; no proof of delivery yet"
            self.store.save(rec)
        elif m.state in (LXMF.LXMessage.FAILED, getattr(LXMF.LXMessage, "REJECTED", -1),
                         getattr(LXMF.LXMessage, "CANCELLED", -1)):
            del self.to_store[rec["id"]]

    def collect(self, now):
        """Ask the store for the messages that wait for this account."""
        if self.store_in_use is None or now < self.next_collect:
            return
        self.next_collect = now + COLLECT_EVERY
        idle = (
            self.LXMF.LXMRouter.PR_IDLE,
            self.LXMF.LXMRouter.PR_COMPLETE,
        )
        if self.router.propagation_transfer_state in idle or self.router.propagation_transfer_state >= 0xF0:
            self.router.request_messages_from_propagation_node(self.identity)

    def follow(self, rec, m):
        """Copy the state of an LXMF message in flight into its record."""
        LXMF = self.LXMF
        if m.state == LXMF.LXMessage.DELIVERED:
            rec["state"], rec["detail"] = "delivered", ""
            del self.in_flight[rec["id"]]
            self.to_store.pop(rec["id"], None)
            self.store.drop_outgoing(rec["id"])
        elif m.state == LXMF.LXMessage.SENT and rec["state"] != "sent":
            rec["state"], rec["detail"] = "sent", "no proof of delivery yet"
        elif m.state in (LXMF.LXMessage.FAILED, getattr(LXMF.LXMessage, "REJECTED", -1)):
            del self.in_flight[rec["id"]]
            rec["next_try"] = time.time() + retry_delay(rec["attempts"])
            if rec.get("stored"):
                rec["state"] = "sent"
            else:
                rec["state"] = "queued"
                rec["detail"] = waiting_detail(rec, "the recipient could not be reached; it will be tried again")
        else:
            return
        self.store.save(rec)

    def tick(self):
        self.accept_outbox()
        self.apply_cancels()
        now = time.time()
        self.choose_store()
        with self.lock:
            for rec in list(self.records.values()):
                if rec.get("direction") != "out":
                    continue
                m = self.in_flight.get(rec["id"])
                if m is not None:
                    self.follow(rec, m)
                s = self.to_store.get(rec["id"])
                if s is not None:
                    self.follow_store(rec, s)
                waiting = rec.get("state") == "queued" or (rec.get("state") == "sent" and rec.get("stored"))
                if not waiting:
                    continue
                if m is None and rec.get("next_try", 0) <= now:
                    self.try_send(rec)
                if rec.get("state") in ("queued", "sent"):
                    self.leave_in_store(rec, now)
            self.recheck_signatures()
            if self.peers_dirty:
                write_json(self.store.path("peers.json"), self.peers)
                self.peers_dirty = False
            self.prune()
        self.collect(now)
        if now - self.last_announce > ANNOUNCE_EVERY:
            self.router.announce(self.dest.hash)
            self.last_announce = now
        if now - self.last_keys_saved > SAVE_KEYS_EVERY:
            self.last_keys_saved = now
            save_keys(self.RNS)
        write_json(
            self.store.path("status.json"),
            {
                "address": self.address,
                "name": self.name,
                "pid": os.getpid(),
                "time": int(now),
                "store": self.store_in_use.hex() if self.store_in_use else None,
            },
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
            self.store.drop_outgoing(rec["id"])
            f = rec.get("file")
            if isinstance(f, dict) and isinstance(f.get("data"), str):
                name = os.path.basename(f["data"])
                if name.split(".", 1)[0] == rec["id"]:
                    try:
                        os.unlink(self.store.path("files", name))
                    except OSError:
                        pass
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


class Node:
    """The machine's message store (ADR 0019): an LXMF propagation node.

    It registers no delivery address, so nothing is ever delivered *to* it:
    it only keeps messages encrypted for others, within a size limit, and
    exchanges them with the stores of the local network.
    """

    def __init__(self, root, client_config, address_file, limit_mb):
        import LXMF
        import RNS

        self.RNS = RNS
        self.stop = threading.Event()
        os.makedirs(root, mode=0o700, exist_ok=True)
        attach(RNS, os.path.join(root, "reticulum"), client_config)
        identity = load_identity(RNS, os.path.join(root, "identity"))
        self.router = LXMF.LXMRouter(
            identity=identity,
            storagepath=os.path.join(root, "lxmf"),
            autopeer=True,
            # Only the stores of the local network itself.
            autopeer_maxdepth=1,
            propagation_cost=NODE_STAMP_COST,
            propagation_cost_flexibility=0,
            propagation_limit=NODE_TRANSFER_LIMIT_KB,
        )
        self.router.set_message_storage_limit(megabytes=limit_mb)
        self.router.enable_propagation()
        self.address = self.router.propagation_destination.hash.hex()
        if address_file:
            tmp = address_file + ".tmp"
            with open(tmp, "w", encoding="ascii") as f:
                f.write(self.address + "\n")
            os.chmod(tmp, 0o644)
            os.replace(tmp, address_file)
        print(f"message store {self.address} running, limit {limit_mb} MB", flush=True)

    def run(self):
        last_announce = last_saved = time.time()
        while not self.stop.wait(10):
            now = time.time()
            if now - last_announce > NODE_ANNOUNCE_EVERY:
                self.router.announce_propagation_node()
                last_announce = now
            if now - last_saved > SAVE_KEYS_EVERY:
                save_keys(self.RNS)
                last_saved = now


def default_dir():
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(base, "resilience", "messages")


def usage():
    sys.stderr.write("\n".join(__doc__.strip().splitlines()[-2:]) + "\n")
    return 64


def main(argv):
    if len(argv) < 2 or argv[1] not in ("run", "node"):
        return usage()
    mode = argv[1]
    opts = {
        "--dir": default_dir() if mode == "run" else None,
        "--client-config": CLIENT_CONFIG,
        "--local-store": LOCAL_STORE,
        "--address-file": None,
        "--limit-mb": str(NODE_STORAGE_MB),
    }
    allowed = {"run": ("--dir", "--client-config", "--local-store"),
               "node": ("--dir", "--client-config", "--address-file", "--limit-mb")}[mode]
    args = argv[2:]
    while args:
        if args[0] in allowed and len(args) > 1:
            opts[args[0]], args = args[1], args[2:]
        else:
            sys.stderr.write(f"unknown argument {args[0]!r}\n")
            return 64
    if opts["--dir"] is None:
        return usage()
    if os.geteuid() == 0:
        sys.stderr.write("resilience-messages must not run as root\n")
        return 1
    client = opts["--client-config"]
    if not os.path.isfile(client):
        sys.stderr.write(f"local messaging is not enabled on this computer ({client} is missing)\n")
        return 1
    os.umask(0o077)
    if mode == "node":
        try:
            limit = int(opts["--limit-mb"])
        except ValueError:
            limit = 0
        if not 1 <= limit <= 100000:
            sys.stderr.write("--limit-mb must be a number of megabytes from 1 to 100000\n")
            return 64
        worker = Node(opts["--dir"], client, opts["--address-file"], limit)
    else:
        worker = Messenger(Store(opts["--dir"]), client, opts["--local-store"])
    # After LXMF's own handlers: stop cleanly, then let LXMF save its state.
    signal.signal(signal.SIGTERM, lambda *_: worker.stop.set())
    signal.signal(signal.SIGINT, lambda *_: worker.stop.set())
    worker.run()
    shut_down(worker.RNS, worker.router)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
