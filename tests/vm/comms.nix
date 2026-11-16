# Local messaging network (ADR 0018, slice 1): two machines on an isolated
# network with no Internet and no DNS. On each, the opt-in Reticulum
# service runs unprivileged and the two find each other; a test program
# run by an ordinary account attaches to its machine's instance with the
# client configuration and sends a signed LXMF message, with its delivery
# proof, to the other machine. A message to a machine that has gone is never "delivered".
#
# The per-user messaging program and the TUI come with the next slices.
{ self }:
{ ... }:
let
  node =
    { config, lib, ... }:
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      virtualisation.memorySize = 1024;
      virtualisation.cores = 1;
      # One isolated VLAN; no slirp NIC (it would give a default route).
      virtualisation.vlans = [ 1 ];
      virtualisation.qemu.networkingOptions = lib.mkForce [ ];
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users.alice.mode = "ultra-stable";
      resilience.comms.enable = true;
      environment.etc."resilience-test/python".source = "${config.resilience.comms.package}/bin/python";
      environment.etc."resilience-test/rnstatus".source =
        "${config.resilience.comms.package}/bin/rnstatus";
      environment.etc."resilience-test/node.py".source = ./comms-node.py;
    };
in
{
  name = "resilience-comms";

  nodes.alpha = node;
  nodes.beta = node;

  testScript = ''
    import re

    CLIENT = "/run/resilience-reticulum/client/config"
    AS_ALICE = "runuser -u alice -- "
    NODE = AS_ALICE + "/etc/resilience-test/python /etc/resilience-test/node.py"

    def prepare(m):
        m.wait_for_unit("multi-user.target")
        m.wait_for_unit("resilience-reticulum.service")
        m.wait_for_file(CLIENT)
        m.succeed(AS_ALICE + f"sh -c 'mkdir -p ~/rns && cp {CLIENT} ~/rns/config'")

    start_all()
    for m in (alpha, beta):
        prepare(m)

    with subtest("the service is unprivileged and the key is not in the store"):
        for m in (alpha, beta):
            user = m.succeed("ps -o user= -C .rnsd-wrapped | sort -u").strip()
            assert user and user != "root", user
            assert m.succeed(f"stat -c %a {CLIENT}").strip() == "644"
            m.succeed("test \"$(stat -c %a /var/lib/private/resilience-reticulum)\" = 700")
            m.succeed(f"grep -Eq '^  rpc_key = [0-9a-f]{{64}}$' {CLIENT}")
            m.succeed("grep -Eqx '[0-9a-f]{64}' /var/lib/private/resilience-reticulum/rpc-key")
            # The store holds the configurations without any key.
            m.fail("grep -l rpc_key /nix/store/*reticulum-*-config")
            # No remote shell or command service, and nothing in PATH.
            m.fail("pgrep -f 'rns[h]|rn[x] '")
            m.fail(AS_ALICE + "sh -c 'command -v rnsd rnsh rnx lxmd'")
        a = alpha.succeed(f"grep rpc_key {CLIENT}")
        b = beta.succeed(f"grep rpc_key {CLIENT}")
        assert a != b, "each machine has its own key"

    with subtest("the two machines find each other on the local network"):
        for m in (alpha, beta):
            m.wait_until_succeeds(
                AS_ALICE + "/etc/resilience-test/rnstatus --config /home/alice/rns | grep -E 'Peers +: 1 reachable'",
                timeout=120,
            )
            out = m.succeed(AS_ALICE + "/etc/resilience-test/rnstatus --config /home/alice/rns")
            print(out)
            assert "AutoInterface[Local network]" in out, out
            # Local network only: no TCP listener of any kind.
            m.fail("ss -H -tln | grep -v '127.0.0.1\\|::1' | grep .")

    with subtest("a signed message is delivered with its proof"):
        beta.succeed(f"({NODE} bob /home/alice/rns listen 150 > /tmp/bob.log 2>&1 &)")
        beta.wait_until_succeeds("grep -q 'address' /tmp/bob.log", timeout=60)
        bob = beta.succeed("cat /home/alice/rns/addr").strip()
        assert re.fullmatch(r"[0-9a-f]{32}", bob), bob
        out = alpha.succeed(f"{NODE} ann /home/alice/rns send {bob} 90 2>&1")
        print(out)
        assert "state delivered" in out and "RESULT delivered" in out, out
        ann = alpha.succeed("cat /home/alice/rns/addr").strip()
        got = beta.wait_until_succeeds("grep RECEIVED /tmp/bob.log", timeout=30)
        print(got)
        assert f"from={ann} verified=True content='hello from ann'" in got, got
        # Nothing was reported delivered before the proof came back.
        assert out.index("state outbound") < out.index("state delivered"), out

    with subtest("a message to a machine that is gone is never delivered"):
        beta.succeed("pkill -f 'node.p[y]'")
        beta.succeed("systemctl stop resilience-reticulum.service")
        rc, out = alpha.execute(f"{NODE} ann /home/alice/rns send {bob} 100 2>&1")
        print(out)
        assert "RESULT not delivered" in out and "state delivered" not in out, out
        assert rc == 4, rc

    with subtest("the key survives a restart; the instance comes back"):
        before = alpha.succeed(f"grep rpc_key {CLIENT}")
        alpha.succeed("systemctl restart resilience-reticulum.service")
        alpha.wait_for_unit("resilience-reticulum.service")
        assert alpha.succeed(f"grep rpc_key {CLIENT}") == before
        alpha.succeed("systemctl is-system-running --wait | grep -qx running")
  '';
}
