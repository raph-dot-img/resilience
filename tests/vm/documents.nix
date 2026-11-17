# Manual pages and "Find & view files" (Phase 3), driven through the real
# TUI on tty1 by an Ultra-Stable account: a text file, a CSV table, a real
# PDF (made with Ghostscript at build time), a zip archive (listed, never
# extracted), file details with SHA-256; then a manual page in less's secure
# mode (no shell escape) and a search of the manuals.
{ self }:
{ hostPkgs, ... }:
let
  pkgs = hostPkgs;
  docs =
    pkgs.runCommand "test-documents"
      {
        nativeBuildInputs = [
          pkgs.ghostscript
          pkgs.libarchive
        ];
      }
      ''
        mkdir -p $out/Notes $out/Data src
        printf 'Boil clear water for one minute.\nplanmarkerword\n' > "$out/Notes/Water plan.md"
        printf 'item,cost,notes\nrice,3,"dry, keep"\nbeans,2,\n' > $out/Data/budget.csv
        cat > a.ps <<'PS'
        %!PS
        /Helvetica findfont 14 scalefont setfont
        72 700 moveto (pdfmarkerword in a real PDF) show
        showpage
        PS
        ps2pdf a.ps $out/Data/manual.pdf
        echo inside > src/inside.txt
        bsdtar -a -cf $out/Data/photos.zip -C src inside.txt
      '';
in
{
  name = "resilience-documents";

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
                raise Exception(f"timed out waiting for {regexp!r} on tty{tty}")
            _time.sleep(0.5)

    def find(name):
        """In Find & view files: replace the query with `name` and open the file."""
        for _ in range(12):
            machine.send_key("backspace")
        machine.send_chars(name)
        wait_tty("1", r"FILES \(1\)")
        machine.send_key("ret")

    def home():
        for _ in range(4):
            machine.send_key("esc")
        wait_tty("1", "MAIN MENU")

    machine.start()
    machine.wait_for_unit("multi-user.target")
    machine.succeed("dmesg -n 1")
    wait_tty("1", "MAIN MENU")
    machine.succeed("cp -r ${docs}/. /home/alice/ && chown -R alice: /home/alice/Notes /home/alice/Data && chmod -R u+w /home/alice/Notes /home/alice/Data")
    want = machine.succeed("sha256sum /home/alice/Data/photos.zip").split()[0]

    with subtest("files: found by name, shown inside the interface, never run"):
        machine.send_chars("/find")
        wait_tty("1", "Find & view files")
        machine.send_key("ret")
        wait_tty("1", "FIND FILES")
        find("plan")
        wait_tty("1", "planmarkerword")
        machine.send_key("esc")
        find("budget")
        wait_tty("1", r"item\s+\| cost \| notes")
        wait_tty("1", r"rice\s+\| 3\s+\| dry, keep")
        machine.send_key("esc")
        find("manual")
        wait_tty("1", "pdfmarkerword in a real PDF", timeout=60)
        machine.send_key("esc")
        find("photos")
        wait_tty("1", "nothing is extracted")
        wait_tty("1", "inside.txt")
        machine.send_key("esc")
        machine.fail("test -e /home/alice/inside.txt -o -e /home/alice/Data/inside.txt")
        # F3: details with the SHA-256.
        machine.send_key("f3")
        wait_tty("1", "FILE DETAILS")
        # The hash wraps inside the dialog: compare hex digits only.
        assert want in re.sub(r"[^0-9a-f]", "", tty_text("1")), tty_text("1")
        machine.send_key("ret")
        home()

    with subtest("manual pages: secure pager, search"):
        machine.send_chars("/manual pages")
        wait_tty("1", "Manual pages")
        machine.send_key("ret")
        wait_tty("1", "MANUAL PAGES")
        machine.send_chars("ls\n")
        wait_tty("1", r"LS\(1\)", timeout=60)
        out = machine.succeed("cat /proc/$(pgrep -u alice -x less)/environ | tr '\\0' '\\n'")
        assert "LESSSECURE=1" in out, out
        machine.send_chars("!")
        wait_tty("1", "Command not available")
        machine.fail("pgrep -u alice -x bash")
        machine.fail("pgrep -u alice -x sh")
        machine.send_chars("q")
        wait_tty("1", "MAIN MENU|Manual pages")
        home()
        machine.send_chars("/manual pages")
        wait_tty("1", "Manual pages")
        machine.send_key("ret")
        wait_tty("1", "MANUAL PAGES")
        machine.send_chars("copy files\n")
        wait_tty("1", r"cp \(1\)", timeout=60)
        machine.succeed("systemctl is-system-running --wait | grep -qx running")
  '';
}
