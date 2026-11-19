# The hotspot (ADR 0025 §6, §7): an administrator turns this computer into
# a WPA2 Wi-Fi network with `resilience-admin set-hotspot on`; a "phone"
# (the second simulated radio, moved into a network namespace, with
# wpa_supplicant and a DHCP client) joins it with the password shown once,
# gets an address, resolves names in home.arpa only, reaches what is
# shared on the hotspot, and is never routed to the other network (a
# cable to `outside`), although NetworkManager's shared mode turns IP
# forwarding on. The password is in no log and no process list.
{ self }:
{ ... }:
{
  name = "resilience-hotspot";

  nodes.hot =
    { lib, pkgs, ... }:
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      virtualisation.memorySize = 1024;
      virtualisation.cores = 2;
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users = {
        alice.mode = "ultra-stable";
        carol.mode = "advanced";
      };
      resilience.interface.autologinUser = "alice";
      users.users.carol.initialPassword = "carol-pw";
      resilience.share.enable = true;
      resilience.share.hotspot.dnsRecords = {
        library = "10.42.0.1";
        printer = "10.42.0.20";
      };
      # Only to check that the mesh and the hotspot exclude each other.
      resilience.mesh.enable = true;

      boot.kernelModules = [ "mac80211_hwsim" ];
      boot.extraModprobeConfig = "options mac80211_hwsim radios=2";
      networking.wireless.enable = lib.mkOverride 0 true;
      environment.systemPackages = [
        pkgs.wpa_supplicant
        pkgs.busybox
        pkgs.dig
        pkgs.curl
        pkgs.iw
      ];
      # The phone's DHCP client script (busybox udhcpc).
      environment.etc."udhcpc.script" = {
        mode = "0755";
        text = ''
          #!/bin/sh
          case "$1" in
            bound|renew)
              ip addr flush dev "$interface"
              ip addr add "$ip/$mask" dev "$interface"
              [ -n "$router" ] && ip route replace default via "''${router%% *}"
              echo "$ip $router $dns" > /tmp/lease
              ;;
          esac
        '';
      };
    };

  nodes.outside =
    { pkgs, ... }:
    {
      virtualisation.memorySize = 384;
      virtualisation.cores = 1;
    };

  testScript = ''
    import re

    HELPER = "resilience-admin"
    CL = "ip netns exec cl "

    start_all()
    hot.wait_for_unit("multi-user.target")
    outside.wait_for_unit("multi-user.target")
    hot.succeed("dmesg -n 1")
    out_ip = outside.succeed("ip -4 -o addr show eth1 | awk '{print $4}' | cut -d/ -f1").strip()
    hot.wait_until_succeeds("ip link show wlan1 && ip link show wlan0", timeout=60)

    # The second radio becomes a phone, outside this computer's own stack.
    phy = hot.succeed("cat /sys/class/net/wlan1/phy80211/name").strip()
    hot.succeed("ip netns add cl")
    hot.succeed(f"iw phy {phy} set netns name cl")
    hot.succeed(CL + "ip link set lo up")
    hot.succeed(CL + "ip link set wlan1 up")

    with subtest("set-hotspot: administrators only, every argument checked, audited"):
        info = hot.succeed("pkaction --verbose --action-id org.resilience.admin.set-hotspot")
        for kind in ["any", "inactive", "active"]:
            assert re.search(rf"implicit {kind}:\s+auth_admin\n", info), info
        rc, _ = hot.execute(f"runuser -u alice -- {HELPER} set-hotspot on")
        assert rc == 77, rc
        for bad in ("", "maybe", "on now", "ON"):
            rc, out = hot.execute(f"{HELPER} set-hotspot {bad}")
            assert rc in (2, 64), (bad, rc, out)
        hot.succeed("grep -q 'op=set-hotspot.*result=refused' /var/log/resilience/audit.log")
        hot.fail("systemctl is-active resilience-hotspot.service")

    with subtest("on: a WPA2 network, the password shown once and nowhere else"):
        out = hot.succeed(f"{HELPER} set-hotspot on")
        print(out)
        m = re.search(r"Password: ([a-z0-9-]{19})", out)
        assert m, out
        password = m.group(1)
        hot.succeed(f"echo -n {password} > /root/pw")
        hot.succeed("grep -q 'op=set-hotspot.*result=ok' /var/log/resilience/audit.log")
        hot.succeed("systemctl is-active resilience-hotspot.service")
        prof = "/run/NetworkManager/system-connections/resilience-hotspot.nmconnection"
        assert hot.succeed(f"stat -c '%a %U' {prof}").strip() == "600 root"
        assert hot.succeed("stat -c '%a' /run/resilience-hotspot/password").strip() == "600"
        state = hot.succeed("cat /run/resilience-share/hotspot")
        assert 'device = "wlan0"' in state and password not in state, state
        hot.wait_until_succeeds("nmcli -t -f GENERAL.STATE connection show resilience-hotspot | grep -q activated", timeout=30)
        assert "type AP" in hot.succeed("iw dev wlan0 info"), hot.succeed("iw dev wlan0 info")
        # Not in the journal, the audit log, or a command line.
        hot.fail("journalctl -b | grep -F -f /root/pw")
        hot.fail("grep -F -f /root/pw /var/log/resilience/audit.log")
        hot.fail("ps -eo args | grep -F -f /root/pw")

    with subtest("a phone joins: an address, DNS for home.arpa only"):
        hot.succeed(
            "printf 'ctrl_interface=/run/wpa-cl\\nnetwork={\\n ssid=\"Resilience-hotspot\"\\n psk=\"%s\"\\n key_mgmt=WPA-PSK\\n proto=RSN\\n}\\n' "
            "$(cat /root/pw) > /root/cl.conf"
        )
        hot.succeed(CL + "wpa_supplicant -B -D nl80211 -i wlan1 -c /root/cl.conf")
        hot.wait_until_succeeds(CL + "iw dev wlan1 link | grep -q 'SSID: Resilience-hotspot'", timeout=60)
        hot.wait_until_succeeds(CL + "busybox udhcpc -i wlan1 -n -q -t 5 -s /etc/udhcpc.script", timeout=90)
        lease = hot.succeed("cat /tmp/lease").split()
        print(lease)
        assert lease[0].startswith("10.42.0.") and lease[1] == "10.42.0.1", lease
        dig = CL + "dig +short +time=3 +tries=1 @10.42.0.1 "
        assert hot.succeed(dig + "resilience.home.arpa").strip() == "10.42.0.1"
        assert hot.succeed(dig + "printer.home.arpa").strip() == "10.42.0.20"
        assert hot.succeed(dig + "library.home.arpa").strip() == "10.42.0.1"
        # Nothing else is answered or forwarded.
        assert hot.succeed(dig + "example.org || true").strip() == ""
        assert hot.succeed(dig + "outside || true").strip() == ""

    with subtest("never routed to the other network"):
        # NetworkManager turns forwarding on for the hotspot's card itself.
        assert hot.succeed("sysctl -n net.ipv4.conf.wlan0.forwarding").strip() == "1", "shared mode forwards"
        hot.fail(CL + f"ping -c 2 -W 2 {out_ip}")
        # Control: on a machine whose other card forwards too (NetworkManager
        # does so for the default route's), without our rule the phone would
        # reach `outside`. Forwarding stays on everywhere for the checks after.
        hot.succeed("sysctl -w net.ipv4.conf.all.forwarding=1")
        hot.fail(CL + f"ping -c 2 -W 2 {out_ip}")
        hot.succeed("iptables -w -F resilience-hotspot-fwd")
        hot.wait_until_succeeds(CL + f"ping -c 1 -W 2 {out_ip}", timeout=30)
        hot.succeed("systemctl reload resilience-hotspot.service")
        hot.fail(CL + f"ping -c 2 -W 2 {out_ip}")
        # A firewall reload puts the rules back too.
        hot.succeed("systemctl reload firewall.service")
        # Never emptied by the firewall itself: no gap while it reloads.
        for _ in range(20):
            hot.succeed("iptables -w -S resilience-hotspot-fwd | grep -q 'wlan0 -j DROP'")
        hot.wait_until_succeeds("iptables -w -S resilience-hotspot | grep -q 'dport 67'", timeout=10)
        hot.fail(CL + f"ping -c 2 -W 2 {out_ip}")

    with subtest("what is shared on the hotspot is reachable by name"):
        hot.succeed(f"{HELPER} set-share on wlan0 --name 'On the hotspot' pages")
        hot.succeed("echo '<p>hotspotpageword</p>' > /var/lib/resilience/shared/pages/index.html && chmod 0644 /var/lib/resilience/shared/pages/index.html")
        hot.wait_until_succeeds(
            CL + "curl -sf --max-time 5 --resolve resilience.home.arpa:80:10.42.0.1 http://resilience.home.arpa/pages/ | grep -q hotspotpageword",
            timeout=30,
        )
        hot.succeed(f"{HELPER} set-share off")
        hot.fail(CL + "curl -sf --max-time 5 http://10.42.0.1/")

    with subtest("the mesh cannot start while the hotspot is on"):
        rc, out = hot.execute(f"{HELPER} set-mesh on 2>&1")
        assert rc != 0, out
        hot.fail("ip link show bat0")
        hot.succeed("systemctl is-active resilience-hotspot.service")

    with subtest("off: the network, its profile, its password and its rules are gone"):
        out = hot.succeed(f"{HELPER} set-hotspot off")
        assert "off" in out, out
        hot.fail("systemctl is-active resilience-hotspot.service")
        hot.fail("test -e /run/NetworkManager/system-connections/resilience-hotspot.nmconnection")
        hot.fail("test -e /run/resilience-hotspot/password")
        hot.fail("test -e /run/resilience-share/hotspot")
        hot.fail("nmcli connection show resilience-hotspot")
        assert hot.succeed("iptables -w -S resilience-hotspot-fwd").strip() == "-N resilience-hotspot-fwd"
        assert hot.succeed("iptables -w -S resilience-hotspot").strip() == "-N resilience-hotspot"
        hot.wait_until_fails(CL + "iw dev wlan1 link | grep -q 'SSID: Resilience-hotspot'", timeout=60)
        # A new password each time.
        out = hot.succeed(f"{HELPER} set-hotspot on")
        m2 = re.search(r"Password: ([a-z0-9-]{19})", out)
        assert m2 and m2.group(1) != password, out
        hot.succeed(f"{HELPER} set-hotspot off")
  '';
}
