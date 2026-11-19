# Computers joined by cables to a switch with no router and no DHCP
# server, plugged in after they have started (the owner's real-hardware
# case, 2026-11-19; a single cable between two computers is the same with
# two). NetworkManager tries "Wired" (DHCP) once, then "Wired without
# router" (link-local IPv4 and IPv6) comes up; its dispatcher restarts the
# Reticulum instance so that it uses the card; every computer hears the
# others and local messages go around.
{ self }:
{ ... }:
let
  node =
    { lib, ... }:
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      virtualisation.memorySize = 1024;
      virtualisation.cores = 1;
      # One switch (vlan 1), no slirp NIC (it would give a default route),
      # and no address from the test framework: NetworkManager alone, as on
      # a real computer.
      virtualisation.vlans = [ 1 ];
      virtualisation.qemu.networkingOptions = lib.mkForce [ ];
      networking.interfaces.eth1.ipv4.addresses = lib.mkForce [ ];
      networking.interfaces.eth1.ipv6.addresses = lib.mkForce [ ];
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users.alice.mode = "ultra-stable";
      resilience.interface.autologinUser = "alice";
      resilience.comms.enable = true;
    };
in
{
  name = "resilience-cable";

  nodes.alpha = node;
  nodes.beta = node;
  nodes.gamma = node;

  testScript = ''
    import json

    BOX = "/home/alice/.local/share/resilience/messages"
    AS_ALICE = "runuser -u alice -- "

    def send(m, mid, to, text):
        req = json.dumps({"to": to, "text": text})
        m.succeed(AS_ALICE + f"sh -c 'cat > {BOX}/outbox/{mid}.json.tmp && mv {BOX}/outbox/{mid}.json.tmp {BOX}/outbox/{mid}.json' <<'END'\n{req}\nEND")

    def rnsd_pid(m):
        return m.succeed("systemctl show -p MainPID --value resilience-reticulum.service").strip()

    trio = (alpha, beta, gamma)
    # The cables are unplugged when they start.
    for m in trio:
        m.start()
        m.send_monitor_command("set_link vlan1 off")
    addr = {}
    for m in trio:
        m.wait_for_unit("multi-user.target")
        m.wait_for_unit("resilience-reticulum.service")
        m.wait_until_succeeds(f"test -s {BOX}/address", timeout=120)
        m.succeed("dmesg -n 1")
        addr[m.name] = m.succeed(f"cat {BOX}/address").strip()

    with subtest("unplugged: nobody heard, NetworkManager's own profile not made"):
        for m in trio:
            m.fail("ip -6 addr show dev eth1 scope link | grep -q fe80")
            m.fail("nmcli connection show 'Wired connection 1'")
        alpha.sleep(10)
        for other in (beta, gamma):
            alpha.fail(f"grep -q {addr[other.name]} {BOX}/peers.json")
        before = {m.name: rnsd_pid(m) for m in trio}

    with subtest("plugged into a switch with no router: link-local, the instance restarted"):
        for m in trio:
            m.send_monitor_command("set_link vlan1 on")
        for m in trio:
            # "Wired" (DHCP) gives up after 45 s; "Wired without router" comes up.
            m.wait_until_succeeds(
                "nmcli -t -g GENERAL.CONNECTION device show eth1 | grep -qx 'Wired without router'",
                timeout=150,
            )
            m.succeed("nmcli -g GENERAL.STATE device show eth1 | grep -q '100 (connected'")
            m.wait_until_succeeds("ip -4 addr show dev eth1 | grep -q 'inet 169.254.'", timeout=30)
            m.succeed("ip -6 addr show dev eth1 scope link | grep -q fe80")
            m.wait_until_succeeds(
                f"test \"$(systemctl show -p MainPID --value resilience-reticulum.service)\" != {before[m.name]}",
                timeout=60,
            )
            m.wait_for_unit("resilience-reticulum.service")

    with subtest("everyone hears everyone, and messages go around"):
        for m in trio:
            for other in trio:
                if other is not m:
                    m.wait_until_succeeds(f"grep -q {addr[other.name]} {BOX}/peers.json", timeout=180)
        for (src, dst, text) in ((alpha, beta, "alpha to beta"), (beta, gamma, "beta to gamma"), (gamma, alpha, "gamma to alpha")):
            mid = "m-" + src.name
            send(src, mid, addr[dst.name], text)
            src.wait_until_succeeds(f"grep -q '\"state\": \"delivered\"' {BOX}/messages/{mid}.json", timeout=300)
            dst.wait_until_succeeds(f"grep -l '{text}' {BOX}/messages/in-*.json", timeout=60)

    with subtest("unplugged and plugged again: back to the same, no stuck profile"):
        alpha.send_monitor_command("set_link vlan1 off")
        alpha.wait_until_fails("ip -6 addr show dev eth1 scope link | grep -q fe80", timeout=60)
        alpha.send_monitor_command("set_link vlan1 on")
        alpha.wait_until_succeeds(
            "nmcli -t -g GENERAL.CONNECTION device show eth1 | grep -qx 'Wired without router'",
            timeout=150,
        )
        send(alpha, "again", addr["gamma"], "plugged again")
        alpha.wait_until_succeeds(f"grep -q '\"state\": \"delivered\"' {BOX}/messages/again.json", timeout=300)
  '';
}
