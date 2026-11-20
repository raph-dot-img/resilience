# Screenshots of the real interface for the README (docs/images/): the
# standard edition in QEMU at 1024x768 (a 128x48 console with the system's
# font and colours), driven like the VM tests, with the Luxembourg map pack
# built from the pinned OpenStreetMap extract. Not a check: nothing is
# asserted beyond waiting for each screen.
#
#   nix build .#screenshots -L      # -> result/*.png (look at them)
#   scripts/update-screenshots.sh   # copies them into docs/images/
{ self }:
{ lib, hostPkgs, ... }:
let
  pkgs = hostPkgs;
  lux = self.legacyPackages.x86_64-linux.mapPacks.luxembourg;
  luxFile = "map-osm-luxembourg_2026-01.mbtiles";
  # The catalog pins the published file; this VM gets the pack built from
  # this tree, so its entry takes that file's own size and hash (read at
  # evaluation; the store installs nothing here, the file is copied in).
  luxSize = lib.toInt (
    lib.fileContents (pkgs.runCommand "lux-size" { } "stat -c %s ${lux}/${luxFile} > $out")
  );
  luxSha = builtins.head (lib.splitString " " (lib.fileContents "${lux}/${luxFile}.sha256"));
in
{
  name = "resilience-screenshots";

  nodes.machine =
    { lib, ... }:
    {
      imports = [
        self.nixosModules.default
        ../nix/systems/common.nix
        ../nix/profiles/standard.nix
      ];
      virtualisation.memorySize = 2048;
      virtualisation.cores = 2;
      virtualisation.qemu.networkingOptions = lib.mkForce [ "-nic none" ];
      virtualisation.qemu.options = [ "-vga std" ];
      boot.kernelParams = [ "video=1024x768" ];
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users.alice.mode = "ultra-stable";
      resilience.interface.autologinUser = "alice";
      resilience.packs.catalog = map (
        p:
        if p.id == "map-luxembourg" then
          p
          // {
            size = luxSize;
            sha256 = luxSha;
          }
        else
          p
      ) (import ../nix/packs/kiwix.nix ++ import ../nix/packs/maps.nix);
      system.extraDependencies = [ lux ];
    };

  testScript = ''
    import base64, re, time as _time

    def tty_text():
        cols = int(machine.succeed("stty -F /dev/tty1 size").split()[1])
        raw = base64.b64decode(machine.succeed("base64 -w0 /dev/vcsu1"))
        text = raw.decode("utf-32-le", errors="replace")
        return "\n".join(text[i:i + cols] for i in range(0, len(text), cols))

    def wait(regexp, timeout=60):
        deadline = _time.monotonic() + timeout
        while True:
            t = tty_text()
            if re.search(regexp, t):
                return t
            if _time.monotonic() > deadline:
                print(t)
                raise Exception(f"timed out waiting for {regexp!r}")
            _time.sleep(0.5)

    def shot(name):
        # Let the screen settle (the status panel, the map's tiles).
        _time.sleep(3)
        print(f"===== {name} =====")
        print(tty_text())
        machine.screenshot(name)

    def home():
        for _ in range(4):
            machine.send_key("esc")
        wait("MAIN MENU")

    machine.start()
    machine.wait_for_unit("multi-user.target")
    machine.succeed("dmesg -n 1")
    machine.succeed("install -o resilience-packs -g resilience-packs -m 0644 ${lux}/${luxFile} /var/lib/resilience/packs/")
    wait("MAIN MENU")
    print(machine.succeed("stty -F /dev/tty1 size"))
    shot("01-main-menu")

    machine.send_chars("/library")
    wait("Offline library")
    machine.send_key("ret")
    wait("System documentation")
    shot("02-offline-library")
    home()

    machine.send_chars("/atlas")
    wait("Map")
    machine.send_key("ret")
    wait("Natural Earth")
    machine.send_chars("g")
    wait("GO TO")
    machine.send_chars("49.6106 6.1300\n")
    _time.sleep(2)
    shot("03-map-luxembourg")
    machine.send_chars("-")
    machine.send_chars("-")
    shot("04-map-luxembourg-wider")

    machine.send_chars("/")
    wait("FIND A PLACE")
    machine.send_chars("pharmacie\n")
    wait(r"Pharmacie", timeout=30)
    shot("05-map-search")
    machine.send_key("esc")
    machine.send_chars("+")
    machine.send_chars("+")
    machine.send_chars("n")
    wait("NEARBY")
    shot("06-map-nearby")
    machine.send_key("esc")
    machine.send_chars("q")
    home()

    machine.send_chars("/local mess")
    wait("Local messages")
    machine.send_key("ret")
    _time.sleep(5)
    shot("07-local-messages")
    home()
  '';
}
