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
    def wait_tty(tty, regexp, timeout=60):
        """wait_until_tty_matches that shows the screen when it fails."""
        try:
            machine.wait_until_tty_matches(tty, regexp, timeout=timeout)
        except Exception:
            print(f"--- tty{tty} contents ---")
            print(machine.get_tty_text(tty))
            raise

    import time

    machine.start()
    machine.wait_for_unit("multi-user.target")
    print(machine.succeed("ip -br link; ip route; ip -6 route"))

    with subtest("system reaches a healthy state with no network"):
        state = machine.succeed("systemctl is-system-running --wait || true").strip()
        print("system state:", state)
        machine.succeed("systemctl list-units --state=failed --plain --no-legend | tee /dev/stderr | (! grep -q .)")

    with subtest("TUI starts on tty1 and reports offline"):
        wait_tty("1", "MAIN MENU")
        wait_tty("1", "OFFLINE")
        wait_tty("1", "ULTRA-STABLE")
        machine.screenshot("home")

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

    with subtest("no unexpected listening sockets"):
        out = machine.succeed("ss -H -tuln")
        print(out)
        assert out.strip() == "", "unexpected listening sockets:\n" + out
  '';
}
