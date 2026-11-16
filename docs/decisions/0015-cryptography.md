# ADR 0015: Guided cryptography

Status: accepted (2026-11-15).

## Context

Phase 4 brings everyday cryptography to every account, Ultra-Stable
included: encrypt and decrypt files and messages, create and exchange
keys, sign files and check signatures, compute and compare checksums,
generate random passwords, and recognise encryption formats
(ORIGINAL-SPEC §17).

The rules are strict:

- no cryptography of our own;
- no secret in argv, logs, the Nix store or permissive temporary files;
- clear warnings about lost keys and irreversible operations;
- never present a result as secure just because a tool succeeded.

Users may be beginners. They must not need to know what a key format, an
armor or a trust model is to use this correctly.

## Decision

### Tools

| Job | Tool | Why |
|---|---|---|
| Encryption (files, messages, own keys) | **age** 1.3 | Small, modern, authenticated (ChaCha20-Poly1305, scrypt for passphrases, X25519 keys), one way to do things, already used for backups (ADR 0008). Public keys are short enough to read aloud or type (62 characters). |
| Signatures | **minisign** 0.12 | Small, Ed25519, signature next to the file, key numbers easy to compare. |
| Checksums | **coreutils** (`sha256sum`, `sha512sum`, `b2sum`, `sha1sum`, `md5sum`) | Standard, and what published checksums are made with. |
| Random passphrases | **xkcdpass** (EFF long word list, a French list) | Uses the system's secure random generator; reports its own entropy figure, which we show. |
| Random passwords and keys | **openssl rand** | Established; 144-bit passwords and 256-bit hexadecimal keys. |
| Format details | **age-inspect** and a pure recogniser in `resilience-core::crypto` | The recogniser only reads magic numbers and text headers, never decrypts. |

| OpenPGP interoperability | **GnuPG** 2.4 with **pinentry-curses** | The format of e-mail encryption and of most software signatures. |

### One unprivileged program, run in the foreground

`resilience-crypto` (new crate) does each operation as the user, started
by the interface in the foreground like `resilience-backup`:

```text
encrypt TO FILE | decrypt FILE | write-message TO | read-message FILE |
sign FILE | verify FILE | checksum FILE | compare SUM FILE |
check-list FILE | inspect FILE | new-keys | add-contact [FILE] |
export-keys DIR          (TO = --passphrase, or --me and/or --to CONTACT...)
```

**No secret passes through our code.** age, minisign and pinentry read
passphrases from the terminal themselves. The interface never asks for one,
and `resilience-crypto` never sees one, so none can be in an argv, an
environment variable, a log line or a file we write. Arguments are file
paths, contact names and checksums, none of them secret. The VM test greps
the journal, the home folder and the temporary folders for every
passphrase used.

Outputs never replace anything:

- they are written to a hidden `.NAME.partial-PID` file next to their
  place, created `O_EXCL|O_NOFOLLOW`;
- the file is renamed with `RENAME_NOREPLACE` (on file systems without it,
  a check and then a rename) only when the tool succeeded;
- it is deleted otherwise. A wrong passphrase leaves nothing behind.

| Operation | Result |
|---|---|
| Encrypt | `NAME.age` next to the original, or `NAME (2).age`... |
| Decrypt | into `~/Decrypted` (0700), never next to the encrypted file, which is often on a stick; `NAME (2)` if needed |
| Write a message | `~/Messages/message-STAMP.age.txt` (armored), also shown on screen |
| Sign | `NAME.minisig`; refused if one exists |

The plaintext of a **read message** exists only in memory: age's output
is captured and shown in `less` with `LESSSECURE=1`. Messages are limited
to 1 MiB, and a binary result is refused ("use Decrypt a file").

### Keys and contacts

```text
~/.local/share/resilience/keys/        0700
  own/encryption.age    age identity, ENCRYPTED with a passphrase (armored)
  own/encryption.pub    its public key (age1...)
  own/signing.key       minisign secret key (minisign encrypts it, scrypt)
  own/signing.pub
  contacts/NAME.toml    age = "age1...", minisign = "RW...", added = "..."
```

- The age secret key is made by `age-keygen` and piped straight into
  `age --passphrase --armor`. It exists only in that pipe and reaches the
  disk already encrypted, and `age -d -i` asks for that passphrase when it
  is used. The VM test checks that `AGE-SECRET-KEY` appears nowhere on
  disk.
- One encryption key and one signing key per account. The interface never
  replaces or deletes them: losing a key means losing what was encrypted
  for it.
- Keys live in the home folder, so encrypted backups (ADR 0008) include
  them; key creation says so.
- Contacts hold public keys only:
  - when a contact is added, age itself checks the encryption key (by
    encrypting nothing for it), and the signing key must decode to a
    minisign Ed25519 key;
  - a file containing any private key (age, minisign, OpenSSH, OpenPGP,
    PEM) is refused loudly;
  - a contact whose key differs from the one already stored is refused
    ("a key that changes is a warning sign"); the user removes the contact
    first.
- Trust is explicit and human: adding a contact shows the end of the age
  key and the minisign key number, and asks the user to check them with
  the person, in person or by phone. Signature checks name the contact
  whose key matched. A signature from an unknown key is "NOT CHECKED", not
  "invalid".
- Exported keys are a commented text file (`USER-public-keys.txt`) on a
  stick. It holds both public keys and says it contains no secret.

### OpenPGP (GnuPG), for interoperability

age and minisign are the default. GnuPG is used when the files or the
people require it:

- **Files and keys are recognised, so there is nothing to choose:**
  - decrypting or reading an OpenPGP message runs gpg instead of age;
  - "Check a signature" finds `NAME.sig` or `NAME.asc` next to the file
    (or a clear-signed text);
  - adding a contact from an OpenPGP key file imports it into the GnuPG
    keyring;
  - "What is this file?" lists the keys a message is encrypted for, with
    the matching names in the keyring, without decrypting anything
    (`--list-only`).
- **Keys & contacts** also lists the keyring: the user's own OpenPGP key,
  and the others with the end of their fingerprint. `g` creates an
  OpenPGP key: name and optional e-mail are checked, then `gpg
  --quick-generate-key UID future-default default 2y`, and gpg's
  revocation certificate is pointed out. Delete removes someone else's
  key after confirmation; the user's own key is never deleted from here.
  The exported keys file also contains the user's OpenPGP public key.
- **"Encrypt for"** also offers the OpenPGP keys that can encrypt. A file
  is for one kind only (passphrase, age keys or OpenPGP keys), and
  ticking one kind unticks the others. The result is `NAME.gpg`; messages
  are armored `.asc` files.
- **Passphrases** are asked by gpg-agent's pinentry-curses on the
  terminal (`GPG_TTY` set by `resilience-crypto`). gpg-agent keeps no
  cache (`default-cache-ttl 0`, `max-cache-ttl 0`), so like age and
  minisign every operation asks again.
- **Offline and predictable.** gpg always runs with `--batch` (no
  question of its own; results read from `--status-fd`, gpg's documented
  machine interface, by pure parsers in `resilience-core::openpgp`). It
  also runs with `--no-auto-key-retrieve` and `--no-auto-key-locate`, and
  no key server is configured.
- **Trust is the user's, not GnuPG's.** Before a key is imported, its
  fingerprint is shown (`--show-keys`) with the request to compare it with
  its owner's, and confirmation is asked. Files with a private key are
  refused. When encrypting, `--trust-model always` uses the key the user
  chose: the web of trust is not used. A valid signature shows the signer
  and the full fingerprint, and says that checking the key belongs to
  them is up to the user. A decryption without integrity protection
  (no MDC or AEAD) is refused, and the partial output deleted.
- OpenPGP signing is not offered in the interface: minisign signs, and
  Stable and Advanced accounts can still use `gpg --detach-sign`.

### Independent review

A security review of the first complete version found eight problems,
all fixed:

1. **A clear-signed text next to a file of the same name.** Checking
   `evil.iso.asc` (clear-signed announcement) would have said VALID while
   covering nothing in `evil.iso`; and the clear-signed path itself was
   broken (`--output` after the file). Now such a signature beside a
   same-named file is refused ("does NOT cover any other file"), and a
   standalone clear-signed or inline-signed text is shown exactly as
   signed, under its verdict. gpg's "not a detached signature" warning is
   a refusal.
2. **gpg options through the name of a new key** (`--status-file=...`
   could overwrite a file). Names must start with a letter or digit, and
   the user id follows `--`. Key files are passed as absolute paths.
3. **Ctrl-C left partial plaintext.** A handler (SIGINT, SIGTERM,
   SIGHUP) deletes the partial file before exiting. Partial files of dead
   processes are removed at the next start.
4. **"What is this file?" mixed a signed file's content with gpg's status
   lines**, which allowed fake recipients. It now runs only on encrypted
   messages, with output to `/dev/null`.
5. **Unbounded memory when reading a compressed message.** At most 1 MiB
   is kept; gpg is stopped beyond that.
6. **A crafted armored age file could crash the recogniser** (cut inside a
   multi-byte character). Only base64 characters are kept before cutting.
7. **Escape sequences in a PEM label** were printed raw. They are now
   filtered like every other displayed name.
8. **Wording.** Decryption now says that integrity does not prove who
   wrote the file; minisign accepts an empty password, so key creation
   warns against it.

### Honest results

- A valid signature "proves that the file was not changed since that key
  signed it. It does not prove that the file is safe, nor that the key
  really belongs to that person."
- A checksum match is "only as trustworthy as the place the expected
  checksum came from". MD5 and SHA-1 matches carry a warning that they only
  detect accidental damage.
- After encrypting: "The original file is unchanged... Deleting it may not
  erase it completely from USB sticks and SSDs."
- Inspect describes weak formats as such (ZipCrypto, `openssl enc`
  without authentication). For unknown data it says that "random-looking
  data alone does not prove that a file is encrypted securely".

### In the interface

Security & cryptography (every mode) lists:

- Encrypt a file;
- Decrypt a file;
- Encrypted messages;
- Sign a file;
- Check a signature;
- Checksums;
- What is this file?;
- Random passwords & keys;
- Keys & contacts.

Files are chosen with Find & view files used as a picker: a title says what
the file is for, and the query is pre-filled (`.age`, `.minisig`...). "Encrypt
for" is a checklist: a passphrase, or me and/or contacts. A passphrase
cannot be combined with keys (age's rule), and ticking one unticks the
other. Random passwords appear once in a dialog, never in the notification
history or the log.

## Alternatives considered

- **GnuPG for everything.** One tool for keys, encryption and signatures,
  and interoperable. But its model (keyrings, trust database, agent, many
  algorithms and options) is hard to present safely to beginners, and its
  long keys and fingerprints are hard to check by voice. It is kept for
  interoperability, behind the same guided screens.
- **GnuPG's web of trust** (signing others' keys, owner trust): a
  powerful model, but hard to explain, and it needs key exchanges that an
  offline group does in person anyway. The user's own fingerprint check
  replaces it.
- **The interface asks for passphrases and passes them on (stdin or a file
  descriptor).** Keeping the secret out of our process entirely is
  stronger and simpler. age and minisign already have good terminal
  prompts.
- **Unencrypted age identity files** (age-keygen's default). They are
  simpler, but a copied home folder or backup would expose the key. The
  passphrase costs one prompt per use.
- **Post-quantum age keys** (`age-keygen -pq`, hybrid ML-KEM-768 + X25519,
  available in age 1.3). They are about 2,000 characters long, cannot be
  read aloud or typed, and are new. X25519 stays the default; files from
  others using PQ keys are recognised.
- **Our own random password generator** (getrandom plus a character table)
  would be trivial, but established tools were requested.
- **Encrypted USB sticks (LUKS).** Formatting needs root or a
  udisks/polkit path that takes the passphrase in D-Bus arguments.
  Unlocking existing LUKS sticks is a better fit for the media screens
  and is left for later. Inspect recognises LUKS images.

## Consequences

- Every account can protect files and messages and check software
  signatures without a shell, a privilege or a secret ever seen by our
  code.
- A forgotten passphrase is final, by design, and every screen that creates
  a secret says so.
- Encrypted files and the original coexist: secure deletion is not offered
  (flash storage makes it unreliable); the user is told.
- Contacts are per account. There is no web of trust and no key server
  (offline-first).
- Not tested: physical hardware; very large files (age streams, so memory
  stays flat, but the time was not measured); file systems without
  `RENAME_NOREPLACE` (handled by a fallback, not tested in a VM).

Tested by: `tests/vm/crypto.nix`, unit tests in `crypto.rs`, TUI tests in
`tests.rs`, and an end-to-end host run of `resilience-crypto` with two
accounts (see docs/TESTING.md).
