# The mesh network (ADR 0021): BATMAN-adv lets local messaging reach a
# computer that is not on the same network.
#
#   alpha ── vlan 1 ── beta ── vlan 2 ── gamma
#
# alpha and gamma share no link; beta has a cable on each side. Here the
# mesh runs over those cables (`resilience.mesh.devices`): a message from
# alpha to gamma waits until the mesh is on, then is delivered through
# beta, which relays it. bat0 never gets a global address or a default
# route. The helper's `set-mesh` and its polkit action are checked too.
#
# Then delta, alone, with two simulated Wi-Fi radios: its first card (the
# default) joins the Wi-Fi ad hoc cell; the second radio, moved into a
# network namespace, plays another computer of the mesh. Turning the mesh
# off gives the card back to NetworkManager.
{ self }:
{ ... }:
let
  # Cards named by vlan (lanN): with no default card the kernel's own
  # eth0, eth1 can collide with the test framework's ethN renames.
  wired =
    vlans:
    { config, lib, ... }:
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      virtualisation.memorySize = 1024;
      virtualisation.cores = 1;
      virtualisation.qemu.networkingOptions = lib.mkForce [ ];
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users.alice.mode = "ultra-stable";
      resilience.interface.autologinUser = "alice";
      resilience.comms.enable = true;
      virtualisation.interfaces = lib.listToAttrs (
        map (v: lib.nameValuePair "lan${toString v}" { vlan = v; }) vlans
      );
      resilience.mesh.devices = map (v: "lan${toString v}") vlans;
    };
in
{
  name = "resilience-mesh";

  nodes.alpha = wired [ 1 ];
  nodes.beta = wired [
    1
    2
  ];
  nodes.gamma = wired [ 2 ];

  nodes.delta =
    { lib, pkgs, ... }:
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      virtualisation.memorySize = 768;
      virtualisation.cores = 1;
      virtualisation.qemu.networkingOptions = lib.mkForce [ "-nic none" ];
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.mesh.enable = true;
      boot.kernelModules = [ "mac80211_hwsim" ];
      boot.extraModprobeConfig = "options mac80211_hwsim radios=2";
      networking.wireless.enable = lib.mkOverride 0 true;
      environment.systemPackages = [ pkgs.iw ];
    };

  testScript = ''
    import json

    BOX = "/home/alice/.local/share/resilience/messages"
    AS_ALICE = "runuser -u alice -- "
    HELPER = "resilience-admin"

    def send(m, mid, to, text):
        req = json.dumps({"to": to, "text": text})
        m.succeed(AS_ALICE + f"sh -c 'cat > {BOX}/outbox/{mid}.json.tmp && mv {BOX}/outbox/{mid}.json.tmp {BOX}/outbox/{mid}.json' <<'END'\n{req}\nEND")

    def record(m, mid):
        return json.loads(m.succeed(f"cat {BOX}/messages/{mid}.json"))

    def nm_state(m, dev):
        return m.succeed(f"nmcli -g GENERAL.STATE device show {dev}").strip()

    wired = (alpha, beta, gamma)
    for m in wired:
        m.start()
    for m in wired:
        m.wait_for_unit("multi-user.target")
        m.wait_for_unit("resilience-reticulum.service")
        m.wait_until_succeeds(f"test -s {BOX}/address", timeout=120)
        m.succeed("dmesg -n 1")
    a_addr = alpha.succeed(f"cat {BOX}/address").strip()
    g_addr = gamma.succeed(f"cat {BOX}/address").strip()

    with subtest("off by default: nothing changed, gamma out of reach"):
        for m in wired:
            m.fail("systemctl is-active resilience-mesh.service")
            m.fail("ip link show bat0")
        before = {m.name: {d: nm_state(m, d) for d in ("lan1", "lan2") if m.execute(f"ip link show {d}")[0] == 0} for m in wired}
        assert [sorted(before[m.name]) for m in wired] == [["lan1"], ["lan1", "lan2"], ["lan2"]], before
        # alpha hears beta, but never gamma: no shared link, no transport.
        alpha.wait_until_succeeds(f"grep -q . {BOX}/peers.json", timeout=120)
        send(alpha, "m1", g_addr, "Through the mesh")
        alpha.sleep(30)
        assert g_addr not in json.loads(alpha.succeed(f"cat {BOX}/peers.json")), "gamma heard"
        assert record(alpha, "m1")["state"] == "queued", record(alpha, "m1")

    with subtest("set-mesh: allowed at the console without a password, typed, audited"):
        tui = alpha.wait_until_succeeds("pgrep -u alice -x resilience").split()[0]
        alpha.succeed(f"pkcheck --action-id org.resilience.admin.set-mesh --process {tui}")
        # Not for a process of the same account outside the active session.
        alpha.succeed(AS_ALICE + "sh -c 'sleep 120 > /dev/null 2>&1 &'")
        outside = alpha.succeed("pgrep -u alice -x sleep").split()[0]
        alpha.fail(f"pkcheck --action-id org.resilience.admin.set-mesh --process {outside}")
        # Changing a mode still needs an administrator.
        alpha.fail(f"pkcheck --action-id org.resilience.admin.set-mode --process {tui}")
        for bad in ("maybe", "restart", "on --force"):
            alpha.fail(f"{HELPER} set-mesh {bad}")
        alpha.fail(AS_ALICE + f"{HELPER} set-mesh on")
        alpha.succeed("grep -q 'op=set-mesh.*result=refused' /var/log/resilience/audit.log")
        for m in wired:
            out = m.succeed(f"{HELPER} set-mesh on")
            assert "The mesh network is on." in out, out
            m.succeed("grep -q 'op=set-mesh.*to=on.*result=ok' /var/log/resilience/audit.log")

    with subtest("bat0: link-local only, no router advertisements, not managed"):
        for m in wired:
            m.succeed("systemctl is-active resilience-mesh.service")
            # Administratively up (a virtual interface's operstate may stay UNKNOWN).
            m.succeed("ip link show bat0 | grep -Eq '[<,]UP[,>]'")
            m.wait_until_succeeds("ip -6 addr show dev bat0 scope link | grep -q fe80", timeout=30)
            m.fail("ip -6 addr show dev bat0 scope global | grep -q inet6")
            m.fail("ip -4 addr show dev bat0 | grep -q inet")
            assert m.succeed("sysctl -n net.ipv6.conf.bat0.accept_ra").strip() == "0"
            assert m.succeed("sysctl -n net.ipv6.conf.bat0.autoconf").strip() == "0"
            m.fail("ip -6 route show default | grep -q bat0")
            m.fail("ip -4 route show default | grep -q bat0")
            assert "unmanaged" in nm_state(m, "bat0"), nm_state(m, "bat0")
            for d in before[m.name]:
                assert "unmanaged" in nm_state(m, d), (d, nm_state(m, d))
        # beta relays: both others are its neighbours; gamma is known to
        # alpha only through beta.
        beta.wait_until_succeeds("test $(batctl n -H | wc -l) -ge 2", timeout=60)
        alpha.wait_until_succeeds("batctl o -H | grep -q .", timeout=60)
        g_ll = gamma.succeed("ip -6 -o addr show dev bat0 scope link | awk '{print $4}' | cut -d/ -f1").strip()
        alpha.wait_until_succeeds(f"ping -6 -c 1 -W 2 {g_ll}%bat0", timeout=60)

    with subtest("a message reaches gamma through beta, with its proof"):
        # The instance restarted to use bat0; the store and the program reattached.
        for m in wired:
            m.wait_for_unit("resilience-reticulum.service")
            m.wait_for_unit("resilience-messages-node.service")
        alpha.wait_until_succeeds(f"grep -q '\"state\": \"delivered\"' {BOX}/messages/m1.json", timeout=300)
        gamma.wait_until_succeeds(f"grep -l 'Through the mesh' {BOX}/messages/in-*.json", timeout=60)
        gamma.wait_until_succeeds(f"grep -q '\"verified\": true' $(grep -l 'Through the mesh' {BOX}/messages/in-*.json)", timeout=60)
        got = json.loads(gamma.succeed(f"cat $(grep -l 'Through the mesh' {BOX}/messages/in-*.json)"))
        assert got["peer"] == a_addr, got

    with subtest("turned off: everything given back, gamma out of reach again"):
        out = beta.succeed(f"{HELPER} set-mesh off")
        assert "The mesh network is off." in out, out
        beta.fail("systemctl is-active resilience-mesh.service")
        beta.fail("ip link show bat0")
        beta.fail("test -e /run/resilience-mesh/devices")
        for d, state in before["beta"].items():
            beta.wait_until_succeeds(f"nmcli -g GENERAL.STATE device show {d} | grep -qx '{state}'", timeout=30)
        beta.wait_for_unit("resilience-reticulum.service")
        alpha.fail(f"ping -6 -c 2 -W 2 {g_ll}%bat0")
        send(alpha, "m2", g_addr, "Not without the relay")
        alpha.sleep(40)
        assert record(alpha, "m2")["state"] != "delivered", record(alpha, "m2")
        for m in wired:
            m.shutdown()

    with subtest("Wi-Fi: the first card joins the ad hoc cell and finds a neighbour"):
        delta.start()
        delta.wait_for_unit("multi-user.target")
        delta.succeed("dmesg -n 1")
        delta.wait_until_succeeds("ip link show wlan1 && ip link show wlan0", timeout=60)
        delta.wait_until_succeeds("nmcli -g GENERAL.STATE device show wlan0 | grep -v unmanaged", timeout=60)
        wlan0_before = nm_state(delta, "wlan0")
        # The second radio becomes another computer of the mesh.
        phy = delta.succeed("cat /sys/class/net/wlan1/phy80211/name").strip()
        delta.succeed("ip netns add far")
        delta.succeed(f"iw phy {phy} set netns name far")
        delta.succeed(
            "ip netns exec far sh -c '"
            "ip link set wlan1 down && iw dev wlan1 set type ibss && ip link set wlan1 mtu 1532 && "
            "ip link set wlan1 up && iw dev wlan1 ibss join resilience-mesh 2412 fixed-freq 02:52:45:53:4d:31 && "
            "ip link add bat0 type batadv && ip link set wlan1 master bat0 && ip link set bat0 up'"
        )
        out = delta.succeed(f"{HELPER} set-mesh on")
        assert "on" in out, out
        info = delta.succeed("iw dev wlan0 info")
        assert "type IBSS" in info, info
        delta.succeed("iw dev wlan0 link | grep -q 'Joined IBSS 02:52:45:53:4d:31'")
        delta.succeed("cat /run/resilience-mesh/devices | grep -qx wlan0")
        delta.wait_until_succeeds("test $(batctl n -H | wc -l) -ge 1", timeout=60)
        far_ll = delta.succeed("ip netns exec far ip -6 -o addr show dev bat0 scope link | awk '{print $4}' | cut -d/ -f1").strip()
        delta.wait_until_succeeds(f"ping -6 -c 1 -W 2 {far_ll}%bat0", timeout=60)

    with subtest("Wi-Fi: turned off, the card is NetworkManager's again"):
        delta.succeed(f"{HELPER} set-mesh off")
        delta.fail("ip link show bat0")
        info = delta.succeed("iw dev wlan0 info")
        assert "type managed" in info, info
        delta.wait_until_succeeds(f"nmcli -g GENERAL.STATE device show wlan0 | grep -qx '{wlan0_before}'", timeout=30)
  '';
}
