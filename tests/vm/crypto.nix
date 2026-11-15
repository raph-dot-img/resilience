# Cryptography (Phase 4, ADR 0015), driven through the real TUI on tty1 by
# an Ultra-Stable account. Passphrases are typed into age and minisign
# themselves. Checks: keys created and stored encrypted (no plaintext secret
# key anywhere), a file encrypted for "me" and with a passphrase, decrypted
# into ~/Decrypted (a wrong passphrase leaves nothing), a file signed and
# its signature checked (then refused after a change), a checksum compared,
# an encrypted message written and read back without being saved in clear,
# a random passphrase shown; finally, no passphrase in the journal, in /tmp
# or in the home folder.
{ self }:
{ ... }:
{
  name = "resilience-crypto";

  nodes.machine =
    { lib, ... }:
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      virtualisation.memorySize = 1536;
      virtualisation.cores = 2;
      virtualisation.qemu.networkingOptions = lib.mkForce [ "-nic none" ];
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users.alice.mode = "ultra-stable";
      resilience.interface.autologinUser = "alice";
    };

  testScript = ''
    import base64, re, time as _time

    KEY_PASS = "tiger lamp river 42"
    SIGN_PASS = "sign pw seventy7"
    FILE_PASS = "boat cloud nine"
    SECRET_TEXT = "meet at the old bridge"
    MESSAGE = "hello from the shelter"

    def tty_text(tty):
        """Screen text from /dev/vcsuN (UTF-32); see modes.nix."""
        cols = int(machine.succeed(f"stty -F /dev/tty{tty} size").split()[1])
        raw = base64.b64decode(machine.succeed(f"base64 -w0 /dev/vcsu{tty}"))
        text = raw.decode("utf-32-le", errors="replace")
        return "\n".join(text[i:i + cols] for i in range(0, len(text), cols))

    def wait_tty(tty, regexp, timeout=90):
        deadline = _time.monotonic() + timeout
        while True:
            text = tty_text(tty)
            if re.search(regexp, text):
                return text
            if _time.monotonic() > deadline:
                print(f"--- tty{tty} contents ---")
                print(text)
                raise Exception(f"timed out waiting for {regexp!r} on tty{tty}")
            _time.sleep(0.5)

    def home():
        for _ in range(5):
            machine.send_key("esc")
        wait_tty("1", "MAIN MENU")

    def security(entry, title):
        home()
        machine.send_chars("3")
        wait_tty("1", "SECURITY & CRYPTOGRAPHY")
        machine.send_chars(entry)
        wait_tty("1", title)

    def pick(name, count=1):
        """In the file picker: replace the query with `name`, choose the file."""
        for _ in range(16):
            machine.send_key("backspace")
        machine.send_chars(name)
        wait_tty("1", rf"FILES \({count}\)")
        machine.send_key("ret")

    def type_secret(prompt, secret):
        wait_tty("1", prompt)
        machine.send_chars(secret + "\n")

    def back_to_interface():
        wait_tty("1", "Press Enter to return to Resilience")
        machine.send_key("ret")
        wait_tty("1", "MODE: ULTRA-STABLE")

    machine.start()
    machine.wait_for_unit("multi-user.target")
    machine.succeed("dmesg -n 1")
    wait_tty("1", "MAIN MENU")
    machine.succeed(
        f"printf '{SECRET_TEXT}\\n' > /home/alice/plan.txt",
        "printf 'field notes\\n' > /home/alice/notes.txt",
        "printf 'water filter v2\\n' > /home/alice/report.txt",
        "printf 'disk image\\n' > /home/alice/image.iso",
        "chown alice: /home/alice/plan.txt /home/alice/notes.txt /home/alice/report.txt /home/alice/image.iso",
    )
    keys = "/home/alice/.local/share/resilience/keys"

    with subtest("keys: created through the interface, stored encrypted"):
        security("9", "KEYS & CONTACTS")
        wait_tty("1", "not created yet")
        machine.send_key("ret")
        type_secret("Enter passphrase", KEY_PASS)
        type_secret("Confirm passphrase", KEY_PASS)
        wait_tty("1", "SIGNING KEY")
        type_secret("Password:", SIGN_PASS)
        type_secret("one more time", SIGN_PASS)
        wait_tty("1", "Key number")
        back_to_interface()
        wait_tty("1", "ends with")
        wait_tty("1", "minisign, number [0-9A-F]{16}")
        assert machine.succeed(f"stat -c %a {keys} {keys}/own").split() == ["700", "700"]
        assert machine.succeed(f"stat -c %a {keys}/own/encryption.age {keys}/own/signing.key").split() == ["600", "600"]
        assert machine.succeed(f"stat -c %U {keys}/own/encryption.age").strip() == "alice"
        machine.succeed(f"grep -q 'BEGIN AGE ENCRYPTED FILE' {keys}/own/encryption.age")
        machine.succeed(f"grep -q 'minisign encrypted secret key' {keys}/own/signing.key")
        # The age secret key never touched the disk unencrypted.
        machine.fail("grep -r AGE-SECRET-KEY /home/alice /tmp /var/tmp")

    with subtest("encrypt for me, decrypt with my key; a wrong passphrase leaves nothing"):
        security("1", "CHOOSE THE FILE TO ENCRYPT")
        pick("plan")
        wait_tty("1", "ENCRYPT FOR")
        wait_tty("1", r"\[x\] A passphrase")
        machine.send_key("down")
        machine.send_chars(" ")
        wait_tty("1", r"\[x\] Me")
        machine.send_key("ret")
        wait_tty("1", "Encrypted copy")
        back_to_interface()
        machine.succeed("head -c 22 /home/alice/plan.txt.age | grep -q age-encryption.org/v1")
        machine.fail(f"grep -q '{SECRET_TEXT}' /home/alice/plan.txt.age")

        security("2", "CHOOSE THE ENCRYPTED FILE")
        pick(".age")
        type_secret("passphrase for identity file", KEY_PASS)
        wait_tty("1", "Decrypted into")
        back_to_interface()
        machine.succeed("cmp /home/alice/plan.txt /home/alice/Decrypted/plan.txt")
        assert machine.succeed("stat -c %a /home/alice/Decrypted /home/alice/Decrypted/plan.txt").split() == ["700", "600"]

        security("2", "CHOOSE THE ENCRYPTED FILE")
        pick(".age")
        type_secret("passphrase for identity file", "not the passphrase")
        wait_tty("1", "not the right one")
        back_to_interface()
        assert machine.succeed("ls -A /home/alice/Decrypted").split() == ["plan.txt"]

    with subtest("encrypt with a passphrase, decrypt with it"):
        security("1", "CHOOSE THE FILE TO ENCRYPT")
        pick("notes")
        wait_tty("1", "ENCRYPT FOR")
        machine.send_key("ret")
        type_secret("Enter passphrase", FILE_PASS)
        type_secret("Confirm passphrase", FILE_PASS)
        wait_tty("1", "Encrypted copy")
        back_to_interface()
        security("2", "CHOOSE THE ENCRYPTED FILE")
        pick("notes.txt.age")
        wait_tty("1", "passphrase of this file")
        type_secret("Enter passphrase", FILE_PASS)
        wait_tty("1", "Decrypted into")
        back_to_interface()
        machine.succeed("cmp /home/alice/notes.txt /home/alice/Decrypted/notes.txt")

    with subtest("inspect recognises an age file"):
        security("7", "CHOOSE THE FILE TO INSPECT")
        pick("notes.txt.age")
        wait_tty("1", "An age encrypted file")
        wait_tty("1", "opens with a passphrase")
        back_to_interface()

    with subtest("sign, check the signature, refuse it after a change"):
        security("4", "CHOOSE THE FILE TO SIGN")
        pick("report")
        type_secret("Password:", SIGN_PASS)
        wait_tty("1", "Signature saved")
        back_to_interface()
        machine.succeed("test -s /home/alice/report.txt.minisig")
        security("5", "CHOOSE THE SIGNED FILE")
        pick(".minisig")
        wait_tty("1", "VALID SIGNATURE")
        wait_tty("1", "your own key")
        back_to_interface()
        machine.succeed("echo changed >> /home/alice/report.txt")
        security("5", "CHOOSE THE SIGNED FILE")
        pick(".minisig")
        wait_tty("1", "INVALID SIGNATURE")
        back_to_interface()

    with subtest("checksum: compared with the expected one"):
        want = machine.succeed("sha256sum /home/alice/image.iso").split()[0]
        security("6", "CHECKSUMS")
        machine.send_key("down")
        machine.send_key("ret")
        wait_tty("1", "CHOOSE THE FILE TO CHECK")
        pick("image.iso")
        wait_tty("1", "COMPARE WITH A CHECKSUM")
        machine.send_chars(want + "\n")
        wait_tty("1", "MATCH")
        assert "DOES NOT" not in tty_text("1")
        back_to_interface()

    with subtest("an encrypted message, written and read back, never saved in clear"):
        security("3", "ENCRYPTED MESSAGES")
        machine.send_key("ret")
        wait_tty("1", "ENCRYPT FOR")
        machine.send_key("down")
        machine.send_chars(" ")
        machine.send_key("ret")
        wait_tty("1", "Type your message")
        machine.send_chars(MESSAGE + "\n.\n")
        wait_tty("1", "BEGIN AGE ENCRYPTED FILE")
        back_to_interface()
        msg = machine.succeed("ls /home/alice/Messages").strip()
        assert msg.startswith("message-") and msg.endswith(".age.txt"), msg
        machine.fail(f"grep -rq '{MESSAGE}' /home/alice/Messages")
        machine.send_key("esc")
        wait_tty("1", "ENCRYPTED MESSAGES|SECURITY")
        security("3", "ENCRYPTED MESSAGES")
        machine.send_key("down")
        machine.send_key("ret")
        wait_tty("1", "CHOOSE THE ENCRYPTED MESSAGE")
        pick("message-")
        type_secret("passphrase for identity file", KEY_PASS)
        wait_tty("1", MESSAGE)
        wait_tty("1", "not saved")
        machine.send_chars("q")
        back_to_interface()

    with subtest("a random passphrase is shown, not stored"):
        security("8", "RANDOM PASSWORDS")
        machine.send_key("ret")
        text = wait_tty("1", "about [0-9]+ bits")
        machine.send_key("ret")

    with subtest("no passphrase anywhere: journal, home, temporary folders"):
        # Patterns come from a file, so no secret is in grep's own command
        # line (which could otherwise be what matches). The decrypted message
        # was only shown, never saved.
        secrets = "\\n".join([KEY_PASS, SIGN_PASS, FILE_PASS, MESSAGE])
        machine.succeed(f"printf '{secrets}\\n' > /root/secrets", "journalctl -b --no-pager > /root/journal")
        machine.fail("grep -F -f /root/secrets /root/journal")
        machine.fail("grep -rF -f /root/secrets /home/alice /tmp /var/tmp")
  '';
}
