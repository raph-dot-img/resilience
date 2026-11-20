# Maps & navigation (ADR 0016), driven through the real TUI on tty1 by an
# Ultra-Stable account, with no network: the built-in world overview opens
# in the map viewer from the menu and from the list of maps; zoom, the
# information screen, quit; the viewer runs as the user, writes only its
# last position and stays small.
#
# Then, on the same overview: search (`/`), what is near (`n`), go to
# coordinates (`g`), a spot height with its height, and an accented name
# drawn as such on the console (slices 10 to 12 and the accents). A
# detailed pack from a stick and hostile files come with slice 13.
{ self }:
{ ... }:
{
  name = "resilience-maps";

  nodes.machine =
    { lib, ... }:
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
      resilience.users.alice.mode = "ultra-stable";
      resilience.interface.autologinUser = "alice";
    };

  testScript = ''
    import base64, re, time as _time

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
                print(machine.execute("cat /home/alice/.local/state/resilience/tui.log")[1][-3000:])
                raise Exception(f"timed out waiting for {regexp!r} on tty{tty}")
            _time.sleep(0.5)

    def home():
        for _ in range(4):
            machine.send_key("esc")
        wait_tty("1", "MAIN MENU")

    READOUT = r"\d+\.\d{5} [NS]  \d+\.\d{5} [EW]"

    machine.start()
    machine.wait_for_unit("multi-user.target")
    machine.succeed("dmesg -n 1")
    wait_tty("1", "MAIN MENU")
    world = machine.succeed("grep -o '/nix/store/[^\"]*map-world[^\"]*mbtiles' /etc/resilience/packs.toml").strip()
    machine.succeed(f"test -f {world}")

    with subtest("Maps & navigation > Map opens on the world overview"):
        text = wait_tty("1", "MAPS & NAVIGATION")
        number = re.search(r"\[(\d)\] MAPS & NAVIGATION", text)
        assert number is not None, text
        machine.send_chars(number.group(1))
        wait_tty("1", "Maps on this computer")
        machine.send_key("ret")
        text = wait_tty("1", "Natural Earth")
        assert re.search(READOUT, text), text
        assert "World overview" in text, text
        # The whole world: land (black) and sea (blue) both on the screen.
        attrs = machine.succeed("od -An -v -tu1 -j4 /dev/vcsa1").split()[1::2]
        blue = sum(1 for a in attrs if (int(a) >> 4) & 7 == 1 or int(a) & 7 == 1)
        assert len(attrs) // 8 < blue < len(attrs) * 7 // 8, f"{blue} blue cells of {len(attrs)}"

        pid = machine.succeed("pgrep -u alice -x resilience-maps").strip()
        argv = machine.succeed(f"tr '\\0' ' ' < /proc/{pid}/cmdline").strip()
        assert argv.endswith(f"resilience-maps view --pack {world}"), argv
        # Zoom in four times: the scale bar changes, the position does not.
        before = re.search(r"┘ (\d+ k?m)", text)
        assert before is not None, text
        for _ in range(4):
            machine.send_chars("+")
        _time.sleep(2)
        text = wait_tty("1", READOUT)
        after = re.search(r"┘ (\d+ k?m)", text)
        assert after is not None and after.group(1) != before.group(1), text
        # Pan, then the information screen.
        machine.send_key("right")
        machine.send_key("down")
        machine.send_chars("i")
        wait_tty("1", "2022-05-13")
        machine.send_key("esc")
        wait_tty("1", READOUT)

        # Small, and no network socket.
        hwm = int(machine.succeed(f"awk '/VmHWM/ {{print $2}}' /proc/{pid}/status"))
        assert hwm < 200 * 1024, f"viewer peak memory {hwm} KiB"
        # (It has a local socket pair for terminal resize signals; no
        # TCP or UDP socket of any kind.)
        machine.succeed("ss -H -tuan -p > /tmp/sockets")
        machine.fail(f"grep -F 'pid={pid},' /tmp/sockets")

        machine.send_chars("q")
        wait_tty("1", "Maps on this computer")
        machine.fail("pgrep -x resilience-maps")
        # Only the last position was written, privately.
        pos = machine.succeed("find /home/alice -name maps.toml").strip()
        assert pos, "no position file"
        assert machine.succeed(f"stat -c '%U %a' {pos}").strip() == "alice 600"
        home()

    with subtest("the list of maps shows the built-in overview and opens it"):
        machine.send_chars("/maps on this")
        wait_tty("1", "Maps on this computer")
        machine.send_key("ret")
        text = wait_tty("1", "World overview map")
        assert "Built in" in text, text
        machine.send_key("ret")
        # The saved position: zoomed in, not the whole world.
        text = wait_tty("1", "Natural Earth")
        assert re.search(READOUT, text), text
        machine.send_chars("q")
        wait_tty("1", "World overview map")
        home()

    with subtest("search, nearby, go-to, spot heights and accents on the overview"):
        machine.send_chars("/atlas")
        wait_tty("1", "Map")
        machine.send_key("ret")
        wait_tty("1", "Natural Earth")
        # Search: Mount Everest, a summit with its height.
        machine.send_chars("/")
        wait_tty("1", "FIND A PLACE")
        machine.send_chars("everest\n")
        wait_tty("1", r"Mount Everest \(summit\)", timeout=30)
        machine.send_key("ret")
        text = wait_tty("1", r"27\.9\d{4} N  86\.\d{5} E")
        wait_tty("1", "Mount Everest 8848 m")
        # What is near: the summit itself, nearest first.
        machine.send_chars("n")
        wait_tty("1", "NEARBY")
        wait_tty("1", r"Mount Everest \(summit\)", timeout=30)
        machine.send_key("esc")
        # Go to coordinates: Zurich, its name drawn with the accent.
        machine.send_chars("g")
        wait_tty("1", "GO TO COORDINATES")
        machine.send_chars("47.3769 8.5417\n")
        text = wait_tty("1", r"47\.37\d{3} N  8\.54\d{3} E")
        for _ in range(6):
            machine.send_chars("-")
        wait_tty("1", "Zürich", timeout=30)
        # Searching without the accent finds it, shown with it.
        machine.send_chars("/")
        wait_tty("1", "FIND A PLACE")
        for _ in range(10):
            machine.send_key("backspace")
        machine.send_chars("zurich\n")
        wait_tty("1", r"Zürich \(city\)", timeout=30)
        machine.send_key("esc")
        wait_tty("1", "The crosshair is on")
        machine.send_chars("q")
        # Back in the interface before the next keys: keys typed within
        # milliseconds of quitting the viewer can stop the interface (known
        # bug, CLAUDE.md §15; found here on 2026-11-20).
        wait_tty("1", "Type to search")
        home()

    with subtest("found by searching, and the system is healthy"):
        machine.send_chars("/atlas")
        wait_tty("1", "Map")
        home()
        machine.succeed("systemctl is-system-running --wait | grep -qx running")
  '';
}
