# Resource packs and the offline library (ADR 0013), driven through the real
# TUI on tty1 by an Ultra-Stable account, with an Advanced account's
# password. A second machine is an HTTPS mirror (nginx, test certificate,
# slow on purpose). Covers: the store's identity and polkit checks, a pack
# copied from a hot-plugged USB stick, reading it in lynx (links, full-text
# search, no shell escape, no other address), a download interrupted with
# Ctrl-C and resumed, a file that does not match the catalog, a wrong
# password, and removing a pack. A map pack (.mbtiles, ADR 0016; fake bytes,
# the store never reads them) goes through the same install and removal,
# with its catalog fields, and is never offered to the text browser.
{ self }:
{ hostPkgs, ... }:
let
  pkgs = hostPkgs;
  zeroSize = 4194304;
  zeroSha = "bb9f8df61474d25e71fa00722318cd387396ca1736605e1248821cc0de3d3af8";

  # Test-only TLS certificate for the mirror (its key is no secret).
  testCert = pkgs.runCommand "test-mirror-cert" { nativeBuildInputs = [ pkgs.openssl ]; } ''
    mkdir $out
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
      -keyout $out/key.pem -out $out/cert.pem -days 3650 -subj /CN=mirror \
      -addext subjectAltName=DNS:mirror
  '';

  # A real ZIM file, built here (ZIM files are never committed).
  fieldnotes = pkgs.runCommand "fieldnotes-zim" { nativeBuildInputs = [ pkgs.zim-tools ]; } ''
    mkdir src
    cat > src/index.html <<'HTML'
    <html><head><meta charset="utf-8"><title>Field notes</title></head><body>
    <h1>Field notes</h1><p>A tiny test pack for Resilience OS.</p>
    <ul><li><a href="water.html">Drinking water</a></li><li><a href="fire.html">Fire</a></li></ul>
    </body></html>
    HTML
    cat > src/water.html <<'HTML'
    <html><head><meta charset="utf-8"><title>Drinking water</title></head><body>
    <h1>Drinking water</h1><p>Bring clear water to a rolling boil for one minute. Marker: pelicanwaterword.</p>
    <p>Outside: <a href="https://example.org/">example.org</a>.</p></body></html>
    HTML
    cat > src/fire.html <<'HTML'
    <html><head><meta charset="utf-8"><title>Fire</title></head><body><h1>Fire</h1><p>Keep it small.</p></body></html>
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

  # A "map pack": only its name, size and hash matter to the pack machinery.
  mapContent = "Not a real map: bytes for the Resilience vm-packs test.\n";
  mapFile = pkgs.writeText "map-osm-testland_2026-01.mbtiles" mapContent;
  mapCoverage = "Testland only (a test region): streets and places. Outside it, no detail. END-OF-COVERAGE";

  # A USB stick image with the pack in its Resilience-packs folder.
  stick =
    pkgs.runCommand "pack-stick.img"
      {
        nativeBuildInputs = [
          pkgs.dosfstools
          pkgs.mtools
        ];
      }
      ''
        truncate -s 16M $out
        mkfs.vfat -n PACKSTICK $out
        mmd -i $out ::/Resilience-packs
        mcopy -i $out ${zim} ::/Resilience-packs/
        mcopy -i $out ${mapFile} ::/Resilience-packs/map-osm-testland_2026-01.mbtiles
      '';
in
{
  name = "resilience-packs";

  nodes.mirror =
    { pkgs, ... }:
    let
      site = pkgs.runCommand "test-mirror" { } ''
        mkdir -p $out/zim
        head -c ${toString zeroSize} /dev/zero > $out/zim/zeros_2026-01.zim
        # Served under the name of a pack whose catalog hash differs.
        head -c 4096 /dev/urandom > $out/zim/tampered_2026-01.zim
      '';
    in
    {
      services.nginx = {
        enable = true;
        virtualHosts.mirror = {
          onlySSL = true;
          sslCertificate = "${testCert}/cert.pem";
          sslCertificateKey = "${testCert}/key.pem";
          root = site;
          # Slow, so the download can be interrupted half-way.
          extraConfig = ''
            limit_rate 256k;
            access_log /var/log/nginx/ranges.log ranges;
          '';
        };
        commonHttpConfig = ''
          log_format ranges '$request $status $http_range';
        '';
      };
      networking.firewall.allowedTCPPorts = [ 443 ];
    };

  nodes.machine =
    {
      config,
      lib,
      pkgs,
      nodes,
      ...
    }:
    let
      # The module's catalog plus the field notes entry, whose size and hash
      # are only known once the file is built.
      catalog = pkgs.runCommand "packs-with-fieldnotes.toml" { } ''
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
        urls = ["https://mirror/zim/fieldnotes_en_2026-01.zim"]
        license = "CC0-1.0"
        source = "Resilience test suite"
        notice = "Test notice: read with care."
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
      virtualisation.memorySize = 1536;
      virtualisation.cores = 2;
      virtualisation.qemu.options = [ "-device qemu-xhci,id=xhci" ];
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users = {
        alice.mode = "ultra-stable";
        carol.mode = "advanced";
      };
      resilience.interface.autologinUser = "alice";
      users.users.carol.initialPassword = "carol-pw";

      # Only the test packs: the real catalog's addresses are unreachable here.
      resilience.packs.catalog = lib.mkForce [
        {
          id = "zeros";
          title = "Zeros (test, slow download)";
          description = "4 MiB served slowly";
          category = "encyclopedia";
          language = "en";
          file = "zeros_2026-01.zim";
          size = zeroSize;
          sha256 = zeroSha;
          urls = [ "https://mirror/zim/zeros_2026-01.zim" ];
          license = "CC0-1.0";
          source = "Resilience test suite";
        }
        {
          id = "tampered";
          title = "Tampered (test)";
          description = "The mirror serves other bytes";
          category = "encyclopedia";
          language = "fr";
          file = "tampered_2026-01.zim";
          size = 4096;
          sha256 = zeroSha;
          urls = [ "https://mirror/zim/tampered_2026-01.zim" ];
          license = "CC0-1.0";
          source = "Resilience test suite";
        }
        {
          id = "map-osm-testland";
          title = "Testland (test map)";
          description = "A map pack with fake bytes";
          category = "maps";
          language = "en";
          file = "map-osm-testland_2026-01.mbtiles";
          size = builtins.stringLength mapContent;
          sha256 = builtins.hashString "sha256" mapContent;
          license = "ODbL-1.0";
          source = "Resilience test suite";
          data_date = "2026-01-01";
          coverage = mapCoverage;
          attribution = "© OpenStreetMap contributors";
        }
      ];
      environment.etc."resilience/packs.toml".source = lib.mkForce catalog;

      security.pki.certificateFiles = [ "${testCert}/cert.pem" ];
      networking.defaultGateway = nodes.mirror.networking.primaryIPAddress;
    };

  testScript = ''
    import base64, os, re, shutil, time as _time

    def tty_text(tty):
        """Screen text from /dev/vcsuN (UTF-32); see modes.nix."""
        cols = int(machine.wait_until_succeeds(f"stty -F /dev/tty{tty} size", timeout=10).split()[1])
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

    def authenticate(password):
        wait_tty("1", "AUTHENTICATING FOR org.resilience.packs.change", timeout=30)
        # The administrator sees the exact request (wrapped at 80 columns).
        wait_tty("1", "Request: /nix/store/")
        assert "resilience-pack-store change --" in tty_text("1").replace("\n", ""), tty_text("1")
        wait_tty("1", "Choose identity|Password:")
        if "Choose identity" in tty_text("1"):
            m = re.search(r"(\d)\. .*carol", tty_text("1"))
            assert m, tty_text("1")
            machine.send_chars(m.group(1) + "\n")
            wait_tty("1", "Password:")
        machine.send_chars(password + "\n")

    def back_to_packs():
        wait_tty("1", "Press Enter to return", timeout=120)
        machine.send_key("ret")
        wait_tty("1", "RESOURCE PACKS")

    def select(regexp):
        """Move the list selection to the line matching `regexp` (lists wrap)."""
        for _ in range(12):
            if re.search(r"> " + regexp, tty_text("1")):
                return
            machine.send_key("down")
            _time.sleep(0.3)
        raise Exception(f"line {regexp!r} not found:\n" + tty_text("1"))

    def quit_lynx(back="OFFLINE LIBRARY"):
        machine.send_chars("q")
        wait_tty("1", "Are you sure you want to quit")
        machine.send_chars("y")
        wait_tty("1", back)

    def tick(title):
        """Select the row whose text contains `title` (lists wrap) and tick it."""
        for _ in range(12):
            if re.search(r"> \[.\] " + re.escape(title), tty_text("1")):
                machine.send_chars(" ")
                return
            machine.send_key("down")
            _time.sleep(0.3)
        raise Exception(f"row {title!r} not found:\n" + tty_text("1"))

    store = machine.succeed("sed -n 's/^store = \"\\(.*\\)\"$/\\1/p' /etc/resilience/packs.toml").strip()
    library = "/var/lib/resilience/packs"

    start_all()
    mirror.wait_for_unit("nginx.service")
    machine.wait_for_unit("multi-user.target")
    machine.succeed("dmesg -n 1")
    wait_tty("1", "MAIN MENU")

    with subtest("store: its own account, an administrator's password, nothing else"):
        print(machine.succeed(f"stat -c '%U:%G %a' {library} {library}/.staging"))
        assert machine.succeed(f"stat -c '%U:%G %a' {library}").strip() == "resilience-packs:resilience-packs 755"
        for who in ["runuser -u alice --", ""]:
            rc, out = machine.execute(f"{who} {store} change --get zeros 2>&1 </dev/null")
            assert rc == 2 and "must run as resilience-packs" in out, (who, rc, out)
        rc, out = machine.execute(f"runuser -u resilience-packs -- {store} change --get ../x 2>&1")
        assert rc == 64, (rc, out)
        rc, out = machine.execute(f"runuser -u resilience-packs -- {store} change --get nope 2>&1 </dev/null")
        assert rc == 2 and "not in the catalog" in out, (rc, out)
        # Alice cannot write there herself.
        machine.fail(f"runuser -u alice -- touch {library}/x.zim")
        info = machine.succeed("pkaction --verbose --action-id org.resilience.packs.change")
        print(info)
        for kind in ["any", "inactive", "active"]:
            assert re.search(rf"implicit {kind}:\s+auth_admin\n", info), info
        pid = machine.succeed("pgrep -u alice -x resilience").split()[0]
        rc, out = machine.execute(f"pkcheck --action-id org.resilience.packs.change --process {pid} 2>&1")
        assert rc == 2, (rc, out)

    with subtest("TUI: the packs screen lists the catalog by category"):
        machine.send_chars("/choose what")
        wait_tty("1", "Resource packs: choose")
        machine.send_key("ret")
        wait_tty("1", "RESOURCE PACKS")
        wait_tty("1", "-- ENCYCLOPEDIA --")
        wait_tty("1", r"\[ \] Zeros \(test, slow download\)\s+4\.0 MiB")
        wait_tty("1", "-- PRACTICAL --")
        machine.send_chars("f")
        wait_tty("1", "language: en")
        assert "Tampered" not in tty_text("1")
        machine.send_chars("ff")
        wait_tty("1", "all languages")
        # The map pack: its own category, and its details in full.
        wait_tty("1", "-- MAPS --")
        select(r"\[ \] Testland \(test map\)")
        machine.send_key("ret")
        wait_tty("1", "PACK DETAILS")
        wait_tty("1", "END-OF-COVERAGE")
        wait_tty("1", "Data date: 2026-01-01")
        wait_tty("1", "Attribution: . OpenStreetMap contributors")
        machine.send_key("esc")
        wait_tty("1", "RESOURCE PACKS")

    with subtest("USB: a pack copied from a stick is checked and installed"):
        img = os.path.join(os.environ.get("TMPDIR", "/tmp"), "packstick.img")
        shutil.copy("${stick}", img)
        os.chmod(img, 0o644)
        machine.send_monitor_command(f"drive_add 0 if=none,id=stick1,format=raw,file={img}")
        machine.send_monitor_command("device_add usb-storage,bus=xhci.0,drive=stick1,id=usbstick1,removable=on")
        machine.wait_until_succeeds("test -b /dev/sda")
        machine.succeed("udevadm settle")
        machine.send_chars("u")
        wait_tty("1", "COPY PACKS FROM A USB STICK")
        wait_tty("1", "PACKSTICK")
        machine.send_key("ret")
        wait_tty("1", "Copy from the USB stick")
        wait_tty("1", r"\+ Field notes \(test\)")
        wait_tty("1", r"\+ Testland \(test map\)")
        machine.send_chars("y")
        authenticate("carol-pw")
        wait_tty("1", r"Field notes \(test\): installed and checked", timeout=60)
        wait_tty("1", r"Testland \(test map\): installed and checked", timeout=60)
        back_to_packs()
        wait_tty("1", r"\[\*\] Field notes \(test\)")
        wait_tty("1", r"\[\*\] Testland \(test map\)")
        out = machine.succeed(f"stat -c '%U %a %s' {library}/map-osm-testland_2026-01.mbtiles")
        assert out.strip() == "resilience-packs 644 ${toString (builtins.stringLength mapContent)}", out
        got = machine.succeed(f"sha256sum {library}/map-osm-testland_2026-01.mbtiles").split()[0]
        assert got == "${builtins.hashString "sha256" mapContent}", got
        out = machine.succeed(f"stat -c '%U %a %s' {library}/fieldnotes_en_2026-01.zim")
        assert out.startswith("resilience-packs 644 "), out
        want = machine.succeed("sed -n '/fieldnotes/,$ s/^sha256 = \"\\(.*\\)\"$/\\1/p' /etc/resilience/packs.toml").strip()
        got = machine.succeed(f"sha256sum {library}/fieldnotes_en_2026-01.zim").split()[0]
        assert want == got, (want, got)
        machine.succeed("journalctl -t resilience-pack-store | grep -q 'op=get caller=uid[0-9]* id=fieldnotes .*result=ok'")

    with subtest("library: read the pack in lynx, search it, no shell, no Internet"):
        for _ in range(3):
            machine.send_key("esc")
        wait_tty("1", "MAIN MENU")
        machine.send_chars("/library")
        wait_tty("1", "Offline library")
        machine.send_key("ret")
        wait_tty("1", "OFFLINE LIBRARY")
        # Built in, installed, and the copy still on the stick.
        wait_tty("1", r"Built in\s+System documentation \(built in\)")
        wait_tty("1", r"Practical\s+Field notes \(test\)")
        wait_tty("1", r"USB stick\s+Field notes \(test\)")
        # Maps are not read in the text browser: not listed, installed or not.
        assert "Testland" not in tty_text("1"), tty_text("1")
        select(r"Built in")
        machine.send_key("ret")
        wait_tty("1", "Offline documentation of this computer", timeout=60)
        quit_lynx()
        machine.send_chars("s")
        wait_tty("1", "Search in 'System documentation")
        machine.send_chars("virtualHosts\n")
        wait_tty("1", r"Results 1-\d+ of \d+", timeout=60)
        wait_tty("1", "from System documentation")
        quit_lynx()
        select(r"Practical")
        machine.send_key("ret")
        wait_tty("1", "Test notice: read with care")
        machine.send_key("ret")
        wait_tty("1", "A tiny test pack", timeout=60)
        # The server listens on the loopback address only.
        listen = machine.succeed("ss -Hltnp | grep kiwix-serve")
        print(listen)
        local = [l.split()[3] for l in listen.strip().splitlines()]
        assert local and all(re.fullmatch(r"127\.0\.0\.1:\d+", a) for a in local), listen
        # Lynx selects the first link ("Drinking water") on its own.
        machine.send_key("ret")
        wait_tty("1", "pelicanwaterword")
        # Shell escape and typing an address are disabled.
        machine.send_chars("!")
        wait_tty("1", "Spawning is currently disabled")
        machine.fail("pgrep -u alice -x bash")
        machine.fail("pgrep -u alice -x sh")
        machine.send_chars("g")
        _time.sleep(1)
        assert "URL to open" not in tty_text("1"), tty_text("1")
        machine.send_chars("q")
        wait_tty("1", "Are you sure you want to quit")
        machine.send_chars("y")
        wait_tty("1", "OFFLINE LIBRARY")
        machine.wait_until_fails("pgrep -x kiwix-serve")
        machine.send_chars("s")
        wait_tty("1", "Search in 'Field notes")
        machine.send_chars("pelicanwaterword\n")
        machine.send_key("ret")  # the notice
        wait_tty("1", r"Results 1-1 of 1", timeout=60)
        wait_tty("1", "Drinking water")
        quit_lynx()

    with subtest("library: a pack on the stick is checked and read without installing it"):
        select(r"USB stick")
        wait_tty("1", "not checked yet")
        machine.send_chars("c")
        wait_tty("1", "MATCHES: this is exactly the file of the catalog", timeout=60)
        machine.send_key("ret")
        wait_tty("1", "OFFLINE LIBRARY")
        select(r"USB stick")
        machine.send_key("ret")
        wait_tty("1", "Test notice: read with care")
        machine.send_key("ret")
        wait_tty("1", "A tiny test pack", timeout=60)
        # The server reads the file on the stick, as alice.
        ps = machine.succeed("ps -o user=,args= -C kiwix-serve")
        print(ps)
        assert ps.startswith("alice ") and "/run/media/alice/PACKSTICK/" in ps, ps
        quit_lynx()

    with subtest("library: a map pack is refused, never given to kiwix-serve"):
        lib_bin = store.replace("/bin/resilience-pack-store", "/bin/resilience-library")
        for f in [f"{library}/map-osm-testland_2026-01.mbtiles",
                  "/run/media/alice/PACKSTICK/Resilience-packs/map-osm-testland_2026-01.mbtiles"]:
            rc, out = machine.execute(f"runuser -u alice -- {lib_bin} open {f} 2>&1 </dev/null")
            assert rc == 2 and "Maps & navigation" in out, (f, rc, out)
        machine.fail("pgrep -x kiwix-serve")
        # Checking it against the catalog works as for any pack.
        rc, out = machine.execute(f"runuser -u alice -- {lib_bin} check /run/media/alice/PACKSTICK/Resilience-packs/map-osm-testland_2026-01.mbtiles 2>&1")
        assert rc == 0 and "MATCHES" in out, (rc, out)

    with subtest("download: a wrong password changes nothing"):
        machine.send_chars("p")
        wait_tty("1", "RESOURCE PACKS")
        tick("Zeros")
        wait_tty("1", r"\[\+\] Zeros")
        machine.send_chars("d")
        wait_tty("1", r"Download:")
        machine.send_chars("y")
        authenticate("not-the-password")
        wait_tty("1", "Not authorised", timeout=90)
        back_to_packs()
        machine.fail(f"ls {library}/.staging/zeros*")

    with subtest("download: interrupted with Ctrl-C, resumed, checked"):
        tick("Zeros")
        machine.send_chars("d")
        wait_tty("1", r"Download:")
        machine.send_chars("y")
        authenticate("carol-pw")
        wait_tty("1", r"of 4\.0 MiB", timeout=60)
        _time.sleep(3)
        machine.send_key("ctrl-c")
        wait_tty("1", "Press Enter to return", timeout=60)
        part = int(machine.succeed(f"stat -c %s {library}/.staging/zeros_2026-01.zim.part"))
        print("partial download:", part)
        assert 0 < part < ${toString zeroSize}, part
        machine.fail(f"test -e {library}/zeros_2026-01.zim")
        machine.send_key("ret")
        wait_tty("1", r"Zeros \(test, slow download\).*% done")
        tick("Zeros")
        machine.send_chars("d")
        wait_tty("1", r"Download:")
        machine.send_chars("y")
        authenticate("carol-pw")
        wait_tty("1", "continuing from", timeout=60)
        wait_tty("1", "Zeros \\(test, slow download\\): installed and checked", timeout=120)
        back_to_packs()
        got = machine.succeed(f"sha256sum {library}/zeros_2026-01.zim").split()[0]
        assert got == "${zeroSha}", got
        machine.fail(f"test -e {library}/.staging/zeros_2026-01.zim.part")
        ranges = mirror.succeed("cat /var/log/nginx/ranges.log")
        print(ranges)
        assert re.search(rf"GET /zim/zeros_2026-01\.zim HTTP/\S+ 206 bytes={part}-", ranges), ranges

    with subtest("download: bytes that do not match the catalog are deleted"):
        tick("Tampered")
        machine.send_chars("d")
        machine.send_chars("y")
        authenticate("carol-pw")
        wait_tty("1", "not the expected file", timeout=60)
        back_to_packs()
        machine.fail(f"ls {library}/tampered* {library}/.staging/tampered*")
        machine.succeed("journalctl -t resilience-pack-store | grep -q 'id=tampered .*result=mismatch'")

    with subtest("remove: a ticked installed pack is removed for everyone"):
        tick("Field notes")
        wait_tty("1", r"\[-\] Field notes")
        tick("Testland")
        wait_tty("1", r"\[-\] Testland")
        machine.send_chars("d")
        wait_tty("1", "Remove \\(for every account\\)")
        machine.send_chars("y")
        authenticate("carol-pw")
        wait_tty("1", "fieldnotes_en_2026-01.zim: removed", timeout=60)
        wait_tty("1", "map-osm-testland_2026-01.mbtiles: removed", timeout=60)
        back_to_packs()
        machine.fail(f"test -e {library}/fieldnotes_en_2026-01.zim")
        machine.fail(f"test -e {library}/map-osm-testland_2026-01.mbtiles")
        machine.succeed(f"test -e {library}/zeros_2026-01.zim")
        machine.succeed("systemctl is-system-running --wait | grep -qx running")
  '';
}
