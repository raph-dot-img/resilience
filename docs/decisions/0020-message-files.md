# ADR 0020: Files in local messages

Status: accepted (2026-11-17).

## Context

Local messages (ADR 0018, 0019) carry text only. In a shelter or a
neighbourhood people also need to pass a list, a form, a map extract, a
key or an encrypted file to someone nearby without a USB stick.

Read in the pinned LXMF (0.9.8), 2026-11-17:

- A message carries files in its field `FIELD_FILE_ATTACHMENTS` (0x05):
  a list of `[name, bytes]`. Other LXMF programs (Sideband, NomadNet,
  MeshChat) use the same field, and Sideband also sends pictures in
  `FIELD_IMAGE` (0x06, `[type, bytes]`).
- A receiving program refuses a direct transfer larger than its
  `delivery_limit`, 1000 KB by default. A client collecting from a
  store announces the same limit, and the store skips larger messages.
- A store (propagation node) accepts an upload from a client up to its
  sync limit (10 MB by default), but passes on to other stores only
  messages under each store's announced `propagation_limit`, 256 KB by
  default.
- The whole message is held in memory while it is built, sent and
  received.

## Decision

1. **One file per message, at most 900 KB**, with an optional text. The
   limit keeps the whole message under LXMF's 1000 KB delivery limit, so
   that any LXMF program can receive it directly or from a store. Larger
   files go on a USB stick (Backups & USB sticks).
2. **Sending**: in a conversation, `f` opens Find & view files as a
   picker; the chosen file is checked (regular file, size) and **copied**
   into the outbox (`outbox/ID.file`) before the request
   (`outbox/ID.json`, with `"file": NAME`) is written. The message sends
   that copy, so changing or deleting the original afterwards changes
   nothing. The program moves the copy to `files/ID.data` while the
   message waits and **deletes it once the message is delivered or given
   up**; the record keeps the name and size.
3. **Receiving**: the program keeps the first attachment of a message
   (first `FIELD_FILE_ATTACHMENTS` entry, else a `FIELD_IMAGE` picture as
   `picture.TYPE`) in `files/ID.EXT`, **named by the message id**, never by
   the name that came from the network; `EXT` is the name's extension if
   it is 1 to 8 lower-case letters or digits (so the file viewers know
   the kind). Mode 0600, written through a temporary name, never
   executable. The record gets `"file": {"name", "size"}` with the name
   cleaned (printable, one line, 100 bytes). Other attachments of the same
   message are not kept and the record says so.
4. **Bounded**: received files take at most **64 MB** per account. When a
   file would not fit, its text is kept, the file is not, and the record
   says "file not kept: received files are full". Space comes back when
   the user saves a file elsewhere (below) or when the program prunes
   old records (their files go with them).
5. **Using a received file**, in the conversation: `o` views it with
   the read-only viewers of Find & view files (text, tables, PDF text,
   archive listing; never run, never opened by another program); `s`
   **moves** it to `~/Received/NAME` (the cleaned name; never replacing
   a file, `NAME (2)` and so on; a name starting with a dot or a dash
   gets a `_` in front), which frees the space. Nothing is opened or
   saved by itself.
6. **Stores**: our stores raise their `propagation_limit` to 1000 KB so
   that they pass files to each other. A program hands a message with a
   file to a store only when that store announced a limit large enough
   for it; otherwise the message waits for direct delivery only, and
   the record says so. Stores' disk limit stays 32 MB: LXMF drops the
   largest and oldest messages first when a store is full.

## Alternatives considered

- **Reticulum resources or a link of our own for large files**: no size
  limit, but another protocol to write and secure, no store-and-forward,
  and other LXMF programs could not take part. Larger files have USB
  sticks.
- **Received files straight into a folder of the home** (like
  `~/Received`): every nearby computer could put files with names of its
  choosing among the user's documents. Kept apart, named by id, and
  moved only on request, they are as visible as the user wants.
- **Accepting files only from named contacts**: safer against
  unsolicited files, but the first exchange with someone would need
  another channel. The quota bounds the cost; files are never opened by
  themselves; "NOT VERIFIED" applies to them as to texts.
- **Sending the original file by path**: no copy, but the message would
  change (or fail) if the file changed during the days it may wait.
- **Several files per message**: possible in LXMF, but one is simpler
  to show and to save; an archive groups files.

## Consequences

- A file message is up to 900 KB of local network traffic per attempt
  and per store; direct attempts are retried as for texts (ADR 0018).
- Up to 64 MB of each home can be filled by nearby computers, as up to
  40 MB already could with texts; the user frees it by saving or by
  nothing (old records are pruned with their files).
- The viewers read hostile files (pdftotext, bsdtar listing) as they
  already do for files found on USB sticks.
- Pictures from Sideband are kept as files but cannot be shown on the
  console (their details and checksum can).

Tested by: `comms-messages` (a file sent through LXMF directly and
through the store, kept under the message id, name cleaned, quota,
outbox copy removed once delivered), TUI tests (picker, size limit,
caption, `o`, `s` into `~/Received` without replacing) and `vm-comms`
(a file sent from alpha's interface, saved on beta's).
