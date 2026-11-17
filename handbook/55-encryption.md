# Encryption, signatures and checksums

WHY
  Encryption keeps a file or a message secret: only someone with the right
  passphrase or key can read it, and any change to it is detected.
  A signature proves who made a file and that it was not changed since.
  A checksum tells whether a file is exactly the one you were promised.

WHERE: SECURITY & CRYPTOGRAPHY
  Every account can use all of it. Passphrases are always typed into the
  encryption program itself, on a plain screen, and never into a menu of
  the interface: they are not shown, saved or logged.

CHOOSING A FILE
  A search box opens: type part of the file's name (your home folder and
  open USB sticks are searched), Enter chooses the file, Esc cancels.

YOUR KEYS (do this once): Keys & contacts, then Enter
  You get two keys, each protected by a passphrase that you choose:
    - an ENCRYPTION key: people encrypt files and messages for you with it;
    - a SIGNING key: you sign files with it, people check your signatures.
  IF YOU FORGET A PASSPHRASE, NOTHING ENCRYPTED FOR YOU CAN EVER BE OPENED
  AGAIN. Nobody can recover it. Write the passphrases on paper, kept safe.
  The keys are in your home folder, so "Backups & USB sticks" saves them
  (encrypted) with the rest of your files.

GIVING YOUR PUBLIC KEYS, ADDING CONTACTS
  Public keys contain no secret. In Keys & contacts:
  e       save your public keys on a USB stick (a small text file)
  a       add a contact from the key file they gave you
  t       add a contact by typing their key (age1... and/or RW...)
  Enter   details of a key or contact; Delete removes a contact
  BEFORE TRUSTING A CONTACT'S KEYS, CHECK THEM WITH THE PERSON, in person
  or by phone: read aloud the end of the encryption key (shown as "ends
  with ...ab12 cd34") and the signing key number. A key sent by message
  could have been replaced on the way.

ENCRYPT A FILE
  Choose the file, then who can open it (Space ticks, Enter goes on):
    - a passphrase: give it to the reader by another way than the file;
    - or yourself ("Me") and/or contacts: they open it with their key.
  The encrypted copy is saved next to the file as NAME.age. THE ORIGINAL IS
  NOT DELETED: anyone who can open it can still read it. Deleting a file
  may not erase it completely from USB sticks and SSDs.

DECRYPT A FILE
  Choose the .age file and type its passphrase, or your key's passphrase.
  The result goes into your Decrypted folder (never over another file).
  A wrong passphrase, or a file changed or damaged on the way, is refused:
  nothing is written.

ENCRYPTED MESSAGES
  Write: choose who can read it, type the text, finish with a line with only
  a dot (.). It is saved in your Messages folder as text that can be copied,
  printed or sent, and shown on screen.
  Read: choose the message file. It is shown only (q closes it) and never
  saved decrypted.

SIGN A FILE / CHECK A SIGNATURE
  Signing saves NAME.minisig next to the file: give both together.
  Checking says:
    VALID SIGNATURE   signed by that contact (or you) and not changed since;
    INVALID           changed after signing, or a damaged signature: do not
                      trust the file;
    NOT CHECKED       signed by a key that is not one of your contacts.
  A valid signature does NOT prove a file is safe, only who signed it.

CHECKSUMS
  Show a file's checksums (SHA-256, SHA-512), compare a file with the
  checksum published with it (paste or type it), or check every file named
  in a list such as SHA256SUMS (the files must be in the same folder).
  "MATCH" is only as trustworthy as the place you got the checksum from.
  MD5 and SHA-1 only detect accidents, not deliberate changes.

WHAT IS THIS FILE?
  Tells if a file is encrypted (age, OpenPGP/GnuPG, LUKS, ZIP, 7-Zip,
  OpenSSL), a signature, a public key or a PRIVATE key (never share those),
  a checksum list, or plain text. Some formats are described as weak.

RANDOM PASSWORDS & KEYS
  Passphrases of 6 words (English or French), passwords of 24 characters,
  and 256-bit keys, from the system's secure random generator. They are
  shown once and saved nowhere: write them down now.

OPENPGP (GnuPG): WITH PEOPLE WHO USE IT
  Many people and software projects use OpenPGP (GnuPG, "PGP", e-mail
  encryption). Resilience recognises its files and keys by itself:
    - an OpenPGP key file (often .asc): add it with a in Keys & contacts;
      its FINGERPRINT is shown: compare it with the one its owner gives you
      or publishes (website, paper) before saying yes;
    - a signature NAME.sig or NAME.asc next to a file: "Check a signature";
    - an OpenPGP encrypted file (.gpg, .asc): "Decrypt a file" or "Read an
      encrypted message".
  To receive OpenPGP files, create your own OpenPGP key with g in Keys &
  contacts (a name, and an e-mail address if you want: others will see
  them). GnuPG asks its passphrase in a box on the screen, every time. When
  it asks for a new passphrase twice in the same box, press Tab to go to the
  second line ("Repeat"), then Enter. On a slow computer it may then ask
  once more, to unlock the new key: type the same passphrase.
  Encrypt for: OpenPGP keys appear as "OpenPGP: name". One file is for one
  kind of key: a passphrase, OR Resilience keys, OR OpenPGP keys.
  GnuPG never looks for keys on the Internet here.

FOR STABLE AND ADVANCED ACCOUNTS
  The same tools exist on the command line: age, age-keygen, minisign, gpg,
  sha256sum... and resilience-crypto (resilience-crypto --help).
