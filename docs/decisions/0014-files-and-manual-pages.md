# ADR 0014: Finding and viewing files; manual pages

Status: accepted (2026-11-14).

## Context

Phase 3 asks for file search, document viewing (including text from PDF
files), a basic CSV tool, archive handling, checksums and local
documentation. All of them must work for Ultra-Stable accounts, which have
no shell and must not get one through a viewer. Several viewers people use
have escapes: `less` (`!`, `v`, `|`, `:e`, LESSOPEN), Midnight Commander,
VisiData (Python expressions).

## Decision

### Find & view files (Productivity)

A screen built into the interface, read-only:

- **Search by name.** When the screen opens, it indexes the home folder and
  every open USB stick: regular files only, at most 100,000 of them. It
  skips hidden entries, never follows symbolic links and stays on each
  root's file system. Typed words filter the index: all must appear in the
  path, in any case. F2 indexes again.
- **Viewing inside the interface** (`resilience_core::files`):
  - **Text:** read directly, at most 2 MiB, with control characters
    replaced so nothing can move the cursor or change the terminal.
  - **CSV/TSV:** a table with quoted fields, at most 20,000 rows; the
    arrows move between rows and columns.
  - **PDF:** text from `pdftotext -layout` (poppler), at most 500 pages and
    60 s, then shown like text. A PDF without text (scanned) says so.
  - **Archives** (zip, tar.*, 7z, iso, …): **listed** with `bsdtar -tvf`
    (libarchive), never extracted.
  - **ZIM files:** opened in the library reader (ADR 0013).
  - **Anything else:** details only.
- **F3, details:** size, date and SHA-256 (`sha256sum`), to compare with
  a published fingerprint.

The external programs are absolute store paths from `system.toml`, run from
a fixed argv with a timeout. They parse untrusted files as the user, like
any document viewer.

### Manual pages (Knowledge)

A dialog asks for a name (`ls`, `5 passwd`) or a few words.
`resilience_core::manpage` accepts only words made of letters, digits and
`. _ + : -`, with no leading `-`. So nothing typed becomes an option of
`man`, and the TUI never uses a shell.

- **A name:** `man -w` first checks that the page exists. Then `man NAME`
  runs in the foreground with `MANPAGER` and `PAGER` set to `less`, and
  `LESSSECURE=1`. Secure mode disables shell commands, the editor, pipes,
  log files, opening other files and input preprocessors. `LESSOPEN`,
  `LESSCLOSE`, `MANOPT`, `BROWSER`, `HTMLPAGER` and `EDITOR`/`VISUAL` are
  emptied.
- **Several words:** `apropos --and` lists the pages whose name or
  description contains every word, and the list is shown in the interface.
  The VM test showed that `man -k` matches any one of the words, which
  buries the right page. This needs the whatis database, so
  `documentation.man.cache.enable = true`.

### After an independent review

- **Large output stalled.** `proc::run_full` used to read a program's
  output only after it exited, so a long PDF, a big archive listing or a
  broad search blocked on the 64 KiB pipe until the timeout, then looked
  like a failure. Both pipes are now read while the program runs, at most
  8 MiB each, and decoded lossily. A unit test covers output above 64 KiB.
- Table column widths are computed once, and rows are capped at 500
  columns, so a hostile CSV cannot freeze the screen.
- Enter on an unknown file shows its details without hashing it. F3
  computes the SHA-256: on the spot up to 256 MiB, otherwise in the
  foreground with Ctrl-C available.
- Folders count towards the index limit.
- Files are opened with `O_NOFOLLOW|O_NONBLOCK`, and only regular files are
  read, so a FIFO or link swapped in cannot hang or redirect the viewer.
- **`man` starts from an empty environment** (`RunRequest.clean_env`):
  `TERM`, `LANG`, `HOME`, `USER`, a fixed `PATH`, then the variables above.
  `GROFF_*`, `MANPATH`, `LESS`, `LESSSECURE_ALLOW` and the like cannot
  change what runs. Ultra-Stable users cannot set their environment today;
  this is defence in depth.
- `resilience-library` removes control characters from the file names it
  prints. A `.zim` file opened from Find & view files that is neither
  installed, built in nor a catalog file shows a warning first.
- **Not addressed** (known gap): no memory or CPU limit on `pdftotext` and
  `bsdtar` beyond their timeouts.

## Alternatives considered

- **Midnight Commander's viewer, or `less` for files:** both have escapes,
  and mc is a file manager (Stable and up only).
- **VisiData for CSV:** powerful, but runs Python expressions; it can be
  added as a Stable application later.
- **A full-text file search** (contents, not only names): needs an index
  that must be kept current and private. File names cover the common need.
- **Extracting archives:** writes many files from an untrusted source. It
  can be added later for Stable accounts with a destination folder that is
  always new, like restores (ADR 0008).

## Consequences

- Every account can find, read and check files without any program that can
  start a shell.
- `pdftotext` and `libarchive` parse untrusted files as the user. A flaw in
  them reaches only what the user can already reach.
- The whatis database is built with the system (a few seconds of build).

Tested by: `tests/vm/documents.nix` (real PDF, zip, CSV, text, SHA-256, a
man page with `!` refused, a search of the manuals), unit tests in
`files.rs` and `manpage.rs`, and TUI tests.
