# Wi-Fi for an Ultra-Stable account (ADR 0010), driven through the real TUI
# on tty1. Two simulated radios (mac80211_hwsim): wlan0 is a WPA2 access
# point (hostapd + dnsmasq, test only), wlan1 is managed by NetworkManager.
# Checks the polkit restriction for accounts without the networkmanager
# group, joining with a wrong and the right password (typed in the TUI,
# passed to nmcli on stdin), the private profile, the secret in no log or
# process list, reconnecting without a password, and forgetting.
{ self }:
{
  name = "resilience-wifi";

  nodes.machine =
    { lib, pkgs, ... }:
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
      resilience.users = {
        alice.mode = "ultra-stable";
        bob.mode = "stable";
      };
      resilience.interface.autologinUser = "alice";
      users.users.bob.initialPassword = "bob-pw";

      # Simulated Wi-Fi. qemu-vm.nix forces wpa_supplicant off in test VMs
      # (NetworkManager enables it on real systems).
      boot.kernelModules = [ "mac80211_hwsim" ];
      boot.extraModprobeConfig = "options mac80211_hwsim radios=2";
      networking.wireless.enable = lib.mkOverride 0 true;
      # Test only: wlan0 is the access point, with a DHCP server.
      networking.networkmanager.unmanaged = [ "wlan0" ];
      networking.firewall.interfaces.wlan0.allowedUDPPorts = [ 67 ];
      environment.systemPackages = [
        pkgs.hostapd
        pkgs.dnsmasq
      ];
      environment.etc."test-hostapd.conf".text = ''
        interface=wlan0
        driver=nl80211
        ssid=Resilience Test
        hw_mode=g
        channel=6
        wpa=2
        wpa_key_mgmt=WPA-PSK
        rsn_pairwise=CCMP
        wpa_passphrase=correct-horse-42
      '';
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

    def profiles():
        return machine.succeed("grep -l 'ssid=Resilience Test' /etc/NetworkManager/system-connections/* 2>/dev/null || true").splitlines()

    def wlan1_state():
        return machine.succeed("nmcli -t -f DEVICE,STATE device | sed -n 's/^wlan1://p'").strip()

    secret = "correct-horse-42"
    # Matches the secret without containing it, so grep never finds its own
    # command line (or the shell running it).
    secret_re = "correct-horse-4[2]"

    machine.start()
    machine.wait_for_unit("multi-user.target")
    machine.wait_for_unit("NetworkManager.service")
    # Test VMs print kernel messages on the console (the ISO uses loglevel=3);
    # keep them from drawing over the interface.
    machine.succeed("dmesg -n 1")
    machine.succeed("ip addr add 192.168.77.1/24 dev wlan0 && ip link set wlan0 up")
    machine.succeed("systemd-run --unit test-ap hostapd /etc/test-hostapd.conf")
    machine.succeed("systemd-run --unit test-dhcp dnsmasq -k -i wlan0 --bind-interfaces --port=0 --dhcp-range=192.168.77.10,192.168.77.50")
    machine.wait_until_succeeds("journalctl -u test-ap | grep -q AP-ENABLED")
    machine.wait_until_succeeds("test \"$(nmcli -t -f DEVICE,STATE device | sed -n 's/^wlan1://p')\" = disconnected", timeout=60)
    wait_tty("1", "MAIN MENU")

    with subtest("polkit: without the networkmanager group only scan, connect, own profiles and the radio"):
        pid = machine.succeed("pgrep -u alice -x resilience").split()[0]
        # enable-disable-wifi is allowed for active sessions only
        # (allow_inactive=no), so its check also proves the TUI's session is
        # the active console session.
        allowed = ["wifi.scan", "network-control", "settings.modify.own", "enable-disable-wifi"]
        denied = [
            "enable-disable-network",
            "enable-disable-connectivity-check",
            "enable-disable-statistics",
            "enable-disable-wwan",
            "wifi.share.open",
            "wifi.share.protected",
            "settings.modify.system",
            "settings.modify.hostname",
            "settings.modify.global-dns",
            "reload",
            "checkpoint-rollback",
        ]
        for a in allowed:
            machine.succeed(f"pkcheck --action-id org.freedesktop.NetworkManager.{a} --process {pid}")
        for a in denied:
            rc, out = machine.execute(f"pkcheck --action-id org.freedesktop.NetworkManager.{a} --process {pid} 2>&1")
            print(a, rc, out.strip())
            # Refused outright, not "ask for an administrator".
            assert rc == 1 and "auth_admin" not in out, (a, rc, out)

    with subtest("polkit: a Stable account (networkmanager group) keeps NetworkManager's rights"):
        machine.send_key("alt-f2")
        wait_tty("2", "login: ")
        machine.send_chars("bob\n")
        wait_tty("2", "Password: ")
        machine.send_chars("bob-pw\n")
        wait_tty("2", "MODE: STABLE")
        machine.wait_until_succeeds("pgrep -u bob -x resilience")
        bpid = machine.succeed("pgrep -u bob -x resilience").split()[0]
        machine.succeed(f"pkcheck --action-id org.freedesktop.NetworkManager.enable-disable-network --process {bpid}")
        machine.succeed(f"pkcheck --action-id org.freedesktop.NetworkManager.settings.modify.system --process {bpid}")
        machine.send_key("alt-f1")
        wait_tty("1", "MAIN MENU")

    with subtest("TUI lists the network"):
        machine.send_chars("/wi-fi")
        wait_tty("1", "Wi-Fi networks")
        machine.send_key("ret")
        wait_tty("1", "WI-FI NETWORKS")
        deadline = _time.monotonic() + 60
        while "Resilience Test" not in tty_text("1"):
            assert _time.monotonic() < deadline, tty_text("1")
            machine.send_chars("r")
            _time.sleep(3)
        wait_tty("1", "WPA2")
        wait_tty("1", "You will be asked")

    def wait_wlan1(state, timeout=90):
        machine.wait_until_succeeds(f"test \"$(nmcli -t -f DEVICE,STATE device | sed -n 's/^wlan1://p')\" = {state}", timeout=timeout)

    with subtest("Esc in the password dialog creates nothing"):
        machine.send_key("ret")
        wait_tty("1", "WI-FI PASSWORD")
        machine.send_chars("abc")
        wait_tty("1", r"> \*\*\*")
        machine.send_key("esc")
        wait_tty("1", "You will be asked")
        assert profiles() == [], profiles()

    with subtest("a wrong password connects nothing, saves nothing and says so"):
        machine.send_key("ret")
        wait_tty("1", "WI-FI PASSWORD")
        machine.send_chars("wrong-password-1\n")
        wait_tty("1", "connecting to 'Resilience Test'")
        text = wait_tty("1", "NOT CONNECTED", timeout=120)
        print(text)
        wait_tty("1", "probably wrong")
        wait_tty("1", "Nothing was")
        assert profiles() == [], profiles()
        # NetworkManager stops trying once the attempt is over.
        wait_wlan1("disconnected", timeout=60)
        machine.send_key("ret")
        wait_tty("1", "WI-FI NETWORKS")

    with subtest("the right password connects; the profile belongs to alice alone"):
        machine.send_key("ret")
        wait_tty("1", "WI-FI PASSWORD")
        machine.send_chars(secret + "\n")
        wait_wlan1("connected")
        machine.succeed("ip -4 addr show wlan1 | grep -q 'inet 192.168.77.'")
        [p] = profiles()
        machine.succeed(f"grep -qx 'permissions=user:alice:;' '{p}'")
        machine.succeed(f"test \"$(stat -c '%U %a' '{p}')\" = 'root 600'")
        wait_tty("1", r"\*Resilience Test")
        wait_tty("1", "saved")
        assert "NOT CONNECTED" not in tty_text("1")

    with subtest("the password is in no log, no process list and not readable by alice"):
        machine.fail(f"journalctl -b | grep -q '{secret_re}'")
        machine.fail(f"grep -rq '{secret_re}' /home/alice")
        machine.fail(f"grep -qs '{secret_re}' /proc/[0-9]*/cmdline")
        # Positive control: the pattern does find the secret where it is.
        machine.succeed(f"grep -q 'psk={secret_re}' '{p}'")
        machine.fail(f"runuser -u alice -- cat {p}")
        machine.fail("runuser -u bob -- nmcli -s -g 802-11-wireless-security.psk connection show 'Resilience Test'")

    with subtest("disconnect, then reconnect without typing the password"):
        machine.send_chars("d")
        wait_wlan1("disconnected", timeout=30)
        machine.send_key("ret")
        wait_tty("1", "connecting to 'Resilience Test'")
        wait_wlan1("connected")
        assert "WI-FI PASSWORD" not in tty_text("1"), tty_text("1")
        wait_tty("1", r"\*Resilience Test")

    with subtest("forget removes the profile and its password"):
        machine.send_chars("f")
        wait_tty("1", "FORGET NETWORK")
        machine.send_chars("y")
        machine.wait_until_succeeds("test -z \"$(grep -l 'ssid=Resilience Test' /etc/NetworkManager/system-connections/* 2>/dev/null)\"", timeout=30)
        machine.wait_until_succeeds("test \"$(nmcli -t -f DEVICE,STATE device | sed -n 's/^wlan1://p')\" = disconnected", timeout=30)
  '';
}
