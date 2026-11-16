# Local messaging network (ADR 0018, slice 1): two machines on an isolated
# network with no Internet and no DNS. On each, the opt-in Reticulum
# service runs unprivileged and the two find each other; a test program
# run by an ordinary account attaches to its machine's instance with the
# client configuration and sends a signed LXMF message, with its delivery
# proof, to the other machine. A message to a machine that has gone is never "delivered".
#
# Then each account's messaging program (resilience-messages, a systemd
# user service): a request file in the outbox is delivered to the other
# machine with its proof; with the other program stopped it waits and
# arrives later. Last, through the interface on both consoles: people
# nearby, writing, "delivered" on one screen and the text on the other.
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
      # Logged in on tty1: the account's messaging program runs.
      resilience.interface.autologinUser = "alice";
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
    import base64, json, re, time as _time

    def tty_text(m):
        """Screen text of tty1 from /dev/vcsu1 (UTF-32); see modes.nix."""
        cols = int(m.succeed("stty -F /dev/tty1 size").split()[1])
        raw = base64.b64decode(m.succeed("base64 -w0 /dev/vcsu1"))
        text = raw.decode("utf-32-le", errors="replace")
        return "\n".join(text[i:i + cols] for i in range(0, len(text), cols))

    def wait_tty(m, regexp, timeout=60):
        deadline = _time.monotonic() + timeout
        while True:
            text = tty_text(m)
            if re.search(regexp, text):
                return text
            if _time.monotonic() > deadline:
                print("--- tty1 contents ---")
                print(text)
                raise Exception(f"timed out waiting for {regexp!r} on tty1")
            _time.sleep(0.5)

    def open_messages(m):
        for _ in range(4):
            m.send_key("esc")
        wait_tty(m, "MAIN MENU")
        m.send_chars("/local mess")
        wait_tty(m, "Local messages")
        m.send_key("ret")
        wait_tty(m, "LOCAL MESSAGES")

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

    with subtest("each account's messaging program: queued, delivered with proof, received"):
        BOX = "/home/alice/.local/share/resilience/messages"
        for m in (alpha, beta):
            m.wait_until_succeeds(f"test -s {BOX}/address", timeout=120)
            assert m.succeed(f"stat -c '%U %a' {BOX}").strip() == "alice 700"
            assert m.succeed(f"stat -c '%U %a' {BOX}/identity").strip() == "alice 600"
            m.succeed("pgrep -u alice -f 'resilience_messages.p[y] run'")
            m.fail("pgrep -u root -f 'resilience_messages.p[y]'")
        a_addr = alpha.succeed(f"cat {BOX}/address").strip()
        b_addr = beta.succeed(f"cat {BOX}/address").strip()
        assert a_addr != b_addr

        def send(m, mid, to, text):
            req = json.dumps({"to": to, "text": text})
            m.succeed(AS_ALICE + f"sh -c 'cat > {BOX}/outbox/{mid}.json.tmp && mv {BOX}/outbox/{mid}.json.tmp {BOX}/outbox/{mid}.json' <<'END'\n{req}\nEND")

        def record(m, mid):
            return json.loads(m.succeed(f"cat {BOX}/messages/{mid}.json"))

        send(alpha, "m1", b_addr, "Water at the school, 9h")
        alpha.wait_until_succeeds(f"grep -q '\"state\": \"delivered\"' {BOX}/messages/m1.json", timeout=120)
        # The signature is checked on arrival, or a moment later when the
        # sender's key was not known yet.
        beta.wait_until_succeeds(f"grep -q '\"verified\": true' {BOX}/messages/in-*.json", timeout=60)
        got = [json.loads(line) for line in beta.succeed(f"cat {BOX}/messages/in-*.json").splitlines()]
        assert len(got) == 1, got
        assert got[0]["state"] == "received" and got[0]["peer"] == a_addr, got
        assert got[0]["text"] == "Water at the school, 9h" and got[0]["verified"] is True, got
        # Each has heard the other, with the name it claims.
        peers = json.loads(alpha.succeed(f"cat {BOX}/peers.json"))
        assert peers[b_addr]["name"] == "alice", peers

        # The other machine's program stops: the message waits, it is not
        # delivered and not lost; when the program is back it arrives.
        beta.succeed("systemctl --user -M alice@ stop resilience-messages.service")
        send(alpha, "m2", b_addr, "second message")
        alpha.wait_until_succeeds(f"test -e {BOX}/messages/m2.json", timeout=30)
        alpha.sleep(20)
        assert record(alpha, "m2")["state"] in ("queued", "sent"), record(alpha, "m2")
        beta.succeed("systemctl --user -M alice@ start resilience-messages.service")
        alpha.wait_until_succeeds(f"grep -q '\"state\": \"delivered\"' {BOX}/messages/m2.json", timeout=180)
        beta.wait_until_succeeds(f"grep -l 'second message' {BOX}/messages/in-*.json", timeout=30)
        # The private key and the messages are the account's only.
        for m in (alpha, beta):
            m.fail(f"find {BOX} -perm /077 | grep .")

    with subtest("through the interface: written on one screen, read on the other"):
        for m in (alpha, beta):
            m.succeed("dmesg -n 1")
            text = wait_tty(m, "MAIN MENU")
            assert "COMMUNICATIONS" in text, text
        open_messages(alpha)
        text = wait_tty(alpha, "Messaging is running")
        # The two messages sent above are one conversation, under the name
        # the other machine claims (quoted and questioned).
        assert '"alice"?' in text and "You: second message" in text, text
        alpha.send_chars("m")
        text = wait_tty(alpha, "MY ADDRESS")
        assert " ".join(a_addr[i:i + 4] for i in range(0, 32, 4)) in text, text
        alpha.send_key("ret")
        # People nearby: the other machine, with the name it claims.
        alpha.send_chars("p")
        text = wait_tty(alpha, "PEOPLE NEARBY")
        text = wait_tty(alpha, re.escape('"alice"?'))
        spoken = " ".join(b_addr[i:i + 4] for i in range(0, 32, 4))
        assert spoken in text, text
        # (The test program of the subtest above is listed too, as "bob".)
        alpha.send_key("esc")
        wait_tty(alpha, "LOCAL MESSAGES")
        # Write to the address as it would be read aloud.
        alpha.send_chars("n")
        wait_tty(alpha, "WRITE TO AN ADDRESS")
        alpha.send_chars(spoken.upper() + "\n")
        wait_tty(alpha, "WRITE A MESSAGE")
        alpha.send_chars("Bring the blue tarp\n")
        text = wait_tty(alpha, r"You \[delivered\][^\n]*\n[^\n]*Bring the blue tarp", timeout=120)
        assert "[FAILED]" not in text and "[waiting]" not in text, text

        open_messages(beta)
        wait_tty(beta, re.escape('"alice"?'))
        beta.send_key("ret")
        wait_tty(beta, "MESSAGES WITH")
        text = wait_tty(beta, "Bring the blue tarp", timeout=60)
        assert re.search(r"Them \[from them\][^\n]*\n[^\n]*Bring the blue tarp", text), text
        assert "from them, NOT VERIFIED]" not in text, text
        # A reply from that screen.
        beta.send_chars("w")
        wait_tty(beta, "WRITE A MESSAGE")
        beta.send_chars("On my way\n")
        wait_tty(beta, r"You \[delivered\][^\n]*\n[^\n]*On my way", timeout=120)
        wait_tty(alpha, r"Them \[from them\][^\n]*\n[^\n]*On my way", timeout=60)
        for m in (alpha, beta):
            for _ in range(4):
                m.send_key("esc")
            wait_tty(m, "MAIN MENU")

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
