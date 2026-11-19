# Community network, server side (ADR 0025, slice 2): an administrator
# shares on ONE interface with `resilience-admin set-share`; nginx serves
# read-only, only GET/HEAD, only catalog packs, only world-readable
# files; kiwix-serve behind it; the other interface sees nothing; off
# closes the port; the NetworkManager connection going down stops it.
# Discovery (slice 3): near's account hears the server's Reticulum
# announce, with the addresses of the shared interface only.
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
  # Served under the name of a pack whose catalog hash (on near) differs.
  tampered =
    pkgs.runCommand "tampered_en_2026-01.zim" { }
      "head -c 4096 /dev/zero | tr '\\0' x > $out";
  tamperedEntry = {
    id = "tampered";
    title = "Tampered (test)";
    description = "The server sends other bytes";
    category = "practical";
    language = "en";
    file = "tampered_en_2026-01.zim";
    size = 4096;
    sha256 = "1111111111111111111111111111111111111111111111111111111111111111";
    license = "CC0-1.0";
    source = "Resilience test suite";
  };

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
      # Discovery (ADR 0025 §4) goes through local messaging.
      resilience.comms.enable = true;
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
        tamperedEntry
      ];
    };
  # A Resilience machine whose account hears the server's announce, and
  # gets packs from it through the TUI with an administrator's password.
  nodes.near =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    let
      # Its own catalog: the field notes with their real hash (known once
      # built), and the tampered pack with a hash the server's bytes miss.
      catalog = pkgs.runCommand "packs-near.toml" { } ''
        cat ${config.resilience.packs.configFile} > $out
        cat >> $out <<EOF
        [[pack]]
        id = "fieldnotes"
        title = "Field notes (test)"
        description = "A tiny pack built by the test"
        category = "practical"
        language = "en"
        file = "fieldnotes_en_2026-01.zim"
        size = $(stat -c %s ${zim})
        sha256 = "$(sha256sum ${zim} | cut -d' ' -f1)"
        license = "CC0-1.0"
        source = "Resilience test suite"
        EOF
        ${config.resilience.package}/bin/resilience-library check-config $out
      '';
    in
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      virtualisation.memorySize = 1024;
      virtualisation.cores = 1;
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users = {
        alice.mode = "ultra-stable";
        carol.mode = "advanced";
      };
      users.users.carol.initialPassword = "carol-pw";
      resilience.interface.autologinUser = "alice";
      resilience.comms.enable = true;
      resilience.share.enable = true;
      resilience.packs.catalog = lib.mkForce [ tamperedEntry ];
      environment.etc."resilience/packs.toml".source = lib.mkForce catalog;
      environment.systemPackages = [ pkgs.curl ];
    };
  nodes.far =
    { lib, ... }:
    {
      imports = [ client ];
      virtualisation.vlans = [ 2 ];
    };

  testScript = ''
    import base64, re, time as _time

    HELPER = "resilience-admin"

    def tty_text(m):
        """near's tty1 from /dev/vcsu1 (UTF-32); see modes.nix."""
        cols = int(m.wait_until_succeeds("stty -F /dev/tty1 size", timeout=10).split()[1])
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
                print(text)
                raise Exception(f"timed out waiting for {regexp!r}")
            _time.sleep(0.5)

    def select(m, regexp):
        """Move the list selection to the line matching `regexp`."""
        for _ in range(12):
            if re.search(r"> " + regexp, tty_text(m)):
                return
            m.send_key("down")
            _time.sleep(0.3)
        raise Exception(f"line {regexp!r} not found:\n" + tty_text(m))
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
    server.succeed(f"install -m 0644 -o resilience-packs ${tampered} {PACKS}/tampered_en_2026-01.zim")
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
        # Even with the firewall opened on eth2 by hand, the kernel keeps
        # nginx to eth1 (RestrictNetworkInterfaces).
        server.succeed("iptables -w -I resilience-share -i eth2 -p tcp --dport 80 -j nixos-fw-accept")
        assert get(far, f"http://{ip2}/") == "000", "nginx answered on eth2"
        server.succeed("iptables -w -D resilience-share -i eth2 -p tcp --dport 80 -j nixos-fw-accept")
        # Not root: a dynamic user.
        users = server.succeed("ps -o user= -C nginx").split()
        assert users and "root" not in users, users

    with subtest("discovery: near's account hears the server, with eth1's addresses"):
        near.wait_for_unit("resilience-reticulum.service")
        server.wait_for_unit("resilience-share-announce@eth1.service")
        shares = "/home/alice/.local/share/resilience/messages/shares.json"
        near.wait_until_succeeds(f"grep -q 'Test shelter' {shares}", timeout=180)
        import json
        heard = json.loads(near.succeed(f"cat {shares}"))
        assert len(heard) == 1, heard
        entry = list(heard.values())[0]
        assert entry["services"] == ["packs", "library", "files", "pages"], entry
        assert entry["port"] == 80, entry
        assert ip1 in entry["addresses"], entry
        assert ip2 not in entry["addresses"], entry
        assert all(a == ip1 or a.startswith("fe80:") for a in entry["addresses"]), entry
        # The announcer runs as a dynamic user.
        server.succeed("systemctl show -p DynamicUser --value resilience-share-announce@eth1.service | grep -qx yes")

    with subtest("joining through the TUI: packs checked, the library through the relay"):
        near.succeed("dmesg -n 1")
        wait_tty(near, "MAIN MENU")
        near.send_chars("/community")
        wait_tty(near, "Community network: share")
        near.send_key("ret")
        wait_tty(near, "COMMUNITY NETWORK")
        wait_tty(near, r'"Test shelter"\?', timeout=60)
        assert ip1 in tty_text(near), tty_text(near)
        select(near, r'"Test shelter"\?')
        near.send_key("ret")
        wait_tty(near, "NEARBY COMPUTER")
        select(near, "Get packs from it")
        near.send_key("ret")
        wait_tty(near, "PACKS FROM A NEARBY COMPUTER", timeout=40)
        for title in ("Field notes", "Tampered"):
            select(near, r"\[ \] " + title)
            near.send_chars(" ")
            wait_tty(near, r"\[\+\] " + title)
        near.send_key("ret")
        wait_tty(near, "GET PACKS")
        near.send_key("left")
        near.send_key("ret")
        wait_tty(near, "AUTHENTICATING FOR org.resilience.packs.change", timeout=30)
        wait_tty(near, "Choose identity|Password:")
        if "Choose identity" in tty_text(near):
            m = re.search(r"(\d)\. .*carol", tty_text(near))
            assert m, tty_text(near)
            near.send_chars(m.group(1) + "\n")
            wait_tty(near, "Password:")
        near.send_chars("carol-pw\n")
        wait_tty(near, r"Field notes \(test\): installed and checked", timeout=90)
        wait_tty(near, r"Tampered \(test\): the data received is not the expected", timeout=60)
        got = near.succeed(f"sha256sum {PACKS}/{ZIM}").split()[0]
        assert got == server.succeed(f"sha256sum {PACKS}/{ZIM}").split()[0]
        near.fail(f"test -e {PACKS}/tampered_en_2026-01.zim")
        near.fail(f"ls {PACKS}/.staging/tampered* {PACKS}/.staging/fieldnotes*")
        wait_tty(near, "Press Enter to return", timeout=30)
        near.send_key("ret")
        wait_tty(near, "NEARBY COMPUTER")
        # The library: lynx stays on localhost; the relay carries it.
        select(near, "Read its library")
        near.send_key("ret")
        wait_tty(near, "Press Enter to open them")
        near.send_key("ret")
        wait_tty(near, "Field notes", timeout=60)
        assert "JavaScript" not in tty_text(near), tty_text(near)
        args = near.succeed("pgrep -a -u alice lynx")
        assert "http://localhost:" in args and ip1 not in args, args
        assert "-localhost" in args, args
        near.send_chars("q")
        wait_tty(near, "Are you sure you want to quit")
        near.send_chars("y")
        wait_tty(near, "NEARBY COMPUTER")
        near.fail("pgrep -u alice lynx")
        # Shared folder (slice 5): a file copied into ~/Received under its
        # name, 0600; the private file is listed by nginx but refused (403),
        # and nothing is kept.
        select(near, "Files in its shared folder")
        near.send_key("ret")
        wait_tty(near, "SHARED FOLDER OF A NEARBY COMPUTER", timeout=40)
        for name, ok in (("notice.txt", True), ("private.txt", False)):
            select(near, re.escape(name))
            near.send_key("ret")
            wait_tty(near, "COPY A FILE")
            near.send_key("left")
            near.send_key("ret")
            wait_tty(near, "Saved as" if ok else "Nothing was kept", timeout=60)
            wait_tty(near, "Press Enter to return", timeout=30)
            near.send_key("ret")
            wait_tty(near, "SHARED FOLDER OF A NEARBY COMPUTER")
        got = near.succeed("cat /home/alice/Received/notice.txt")
        assert "sharedfileword" in got, got
        assert near.succeed("stat -c %a /home/alice/Received/notice.txt").strip() == "600"
        near.fail("ls /home/alice/Received/private* /home/alice/Received/.download-*")
        near.send_key("esc")
        wait_tty(near, "NEARBY COMPUTER")

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
        server.fail("systemctl is-active resilience-share-announce@eth1.service")
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
        # Not connected: refused, so that it can never follow the card
        # onto whatever network it joins next (review finding).
        rc, out = server.execute(f"{HELPER} set-share on eth2 --name Later pages 2>&1")
        assert rc != 0, out
        server.fail("systemctl is-active resilience-share@eth2.service")
        server.fail("test -e /run/resilience-share/state.toml")
        assert server.succeed("iptables -S resilience-share").strip() == "-N resilience-share"
  '';
}
