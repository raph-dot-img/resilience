# Security modes are real OS privileges, not menu entries.
# Covers the permission tests of docs/TESTING.md, plus LOCAL LINK detection
# on an isolated network (offline scenario 3: no DNS, no route).
{ self }:
{
  name = "resilience-modes";

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
      # One isolated VLAN with a static address and no gateway or DNS.
      virtualisation.vlans = [ 1 ];
      # Drop the framework's slirp NIC (it provides a default route via DHCP).
      virtualisation.qemu.networkingOptions = lib.mkForce [ ];
      resilience.users = {
        alice.mode = "ultra-stable";
        bob.mode = "stable";
        carol.mode = "advanced";
      };
      # Test-only passwords for a throw-away VM.
      users.users = {
        alice.initialPassword = "alice-pw";
        bob.initialPassword = "bob-pw";
        carol.initialPassword = "carol-pw";
      };
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

    machine.start()
    machine.wait_for_unit("multi-user.target")

    def as_user(user, cmd):
        return f"runuser -u {user} -- sh -c {cmd!r}"

    with subtest("modes are reported from real group membership"):
        for user, mode in [("alice", "ultra-stable"), ("bob", "stable"), ("carol", "advanced")]:
            out = machine.succeed(as_user(user, "resilience status"))
            assert f"mode: {mode}" in out, (user, out)

    with subtest("ultra-stable: no sudo, no Nix daemon, no shell via login"):
        machine.fail("id -nG alice | grep -wE 'wheel|resilience-stable|networkmanager'")
        machine.fail(as_user("alice", "sudo -n true"))
        machine.fail(as_user("alice", "echo alice-pw | sudo -S true"))
        out = machine.fail(as_user("alice", "nix store info 2>&1"))
        print(out)
        # The login shell refuses to run commands (su -c / ssh host cmd).
        out = machine.fail("su - alice -c id 2>&1")
        assert "not supported" in out, out
        machine.succeed("getent passwd alice | grep -q resilience-session")

    with subtest("ultra-stable cannot modify the system"):
        machine.fail(as_user("alice", "touch /etc/resilience/x"))
        machine.fail(as_user("alice", "touch /nix/store/x"))
        machine.fail(as_user("alice", "systemctl --no-ask-password stop nix-daemon.socket"))
        machine.fail(as_user("alice", "nmcli --wait 5 networking off"))
        machine.fail(as_user("alice", "cat /home/bob/.bash_history /home/carol/x 2>&1; ls /home/bob"))

    with subtest("stable: Nix daemon yes, sudo no, system configuration no"):
        machine.succeed(as_user("bob", "nix store info"))
        machine.fail(as_user("bob", "sudo -n true"))
        machine.fail(as_user("bob", "echo bob-pw | sudo -S true"))
        machine.fail(as_user("bob", "touch /etc/resilience/x"))
        machine.fail(as_user("bob", "systemctl --no-ask-password stop nix-daemon.socket"))

    with subtest("advanced: sudo works only with the password"):
        machine.fail(as_user("carol", "sudo -n true"))
        machine.succeed(as_user("carol", "echo carol-pw | sudo -S true"))
        machine.fail(as_user("carol", "echo wrong | sudo -S true"))

    with subtest("root has no password login"):
        machine.succeed("getent shadow root | cut -d: -f2 | grep -qx '!'")

    with subtest("isolated LAN is detected as LOCAL LINK, not online"):
        machine.wait_until_succeeds("ip -4 addr show eth1 | grep -q inet")
        out = machine.succeed(as_user("alice", "resilience status"))
        print(out)
        assert "network: local link" in out, out
        assert "gateway: none" in out, out

    with subtest("TUI login as each mode shows the right capabilities"):
        # Ultra-stable on tty2: no terminal shell anywhere.
        machine.send_key("alt-f2")
        wait_tty("2", "login: ")
        machine.send_chars("alice\n")
        wait_tty("2", "Password: ")
        machine.send_chars("alice-pw\n")
        wait_tty("2", "MAIN MENU")
        wait_tty("2", "LOCAL LINK")
        machine.send_chars("/shell")
        wait_tty("2", "No matches")
        machine.send_key("esc")
        machine.send_chars("q")
        wait_tty("2", "LOG OUT")
        machine.send_chars("y")
        wait_tty("2", "login: ")

        # Stable on tty3: the terminal shell is offered and is unprivileged.
        machine.send_key("alt-f3")
        wait_tty("3", "login: ")
        machine.send_chars("bob\n")
        wait_tty("3", "Password: ")
        machine.send_chars("bob-pw\n")
        wait_tty("3", "MODE: STABLE")
        machine.send_chars("/shell")
        wait_tty("3", "Terminal shell")
        machine.send_key("ret")
        wait_tty("3", "RESILIENCE SHELL")
        machine.send_chars("id -un > /tmp/shell-user; exit\n")
        machine.wait_until_succeeds("grep -qx bob /tmp/shell-user")
        wait_tty("3", "RESULTS")
  '';
}
