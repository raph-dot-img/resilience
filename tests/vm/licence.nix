# The licence notice (ADR 0024), driven through the real TUI. Alice
# (Ultra-Stable, logged in automatically on tty1) sees the summary and the
# original text before anything else, cannot reach the menu, ticks the box
# and confirms: the helper appends a timestamped line to the root-owned
# record (and the audit log), and the next session opens on the menu.
# Carol declines on tty2: logged out, nothing recorded. The helper refuses
# any other text's hash and callers that are not root.
{ self }:
{
  name = "resilience-licence";

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
      # This test is about the notice: on, as on every real system.
      resilience.licence.prompt = true;
      resilience.users = {
        alice.mode = "ultra-stable";
        carol.mode = "advanced";
      };
      resilience.interface.autologinUser = "alice";
      users.users.carol.initialPassword = "carol-pw";
    };

  testScript = ''
    import base64, re, time as _time

    def tty_text(tty):
        """Screen text from /dev/vcsuN (UTF-32); see modes.nix."""
        cols = int(machine.wait_until_succeeds(f"stty -F /dev/tty{tty} size", timeout=10).split()[1])
        raw = base64.b64decode(machine.succeed(f"base64 -w0 /dev/vcsu{tty}"))
        text = raw.decode("utf-32-le", errors="replace")
        return "\n".join(text[i:i + cols] for i in range(0, len(text), cols))

    def wait_tty(tty, regexp, timeout=60):
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

    RECORD = "/var/lib/resilience/licence-acceptances.log"

    def audit():
        return machine.succeed("cat /var/log/resilience/audit.log 2>/dev/null || true")

    machine.start()
    machine.wait_for_unit("multi-user.target")
    machine.succeed("dmesg -n 1")
    helper = machine.succeed("sed -n 's/^helper = \"\\(.*\\)\"$/\\1/p' /etc/resilience/admin.toml").strip()
    sha = machine.succeed("sed -n '/^\\[licence\\]/,/^\\[/s/^sha256 = \"\\(.*\\)\"$/\\1/p' /etc/resilience/system.toml").strip()
    assert re.fullmatch(r"[0-9a-f]{64}", sha), sha
    licence_file = machine.succeed("sed -n '/^\\[licence\\]/,/^\\[/s/^file = \"\\(.*\\)\"$/\\1/p' /etc/resilience/system.toml").strip()

    with subtest("the configured hash is the shown text's, and admin.toml agrees"):
        machine.succeed(f"test \"$(sha256sum {licence_file} | cut -d' ' -f1)\" = {sha}")
        machine.succeed(f"grep -q 'Hippocratic License Version Number: 2.1' {licence_file}")
        machine.succeed(f"grep -q '{sha}' /etc/resilience/admin.toml")
        machine.fail(f"test -e {RECORD}")

    with subtest("first session: the notice, and nothing else"):
        wait_tty("1", "HIPPOCRATIC LICENSE 2.1: SUMMARY")
        wait_tty("1", r"\[ \] I certify that I have read the Hippocratic License 2.1")
        machine.send_chars("/")
        machine.send_key("f1")
        machine.send_key("f5")
        _time.sleep(1)
        screen = tty_text("1")
        assert "MAIN MENU" not in screen and "SEARCH" not in screen and "SUMMARY" in screen, screen
        # Esc asks to log out; the default answer keeps the notice.
        machine.send_key("esc")
        wait_tty("1", "LOG OUT")
        machine.send_key("ret")
        wait_tty("1", "SUMMARY")
        # Enter before reading the text records nothing.
        machine.send_key("ret")
        wait_tty("1", "Read the original text first")
        machine.fail(f"test -e {RECORD}")

    with subtest("polkit: the console session may record without a password"):
        pid = machine.succeed("pgrep -u alice -x resilience").split()[0]
        machine.succeed(f"pkcheck --action-id org.resilience.admin.record-licence --process {pid}")
        info = machine.succeed("pkaction --verbose --action-id org.resilience.admin.record-licence")
        assert re.search(r"implicit any:\s+no\n", info), info
        assert re.search(r"implicit active:\s+yes\n", info), info

    with subtest("helper: root only, this system's text only, audited"):
        rc, out = machine.execute(f"runuser -u alice -- {helper} record-licence {sha} 2>&1")
        assert rc == 77, (rc, out)
        for arg, reason in [
            ("0" * 64, "not the licence text of this system"),
            (sha.upper(), "not a SHA-256 hash"),
            ("abc", "not a SHA-256 hash"),
            ("'../x'", "not a SHA-256 hash"),
        ]:
            rc, out = machine.execute(f"{helper} record-licence {arg} 2>&1")
            assert rc == 2 and reason in out, (arg, rc, out)
        log = audit()
        print(log)
        assert len(re.findall(r"op=record-licence .*result=refused", log)) == 4, log
        machine.fail(f"test -e {RECORD}")

    with subtest("Alice reads the text, ticks the box and confirms"):
        machine.send_key("tab")
        wait_tty("1", "ORIGINAL TEXT")
        wait_tty("1", "Hippocratic License Version Number: 2.1")
        machine.send_key("end")
        machine.send_chars(" ")
        wait_tty("1", r"\[x\] I certify")
        machine.send_key("ret")
        wait_tty("1", "MAIN MENU")
        wait_tty("1", "is recorded")
        rec = machine.succeed(f"cat {RECORD}")
        print(rec)
        assert re.fullmatch(
            rf"time=\d{{4}}-\d\d-\d\dT\d\d:\d\d:\d\dZ user=alice uid=\d+ sha256={sha} "
            r'licence="Hippocratic License" version=2\.1\n',
            rec,
        ), rec
        machine.succeed(f"test \"$(stat -c '%U %a' {RECORD})\" = 'root 644'")
        assert re.search(rf"op=record-licence caller=alice caller_uid=\d+ sha256={sha} result=ok", audit())
        machine.succeed("journalctl -t resilience-admin | grep -q 'op=record-licence.*result=ok'")
        # The account cannot change or erase its record.
        machine.fail(f"runuser -u alice -- sh -c 'echo forged >> {RECORD}'")
        machine.fail(f"runuser -u alice -- rm -f {RECORD}")

    with subtest("the next session opens on the menu"):
        machine.send_chars("q")
        wait_tty("1", "LOG OUT")
        machine.send_chars("y")
        # Logged in again automatically.
        machine.wait_until_succeeds("pgrep -u alice -x resilience", timeout=60)
        wait_tty("1", "MAIN MENU", timeout=60)
        assert "I certify" not in tty_text("1")

    with subtest("Carol declines on tty2: logged out, nothing recorded"):
        machine.send_key("alt-f2")
        wait_tty("2", "login: ")
        machine.send_chars("carol\n")
        wait_tty("2", "Password: ")
        machine.send_chars("carol-pw\n")
        wait_tty("2", "HIPPOCRATIC LICENSE 2.1: SUMMARY")
        machine.send_chars("q")
        wait_tty("2", "LOG OUT")
        machine.send_chars("y")
        wait_tty("2", "login: ")
        machine.fail(f"grep -q user=carol {RECORD}")
        assert machine.succeed(f"wc -l < {RECORD}").strip() == "1"
  '';
}
