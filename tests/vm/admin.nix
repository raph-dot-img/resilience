# The privileged helper `resilience-admin` (ADR 0007): typed operations,
# polkit authorisation with an administrator password, server-side
# validation, audit log, persistence across reboots, session termination on
# demotion, and PAM faillock. Driven through the real TUI on tty1.
{ self }:
{
  name = "resilience-admin";

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
      # The test instrumentation gives root an empty password; see modes.nix.
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users = {
        alice.mode = "ultra-stable";
        bob.mode = "stable";
        carol = {
          mode = "advanced";
          description = "Carol";
        };
      };
      resilience.interface.autologinUser = "alice";
      # Test-only passwords for a throw-away VM.
      users.users = {
        alice.initialPassword = "alice-pw";
        bob.initialPassword = "bob-pw";
        carol.initialPassword = "carol-pw";
      };
    };

  testScript = ''
    import base64, re, time as _time

    def tty_text(tty):
        """Screen text from /dev/vcsuN (UTF-32); see modes.nix."""
        cols = int(machine.succeed(f"stty -F /dev/tty{tty} size").split()[1])
        raw = base64.b64decode(machine.succeed(f"base64 -w0 /dev/vcsu{tty}"))
        text = raw.decode("utf-32-le", errors="replace")
        return "\n".join(text[i:i + cols] for i in range(0, len(text), cols))

    def wait_tty(tty, regexp, timeout=60):
        deadline = _time.monotonic() + timeout
        while True:
            text = tty_text(tty)
            if re.search(regexp, text):
                return
            if _time.monotonic() > deadline:
                print(f"--- tty{tty} contents ---")
                print(text)
                raise Exception(f"timed out waiting for {regexp!r} on tty{tty}")
            _time.sleep(0.5)

    def as_user(user, cmd):
        return f"runuser -u {user} -- sh -c {cmd!r}"

    def groups(user):
        return machine.succeed(f"id -nG {user}").split()

    def audit():
        return machine.succeed("cat /var/log/resilience/audit.log")

    machine.start()
    machine.wait_for_unit("multi-user.target")

    helper = machine.succeed("sed -n 's/^helper = \"\\(.*\\)\"$/\\1/p' /etc/resilience/admin.toml").strip()
    assert helper.startswith("/nix/store/"), helper

    with subtest("helper files are root-owned and private where needed"):
        machine.succeed("test \"$(stat -c '%U:%G %a' /var/log/resilience)\" = 'root:wheel 750'")
        machine.succeed("test \"$(stat -c '%U:%G %a' /var/log/resilience/audit.log)\" = 'root:wheel 640'")
        machine.succeed("test \"$(stat -c '%U' /var/lib/resilience)\" = root")
        machine.fail(as_user("alice", "cat /var/log/resilience/audit.log"))
        machine.fail(as_user("bob", "cat /var/log/resilience/audit.log"))
        machine.succeed(as_user("carol", "cat /var/log/resilience/audit.log"))
        machine.fail(as_user("alice", "touch /var/lib/resilience/modes.toml"))
        machine.succeed("grep -q 'org.resilience.admin.set-mode' /run/current-system/sw/share/polkit-1/actions/org.resilience.admin.policy")

    with subtest("anyone may list modes; nobody may change them without root"):
        out = machine.succeed(as_user("alice", "resilience-admin list"))
        print(out)
        assert re.search(r"bob\s+stable\s+stable\s+stable", out), out
        assert re.search(r"alice\s+ultra-stable.*autologin", out), out
        rc, out = machine.execute(as_user("alice", f"{helper} set-mode bob advanced 2>&1"))
        assert rc == 77, (rc, out)
        rc, out = machine.execute(as_user("carol", f"{helper} set-mode bob advanced 2>&1"))
        assert rc == 77, (rc, out)
        # pkexec without an authentication agent (setsid: no controlling
        # terminal for its text agent): not authorised, nothing runs.
        rc, out = machine.execute("setsid -w " + as_user("alice", f"pkexec {helper} set-mode bob advanced") + " < /dev/null 2>&1")
        print(out)
        assert rc != 0, out
        assert "wheel" not in groups("bob")
        assert "result=ok" not in audit()

    with subtest("server-side validation refuses unsafe or malformed requests"):
        for args, reason in [
            ("root advanced", "not an account managed"),
            ("'bob;id' stable", "invalid user name"),
            ("../carol stable", "invalid user name"),
            ("bob root", "unknown mode"),
            ("bob Advanced", "unknown mode"),
            ("carol stable", "last Advanced account"),
            ("alice stable", "logs in automatically"),
            ("bob stable", "already in STABLE"),
        ]:
            rc, out = machine.execute(f"{helper} set-mode {args} 2>&1")
            assert rc == 2 and reason in out, (args, rc, out)
        rc, out = machine.execute(f"{helper} set-mode bob stable extra 2>&1")
        assert rc == 64, (rc, out)
        log = audit()
        print(log)
        assert log.count("result=refused") == 8, log
        assert "wheel" in groups("carol") and "wheel" not in groups("bob")

    with subtest("TUI: a wrong administrator password changes nothing"):
        wait_tty("1", "MAIN MENU")
        machine.send_chars("/users")
        wait_tty("1", "Users & security modes")
        machine.send_key("ret")
        wait_tty("1", "USERS & SECURITY MODES")
        machine.send_key("down")  # alice, bob, carol: select bob
        machine.send_key("ret")
        wait_tty("1", "NEW MODE FOR 'bob'")
        machine.send_key("home")  # Ultra-Stable
        machine.send_key("ret")
        wait_tty("1", "CHANGE SECURITY MODE")
        wait_tty("1", "all sessions of 'bob'")
        machine.send_chars("y")
        wait_tty("1", "AUTHENTICATING FOR org.resilience.admin.set-mode")
        wait_tty("1", "Password:")
        machine.send_chars("not-the-password\n")
        wait_tty("1", "Press Enter to return", timeout=90)
        assert "resilience-stable" in groups("bob")
        assert "target=bob from=" not in audit()
        machine.send_key("ret")
        wait_tty("1", "USERS & SECURITY MODES")

    with subtest("TUI: Ultra-Stable user promotes bob with the administrator's password"):
        # The list keeps bob selected after the failed attempt.
        machine.send_key("ret")
        wait_tty("1", "NEW MODE FOR 'bob'")
        machine.send_key("end")  # Advanced
        machine.send_key("ret")
        wait_tty("1", "next login of 'bob'")
        machine.send_chars("y")
        wait_tty("1", "AUTHENTICATING FOR org.resilience.admin.set-mode")
        wait_tty("1", "Password:")
        machine.send_chars("carol-pw\n")
        wait_tty("1", "bob is now in ADVANCED mode", timeout=90)
        wait_tty("1", "Press Enter to return")
        assert "wheel" in groups("bob"), groups("bob")
        log = audit()
        print(log)
        assert re.search(r"op=set-mode caller=alice caller_uid=\d+ target=bob from=stable to=advanced result=ok", log), log
        machine.succeed("grep -q 'mode = \"advanced\"' /var/lib/resilience/modes.toml")
        machine.succeed("journalctl -t resilience-admin | grep -q 'target=bob from=stable to=advanced result=ok'")
        machine.send_key("ret")
        wait_tty("1", "USERS & SECURITY MODES")
        wait_tty("1", "bob +ADVANCED")

    with subtest("the new rights apply at bob's next login"):
        machine.send_key("alt-f3")
        wait_tty("3", "login: ")
        machine.send_chars("bob\n")
        wait_tty("3", "Password: ")
        machine.send_chars("bob-pw\n")
        wait_tty("3", "MODE: ADVANCED")
        machine.wait_until_succeeds("loginctl list-sessions --no-legend | grep -w bob | grep -qw tty3")

    with subtest("demotion closes the demoted account's sessions"):
        out = machine.succeed(f"{helper} set-mode bob stable")
        print(out)
        assert "sessions of bob are being closed" in out, out
        machine.wait_until_succeeds("! loginctl list-sessions --no-legend | grep -qw bob", timeout=30)
        wait_tty("3", "login: ")
        assert "wheel" not in groups("bob")
        # Back to the declared mode: no override is kept.
        machine.fail("grep -q '\\[users.bob\\]' /var/lib/resilience/modes.toml")

    with subtest("a recorded mode survives a reboot"):
        machine.succeed(f"{helper} set-mode bob advanced")
        machine.shutdown()
        machine.start()
        machine.wait_for_unit("multi-user.target")
        assert "wheel" in groups("bob"), groups("bob")
        log = audit()
        assert "op=apply-state target=bob to=advanced result=ok" in log, log
        out = machine.succeed("resilience-admin list")
        print(out)
        assert re.search(r"bob\s+stable\s+advanced\s+advanced\s+changed .* by root", out), out

    with subtest("faillock: repeated wrong passwords lock the account for a while"):
        for _ in range(5):
            machine.fail(as_user("carol", "echo wrong | sudo -k -S true"))
        out = machine.fail(as_user("carol", "echo carol-pw | sudo -k -S true 2>&1"))
        print(out)
        out = machine.succeed("faillock --user carol")
        print(out)
        assert re.search(r"\d{4}-\d{2}-\d{2}", out), out
        machine.succeed("faillock --user carol --reset")
        machine.succeed(as_user("carol", "echo carol-pw | sudo -k -S true"))
  '';
}
