# Community network, server side (ADR 0025, slice 2): an administrator
# shares on ONE interface with `resilience-admin set-share`; nginx serves
# read-only, only GET/HEAD, only catalog packs, only world-readable
# files; kiwix-serve behind it; the other interface sees nothing; off
# closes the port; the NetworkManager connection going down stops it.
#
#   near ── vlan 1 ── server ── vlan 2 ── far
{ self }:
{ hostPkgs, ... }:
let
  pkgs = hostPkgs;

  # A real ZIM file for the library (ZIM files are never committed).
  fieldnotes = pkgs.runCommand "fieldnotes-zim" { nativeBuildInputs = [ pkgs.zim-tools ]; } ''
    mkdir src
    cat > src/index.html <<'HTML'
    <html><head><meta charset="utf-8"><title>Field notes</title></head><body>
    <h1>Field notes</h1><p>Marker: sharedpelicanword.</p></body></html>
    HTML
    base64 -d > src/icon.png <<'PNG'
    iVBORw0KGgoAAAANSUhEUgAAADAAAAAwCAMAAABg3Am1AAAAIGNIUk0AAHomAACAhAAA+gAAAIDoAAB1MAAA6mAAADqYAAAXcJy6UTwAAAADUExURQCAAJz5pZEAAAAHdElNRQfqCw4TKisxs/wnAAAAJXRFWHRkYXRlOmNyZWF0ZQAyMDI2LTExLTE0VDE5OjQyOjQzKzAwOjAwhGtc7gAAACV0RVh0ZGF0ZTptb2RpZnkAMjAyNi0xMS0xNFQxOTo0Mjo0MyswMDowMPU25FIAAAAodEVYdGRhdGU6dGltZXN0YW1wADIwMjYtMTEtMTRUMTk6NDI6NDMrMDA6MDCiI8WNAAAAGUlEQVRIx+3BgQAAAADDoPlT3+AEVQEA8AwJMAAB0q0T2gAAAABJRU5ErkJggg==
    PNG
    mkdir $out
    zimwriterfs --welcome=index.html --illustration=icon.png --language=eng \
      --title="Field notes" --description="Test pack" --creator=Resilience \
      --publisher=Resilience --name=fieldnotes_en src $out/fieldnotes_en_2026-01.zim
  '';
  zim = "${fieldnotes}/fieldnotes_en_2026-01.zim";

  client =
    { lib, pkgs, ... }:
    {
      virtualisation.memorySize = 512;
      virtualisation.cores = 1;
      environment.systemPackages = [ pkgs.curl ];
    };
in
{
  name = "resilience-share";

  nodes.server =
    { lib, ... }:
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      virtualisation.memorySize = 1536;
      virtualisation.cores = 2;
      virtualisation.vlans = [
        1
        2
      ];
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users = {
        alice.mode = "ultra-stable";
        carol.mode = "advanced";
      };
      resilience.interface.autologinUser = "alice";
      users.users.carol.initialPassword = "carol-pw";
      resilience.share.enable = true;
      # The server never hashes: only the names in the catalog matter.
      resilience.packs.catalog = lib.mkForce [
        {
          id = "fieldnotes";
          title = "Field notes (test)";
          description = "A tiny pack built by the test";
          category = "practical";
          language = "en";
          file = "fieldnotes_en_2026-01.zim";
          size = 1;
          sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
          license = "CC0-1.0";
          source = "Resilience test suite";
        }
      ];
    };
  nodes.near = client;
  nodes.far =
    { lib, ... }:
    {
      imports = [ client ];
      virtualisation.vlans = [ 2 ];
    };

  testScript = ''
    HELPER = "resilience-admin"
    AS_ALICE = "runuser -u alice -- "
    PACKS = "/var/lib/resilience/packs"
    SHARED = "/var/lib/resilience/shared"
    ZIM = "fieldnotes_en_2026-01.zim"

    def get(m, url, extra=""):
        # HTTP status code only (000: no answer).
        return m.succeed(f"curl -s -o /dev/null -w '%{{http_code}}' --max-time 5 {extra} '{url}' || true").strip()

    start_all()
    server.wait_for_unit("multi-user.target")
    near.wait_for_unit("multi-user.target")
    far.wait_for_unit("multi-user.target")
    server.succeed("dmesg -n 1")
    ip1 = server.succeed("ip -4 -o addr show eth1 | awk '{print $4}' | cut -d/ -f1").strip()
    ip2 = server.succeed("ip -4 -o addr show eth2 | awk '{print $4}' | cut -d/ -f1").strip()
    near.wait_until_succeeds(f"ping -c 1 -W 2 {ip1}", timeout=60)
    far.wait_until_succeeds(f"ping -c 1 -W 2 {ip2}", timeout=60)

    # Content: a catalog pack, a pack not in the catalog, shared files.
    server.succeed(f"install -m 0644 -o resilience-packs ${zim} {PACKS}/{ZIM}")
    server.succeed(f"echo not listed > {PACKS}/notlisted_en.zim && chmod 0644 {PACKS}/notlisted_en.zim")
    server.succeed(f"echo 'public notice sharedfileword' > {SHARED}/files/notice.txt && chmod 0644 {SHARED}/files/notice.txt")
    server.succeed(f"echo 'private sharedsecretword' > {SHARED}/files/private.txt && chmod 0600 {SHARED}/files/private.txt")
    server.succeed(f"echo hidden > {SHARED}/files/.hidden && chmod 0644 {SHARED}/files/.hidden")
    server.succeed(f"ln -s /etc/os-release {SHARED}/files/link.txt")
    server.succeed(f"echo '<html><body><h1>Shelter news</h1><p>sharedpageword</p></body></html>' > {SHARED}/pages/index.html && chmod 0644 {SHARED}/pages/index.html")
    server.succeed(f"stat -c '%a %U %G' {SHARED}/files | grep -qx '2775 root wheel'")

    with subtest("nothing listens until an administrator shares"):
        server.fail("ss -Htln 'sport = :80' | grep -q .")
        assert get(near, f"http://{ip1}/") == "000", get(near, f"http://{ip1}/")
        server.fail("systemctl is-active 'resilience-share@*'")
        server.fail("test -e /run/resilience-share/state.toml")

    with subtest("set-share: administrators only, every argument checked, audited"):
        tui = server.wait_until_succeeds("pgrep -u alice -x resilience").split()[0]
        # auth_admin: never allowed without a password, even at the console.
        server.fail(f"pkcheck --action-id org.resilience.admin.set-share --process {tui}")
        rc, _ = server.execute(AS_ALICE + f"{HELPER} set-share on eth1 --name Test packs")
        assert rc == 77, rc
        for bad in (
            "on",
            "on eth1 packs",
            "on lo --name Test packs",
            "on eth9 --name Test packs",
            "on eth1 --name Test",
            "on eth1 --name Test shell",
            "on eth1 --name Test packs packs",
            "on 'eth1 eth2' --name Test packs",
            "on eth1 --name '-x' packs",
            "on 'eth1;id' --name Test packs",
            "off now",
        ):
            rc, out = server.execute(f"{HELPER} set-share {bad}")
            assert rc in (2, 64), (bad, rc, out)
        server.succeed("grep -q 'op=set-share.*result=refused' /var/log/resilience/audit.log")
        server.fail("systemctl is-active 'resilience-share@*'")

    with subtest("on: served on eth1 only"):
        out = server.succeed(f"{HELPER} set-share on eth1 --name 'Test shelter' packs library files pages")
        assert "Sharing" in out and "eth1" in out, out
        server.succeed("grep -q 'op=set-share.*result=ok' /var/log/resilience/audit.log")
        server.wait_for_unit("resilience-share-web@eth1.service")
        server.wait_for_unit("resilience-share-library@eth1.service")
        state = server.succeed("cat /run/resilience-share/state.toml")
        assert 'interface = "eth1"' in state and "Test shelter" in state, state
        near.wait_until_succeeds(f"curl -sf --max-time 5 http://{ip1}/ | grep -q 'Test shelter'", timeout=30)
        assert get(far, f"http://{ip2}/") == "000", "served on the other network"
        # Not root: a dynamic user.
        users = server.succeed("ps -o user= -C nginx").split()
        assert users and "root" not in users, users

    with subtest("packs: catalog files only, the exact bytes"):
        index = near.succeed(f"curl -sf http://{ip1}/")
        assert ZIM in index and "notlisted" not in index, index
        near.succeed(f"curl -sf -o /tmp/p.zim http://{ip1}/packs/{ZIM}")
        assert near.succeed("sha256sum /tmp/p.zim").split()[0] == server.succeed(f"sha256sum {PACKS}/{ZIM}").split()[0]
        # Resuming works (ranges).
        assert get(near, f"http://{ip1}/packs/{ZIM}", "-r 10-20") == "206", get(near, f"http://{ip1}/packs/{ZIM}", "-r 10-20")
        assert get(near, f"http://{ip1}/packs/notlisted_en.zim") in ("404", "403"), get(near, f"http://{ip1}/packs/notlisted_en.zim")
        assert get(near, f"http://{ip1}/packs/") == "404", get(near, f"http://{ip1}/packs/")
        assert get(near, f"http://{ip1}/packs/../../../etc/passwd", "--path-as-is") in ("400", "404"), get(near, f"http://{ip1}/packs/../../../etc/passwd", "--path-as-is")

    with subtest("library: kiwix-serve behind nginx"):
        near.wait_until_succeeds(f"curl -sf --max-time 10 http://{ip1}/library/ | grep -q .", timeout=60)
        server.fail("ss -Htln 'sport = :81' | grep -v 127.0.0.1 | grep -q .")
        out = near.succeed(f"curl -sfL --max-time 10 'http://{ip1}/library/search?pattern=sharedpelicanword&books.name=fieldnotes_en' || curl -sfL --max-time 10 'http://{ip1}/library/content/fieldnotes_en_2026-01/'")
        assert "sharedpelicanword" in out or "Field notes" in out, out[:500]

    with subtest("files and pages: world-readable files only, no dotfiles, no links"):
        listing = near.succeed(f"curl -sf http://{ip1}/files/")
        assert "notice.txt" in listing, listing
        near.succeed(f"curl -sf http://{ip1}/files/notice.txt | grep -q sharedfileword")
        assert get(near, f"http://{ip1}/files/private.txt") == "403", get(near, f"http://{ip1}/files/private.txt")
        assert get(near, f"http://{ip1}/files/.hidden") == "404", get(near, f"http://{ip1}/files/.hidden")
        assert get(near, f"http://{ip1}/files/link.txt") in ("403", "404"), get(near, f"http://{ip1}/files/link.txt")
        near.succeed(f"curl -sf http://{ip1}/pages/ | grep -q sharedpageword")
        headers = near.succeed(f"curl -sfI http://{ip1}/pages/")
        assert "default-src 'none'" in headers and "nosniff" in headers, headers
        assert "nginx/" not in headers, headers

    with subtest("read-only: GET and HEAD only"):
        for method in ("POST", "PUT", "DELETE", "OPTIONS"):
            code = get(near, f"http://{ip1}/files/notice.txt", f"-X {method} --data x")
            assert code in ("405", "413"), (method, code)
        server.succeed(f"test ! -e {SHARED}/files/x")

    with subtest("a firewall reload keeps sharing on eth1 only"):
        server.succeed("systemctl reload firewall")
        near.wait_until_succeeds(f"curl -sf --max-time 5 http://{ip1}/ | grep -q 'Test shelter'", timeout=30)
        assert get(far, f"http://{ip2}/") == "000", get(far, f"http://{ip2}/")

    with subtest("off: the port is closed again, the state is gone"):
        out = server.succeed(f"{HELPER} set-share off")
        assert "off" in out, out
        server.fail("systemctl is-active resilience-share@eth1.service")
        server.fail("systemctl is-active resilience-share-web@eth1.service")
        server.fail("systemctl is-active resilience-share-library@eth1.service")
        server.fail("test -e /run/resilience-share/state.toml")
        assert server.succeed("iptables -S resilience-share").strip() == "-N resilience-share"
        assert get(near, f"http://{ip1}/") == "000", get(near, f"http://{ip1}/")

    with subtest("only the chosen services; another interface replaces the first"):
        server.succeed(f"{HELPER} set-share on eth1 --name 'Packs only' packs")
        near.wait_until_succeeds(f"curl -sf --max-time 5 http://{ip1}/ | grep -q 'Packs only'", timeout=30)
        assert get(near, f"http://{ip1}/library/") == "404", get(near, f"http://{ip1}/library/")
        assert get(near, f"http://{ip1}/files/notice.txt") == "404", get(near, f"http://{ip1}/files/notice.txt")
        server.fail("systemctl is-active resilience-share-library@eth1.service")
        server.succeed(f"{HELPER} set-share on eth2 --name 'Far side' pages")
        far.wait_until_succeeds(f"curl -sf --max-time 5 http://{ip2}/pages/ | grep -q sharedpageword", timeout=30)
        assert get(near, f"http://{ip1}/") == "000", get(near, f"http://{ip1}/")
        server.fail("systemctl is-active resilience-share@eth1.service")

    with subtest("the interface's connection goes down: sharing stops"):
        server.succeed("nmcli device disconnect eth2")
        server.wait_until_fails("systemctl is-active resilience-share@eth2.service", timeout=30)
        server.fail("test -e /run/resilience-share/state.toml")
        assert server.succeed("iptables -S resilience-share").strip() == "-N resilience-share"
  '';
}
