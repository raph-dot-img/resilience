# Boot with NO network hardware at all and use the interface.
# Covers offline scenarios 1, 4, 5 and 14 of docs/TESTING.md.
{ self }:
{
  name = "resilience-boot-offline";

  nodes.machine =
    { lib, ... }:
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/standard.nix
      ];
      # No network hardware at all: drop the test framework's default slirp
      # NIC and stop QEMU from adding its own default NIC.
      virtualisation.vlans = [ ];
      virtualisation.qemu.networkingOptions = lib.mkForce [ "-nic none" ];
      virtualisation.memorySize = 1536;
      # The test instrumentation sets an empty root password for debugging;
      # drop our lock in the test VM only, to avoid conflicting definitions.
      users.users.root.hashedPassword = lib.mkForce null;
      virtualisation.cores = 2;
      resilience.users.alice.mode = "ultra-stable";
      users.users.alice.hashedPassword = "!";
      resilience.interface.autologinUser = "alice";
    };

  testScript = ''
    import base64, re, time as _time

    def tty_text(tty):
        """Screen text from /dev/vcsuN (UTF-32), decoded on the host.

        The driver's get_tty_text() reads /dev/vcsN, which holds 8-bit glyph
        indices; with a CP437-ordered console font the box-drawing indices
        are invalid UTF-8 and corrupt neighbouring characters.
        """
        cols = int(machine.succeed(f"stty -F /dev/tty{tty} size").split()[1])
        raw = base64.b64decode(machine.succeed(f"base64 -w0 /dev/vcsu{tty}"))
        text = raw.decode("utf-32-le", errors="replace")
        return "\n".join(text[i:i + cols] for i in range(0, len(text), cols))

    def wait_tty(tty, regexp, timeout=60):
        """Wait until the screen of ttyN matches, showing it on failure."""
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

    import time

    machine.start()
    machine.wait_for_unit("multi-user.target")
    print(machine.succeed("ip -br link; ip route; ip -6 route"))

    with subtest("system reaches a healthy state with no network"):
        state = machine.succeed("systemctl is-system-running --wait || true").strip()
        print("system state:", state)
        machine.succeed("systemctl list-units --state=failed --plain --no-legend | tee /dev/stderr | (! grep -q .)")

    with subtest("Ultra-Stable-only machine: no user namespaces for anyone (ADR 0012)"):
        machine.succeed("test \"$(cat /proc/sys/user/max_user_namespaces)\" = 0")
        rc, out = machine.execute("runuser -u alice -- unshare --user true 2>&1")
        assert rc != 0 and "No space left on device" in out, (rc, out)
        machine.fail("unshare --user true")
        # Nothing on the system needed them: no failed unit (checked above).
        # ...and no account can be raised at runtime (no Nix sandbox here).
        machine.succeed("grep -qx 'ultra_stable_only = true' /etc/resilience/admin.toml")
        helper = machine.succeed("sed -n 's/^helper = \"\\(.*\\)\"$/\\1/p' /etc/resilience/admin.toml").strip()
        rc, out = machine.execute(f"{helper} set-mode alice stable 2>&1")
        assert rc == 2 and "Ultra-Stable accounts only" in out, (rc, out)

    with subtest("TUI starts on tty1 and reports offline"):
        wait_tty("1", "MAIN MENU")
        wait_tty("1", "OFFLINE")
        wait_tty("1", "ULTRA-STABLE")
        machine.screenshot("home")

    with subtest("autologin is limited to tty1; tty2 offers a login prompt"):
        machine.send_key("alt-f2")
        wait_tty("2", "login: ")
        print(machine.succeed("loginctl list-sessions --no-legend"))
        machine.fail("loginctl list-sessions --no-legend | grep -qw tty2")
        machine.send_key("alt-f1")
        wait_tty("1", "MAIN MENU")

    with subtest("status CLI agrees"):
        out = machine.succeed("runuser -u alice -- resilience status")
        print(out)
        assert "mode: ultra-stable" in out, out
        assert "network: offline" in out, out
        assert "warning" not in out, out

    with subtest("catalog: every visible program is available"):
        out = machine.succeed("runuser -u alice -- resilience apps")
        print(out)
        assert "unavailable" not in out, out

    with subtest("handbook opens offline"):
        machine.send_chars("1")
        wait_tty("1", "Resilience Handbook")
        machine.send_key("ret")
        wait_tty("1", "First steps")
        machine.send_key("ret")
        wait_tty("1", "EVERYTHING HERE WORKS OFFLINE")
        machine.send_key("esc")
        machine.send_key("esc")
        machine.send_key("esc")
        wait_tty("1", "MAIN MENU")

    with subtest("network probe is skipped instantly without a route"):
        machine.send_chars("/diagnostics")
        wait_tty("1", "Network status")
        machine.send_key("ret")
        wait_tty("1", "NETWORK STATUS & DIAGNOSTICS")
        start = time.monotonic()
        machine.send_chars("p")
        wait_tty("1", "not attempted")
        elapsed = time.monotonic() - start
        print(f"probe result shown after {elapsed:.1f}s")
        machine.send_key("esc")
        machine.send_key("esc")

    with subtest("a program launches and returns to the interface"):
        machine.send_chars("/months")
        wait_tty("1", "Calendar")
        machine.send_key("ret")
        wait_tty("1", "Press Enter to return")
        machine.send_key("ret")
        wait_tty("1", "RESULTS")
        machine.send_key("esc")
        wait_tty("1", "MAIN MENU")

    with subtest("Ctrl-C does not leave the interface"):
        machine.send_key("ctrl-c")
        time.sleep(1)
        wait_tty("1", "MAIN MENU")
        machine.fail("pgrep -u alice -x bash")

    with subtest("interface crash is recovered by the session wrapper"):
        # SIGABRT: Rust handles SIGSEGV itself (stack-overflow guard).
        machine.succeed("pkill -ABRT -u alice -x resilience")
        wait_tty("1", "STOPPED UNEXPECTEDLY")
        machine.send_key("ret")
        wait_tty("1", "MAIN MENU")

    with subtest("measure resource usage (reported, not asserted)"):
        print("MEASURE tui_rss_kib", machine.succeed("ps -o rss= -C resilience | head -1").strip())
        print("MEASURE mem_used_mib", machine.succeed("free -m | awk '/^Mem:/ {print $3}'").strip())
        print("MEASURE boot_userspace", machine.succeed("systemd-analyze 2>/dev/null | head -1 || true").strip())
        print("MEASURE running_services", machine.succeed("systemctl list-units --type=service --state=running --plain --no-legend | wc -l").strip())
        print(machine.succeed("systemctl list-units --type=service --state=running --plain --no-legend | awk '{print $1}'"))

    with subtest("no unexpected listening sockets"):
        out = machine.succeed("ss -H -tuln")
        print(out)
        assert out.strip() == "", "unexpected listening sockets:\n" + out
  '';
}
