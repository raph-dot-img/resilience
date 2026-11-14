# Making an earlier system generation the default (ADR 0011), on a VM that
# really boots through systemd-boot (UEFI), with two different generations:
# generation 1 is the image's own system; generation 2 is the same system
# plus a marker file in /etc, created in the test the way nixos-rebuild
# does. Driven through the real TUI on tty1 (Ultra-Stable account, Advanced
# account's password), then checked across real reboots.
{ self }:
{
  name = "resilience-generations";

  nodes.machine =
    {
      config,
      lib,
      extendModules,
      ...
    }:
    let
      # The second generation: this configuration plus a marker. The option
      # below stops the nesting (the second system does not embed a third).
      second = extendModules {
        modules = [
          {
            resilienceTest.second = true;
            environment.etc."resilience-test-generation".text = "second\n";
          }
        ];
      };
    in
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      options.resilienceTest.second = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Test only: this is the second generation.";
      };
      config = {
        virtualisation.memorySize = 1536;
        virtualisation.cores = 2;
        virtualisation.qemu.networkingOptions = lib.mkForce [ "-nic none" ];
        virtualisation.useBootLoader = true;
        virtualisation.useEFIBoot = true;
        boot.loader.systemd-boot.enable = true;
        boot.loader.efi.canTouchEfiVariables = true;
        users.users.root.hashedPassword = lib.mkForce null;
        resilience.users = {
          alice.mode = "ultra-stable";
          carol.mode = "advanced";
        };
        resilience.interface.autologinUser = "alice";
        users.users.carol.initialPassword = "carol-pw";
        system.extraDependencies = lib.optional (
          !config.resilienceTest.second
        ) second.config.system.build.toplevel;
        # Read by the test script.
        environment.etc."resilience-test-second".text = lib.optionalString (
          !config.resilienceTest.second
        ) "${second.config.system.build.toplevel}\n";
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
                return text
            if _time.monotonic() > deadline:
                print(f"--- tty{tty} contents ---")
                print(text)
                raise Exception(f"timed out waiting for {regexp!r} on tty{tty}")
            _time.sleep(0.5)

    def audit():
        return machine.succeed("cat /var/log/resilience/audit.log")

    def default_link():
        return machine.succeed("readlink /nix/var/nix/profiles/system").strip()

    def loader_default():
        return machine.succeed("sed -n 's/^default //p' /boot/loader/loader.conf").strip()

    def boot():
        machine.start()
        machine.wait_for_unit("multi-user.target")
        machine.succeed("dmesg -n 1")
        wait_tty("1", "MAIN MENU")

    boot()
    helper = machine.succeed("sed -n 's/^helper = \"\\(.*\\)\"$/\\1/p' /etc/resilience/admin.toml").strip()
    second = machine.succeed("cat /etc/resilience-test-second").strip()
    assert second.startswith("/nix/store/"), second

    with subtest("setup: a second generation becomes the default, as after nixos-rebuild boot"):
        print(machine.succeed("ls -l /nix/var/nix/profiles/"))
        assert default_link() == "system-1-link", default_link()
        machine.succeed(f"nix-env -p /nix/var/nix/profiles/system --set {second}")
        machine.succeed(f"{second}/bin/switch-to-configuration boot")
        assert default_link() == "system-2-link", default_link()
        assert re.fullmatch(r"nixos-generation-2\.conf", loader_default()), loader_default()
        machine.shutdown()
        boot()
        machine.succeed("test -e /etc/resilience-test-generation")
        machine.succeed(f"test \"$(readlink -f /run/booted-system)\" = {second}")

    with subtest("helper: root only, generation number validated, audited"):
        machine.succeed("grep -q 'org.resilience.admin.set-default-generation' /run/current-system/sw/share/polkit-1/actions/org.resilience.admin.policy")
        rc, out = machine.execute(f"runuser -u alice -- {helper} set-default-generation 1 2>&1")
        assert rc == 77, (rc, out)
        for arg, reason in [
            ("0", "by its number"),
            ("01", "by its number"),
            ("-1", "by its number"),
            ("abc", "by its number"),
            ("'1;id'", "by its number"),
            ("../system-1-link", "by its number"),
            ("99", "no generation 99"),
            ("2", "already the default"),
        ]:
            rc, out = machine.execute(f"{helper} set-default-generation {arg} 2>&1")
            assert rc == 2 and reason in out, (arg, rc, out)
        rc, out = machine.execute(f"{helper} set-default-generation 1 extra 2>&1")
        assert rc == 64, (rc, out)
        log = audit()
        print(log)
        assert len(re.findall(r"op=set-default-generation .*result=refused", log)) == 8, log
        assert default_link() == "system-2-link"
        assert loader_default() == "nixos-generation-2.conf"

    with subtest("polkit: an Ultra-Stable session needs an administrator's password"):
        pid = machine.succeed("pgrep -u alice -x resilience").split()[0]
        rc, out = machine.execute(f"pkcheck --action-id org.resilience.admin.set-default-generation --process {pid} 2>&1")
        print(rc, out)
        # 2: authorization requires a challenge (a password), not yes or no.
        assert rc == 2, (rc, out)
        info = machine.succeed("pkaction --verbose --action-id org.resilience.admin.set-default-generation")
        print(info)
        for kind in ["any", "inactive", "active"]:
            assert re.search(rf"implicit {kind}:\s+auth_admin\n", info), info

    def choose_generation_1():
        wait_tty("1", "SYSTEM GENERATIONS")
        wait_tty("1", r"2  \d{4}-\d\d-\d\d  started, default")
        # The list wraps around: End always selects the oldest generation.
        machine.send_key("end")
        wait_tty("1", r"Generation\s+1")
        machine.send_key("ret")
        wait_tty("1", "DEFAULT GENERATION")
        wait_tty("1", "older system")
        machine.send_chars("y")
        wait_tty("1", "AUTHENTICATING FOR org.resilience.admin.set-default-generation")
        wait_tty("1", "Choose identity|Password:")
        screen = tty_text("1")
        if "Choose identity" in screen:
            m = re.search(r"(\d)\. .*carol", screen)
            assert m, screen
            machine.send_chars(m.group(1) + "\n")
            wait_tty("1", "Password:")

    with subtest("TUI: a wrong administrator password changes nothing"):
        machine.send_chars("/rollback")
        wait_tty("1", "Recovery")
        machine.send_key("ret")
        wait_tty("1", "RECOVERY & DIAGNOSTICS")
        machine.send_chars("2")
        choose_generation_1()
        machine.send_chars("not-the-password\n")
        wait_tty("1", "Press Enter to return", timeout=90)
        assert default_link() == "system-2-link"
        assert loader_default() == "nixos-generation-2.conf"
        assert not re.search(r"op=set-default-generation .*result=ok", audit())
        machine.send_key("ret")

    with subtest("TUI: Ultra-Stable user makes generation 1 the default with an administrator's password"):
        choose_generation_1()
        machine.send_chars("carol-pw\n")
        wait_tty("1", "Generation 1 is now started by default", timeout=180)
        wait_tty("1", "Press Enter to return")
        assert default_link() == "system-1-link", default_link()
        assert loader_default() == "nixos-generation-1.conf", loader_default()
        # The running system is untouched until the restart.
        machine.succeed("test -e /etc/resilience-test-generation")
        machine.succeed(f"test \"$(readlink -f /run/current-system)\" = {second}")
        log = audit()
        print(log)
        assert re.search(r"op=set-default-generation caller=alice caller_uid=\d+ from=2 to=1 result=ok", log), log
        machine.succeed("journalctl -t resilience-admin | grep -q 'op=set-default-generation.*to=1 result=ok'")
        machine.send_key("ret")
        wait_tty("1", r"1  \d{4}-\d\d-\d\d  default")

    with subtest("after a restart generation 1 runs; generation 2 is still there"):
        machine.shutdown()
        boot()
        machine.fail("test -e /etc/resilience-test-generation")
        machine.fail(f"test \"$(readlink -f /run/booted-system)\" = {second}")
        assert default_link() == "system-1-link"
        machine.succeed("test -e /nix/var/nix/profiles/system-2-link")
        machine.succeed("test -e /boot/loader/entries/nixos-generation-2.conf")
        machine.succeed("systemctl is-system-running --wait | grep -qx running")

    with subtest("the newer generation can be made the default again (root, as with sudo)"):
        machine.succeed(f"{helper} set-default-generation 2")
        assert default_link() == "system-2-link"
        assert loader_default() == "nixos-generation-2.conf"
        assert re.search(r"op=set-default-generation caller=root caller_uid=0 from=1 to=2 result=ok", audit())
  '';
}
