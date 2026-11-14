# Removable media and encrypted backups (ADR 0008), driven through the real
# TUI on tty1 by an Ultra-Stable account: a virtual USB stick is hot-plugged
# through the QEMU monitor, opened without a password (while an internal
# disk is neither listed nor mountable without an administrator), backed up
# with an age passphrase, ejected, unplugged, plugged back in, checked,
# restored (wrong passphrase first), and finally damaged on purpose.
{ self }:
{
  name = "resilience-backup";

  nodes.machine =
    { lib, pkgs, ... }:
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      virtualisation.memorySize = 1536;
      virtualisation.cores = 2;
      virtualisation.qemu.networkingOptions = lib.mkForce [ "-nic none" ];
      # A USB controller to hot-plug the stick into, and an internal disk.
      virtualisation.qemu.options = [ "-device qemu-xhci,id=xhci" ];
      virtualisation.emptyDiskImages = [ 64 ];
      users.users.root.hashedPassword = lib.mkForce null;
      resilience.users.alice.mode = "ultra-stable";
      resilience.interface.autologinUser = "alice";
      # Test only: to format the virtual stick.
      environment.systemPackages = [ pkgs.dosfstools ];
    };

  testScript = ''
    import base64, os, re, time as _time

    def tty_text(tty):
        """Screen text from /dev/vcsuN (UTF-32); see modes.nix."""
        cols = int(machine.succeed(f"stty -F /dev/tty{tty} size").split()[1])
        raw = base64.b64decode(machine.succeed(f"base64 -w0 /dev/vcsu{tty}"))
        text = raw.decode("utf-32-le", errors="replace")
        return "\n".join(text[i:i + cols] for i in range(0, len(text), cols))

    def wait_tty(tty, regexp, timeout=60):
        deadline = _time.monotonic() + timeout
        while True:
            text = tty_text(tty)
            if re.search(regexp, text):
                return
            if _time.monotonic() > deadline:
                print(f"--- tty{tty} contents ---")
                print(text)
                raise Exception(f"timed out waiting for {regexp!r} on tty{tty}")
            _time.sleep(0.5)

    stick = os.path.join(os.environ.get("TMPDIR", "/tmp"), "stick.img")
    with open(stick, "wb") as f:
        f.truncate(128 * 1024 * 1024)
    plugs = 0

    def plug():
        global plugs
        plugs += 1
        machine.send_monitor_command(f"drive_add 0 if=none,id=stick{plugs},format=raw,file={stick}")
        machine.send_monitor_command(f"device_add usb-storage,bus=xhci.0,drive=stick{plugs},id=usbstick{plugs},removable=on")
        machine.wait_until_succeeds("test -b /dev/sda")
        machine.succeed("udevadm settle")

    def unplug():
        machine.send_monitor_command(f"device_del usbstick{plugs}")
        machine.wait_until_succeeds("! test -b /dev/sda")

    media = "/run/media/alice/BACKUPSTICK"
    backups = media + "/Resilience-backups"
    files = "'Notes/first note.md' photo.bin"

    machine.start()
    machine.wait_for_unit("multi-user.target")
    wait_tty("1", "MAIN MENU")

    with subtest("alice has files worth saving"):
        machine.succeed("""runuser -u alice -- sh -s <<'EOS'
    set -e
    cd ~
    mkdir -p Notes .cache
    printf 'MARKER-7c1e first note\n' > 'Notes/first note.md'
    head -c 1048576 /dev/urandom > photo.bin
    printf cache > .cache/junk
    EOS""")
        before = machine.succeed(f"cd /home/alice && sha256sum {files}")

    with subtest("polkit: removable media for the console session, internal disks need an administrator"):
        machine.succeed("systemctl cat udisks2.service")
        machine.fail("systemctl is-active udisks2.service")  # D-Bus activated
        pid = machine.succeed("pgrep -u alice -x resilience").split()[0]
        machine.succeed(f"pkcheck --action-id org.freedesktop.udisks2.filesystem-mount --process {pid}")
        rc, out = machine.execute(f"pkcheck --action-id org.freedesktop.udisks2.filesystem-mount-system --process {pid} 2>&1")
        print(out)
        assert rc != 0, (rc, out)
        machine.succeed("mkfs.ext4 -q -L INTERNAL /dev/vdb && udevadm settle")

    with subtest("a hot-plugged stick is listed and opened without a password; the internal disk is not listed"):
        plug()
        machine.succeed("echo ',,c' | sfdisk -q /dev/sda && udevadm settle")
        machine.succeed("mkfs.vfat -n BACKUPSTICK /dev/sda1 && udevadm settle")
        machine.wait_until_succeeds("lsblk -no LABEL /dev/sda1 | grep -qx BACKUPSTICK")
        machine.send_chars("/backup")
        wait_tty("1", "Backups & USB sticks")
        machine.send_key("ret")
        wait_tty("1", "BACKUPS & USB STICKS")
        wait_tty("1", "BACKUPSTICK")
        screen = tty_text("1")
        assert "INTERNAL" not in screen and "vdb" not in screen, screen
        machine.send_key("ret")
        wait_tty("1", "BACKUPS ON BACKUPSTICK")
        opts = machine.succeed(f"findmnt -n -o OPTIONS {media}")
        print(opts)
        assert "nosuid" in opts and "nodev" in opts, opts
        machine.succeed(f"test \"$(stat -c %U {media})\" = alice")

    with subtest("backup: age asks for the passphrase on the console"):
        machine.send_key("ret")  # + Make a new backup of my files
        wait_tty("1", "NEW BACKUP")
        wait_tty("1", "not even you")
        machine.send_chars("y")
        wait_tty("1", "Enter passphrase")
        machine.send_chars("correct horse battery\n")
        wait_tty("1", "Confirm passphrase")
        machine.send_chars("correct horse battery\n")
        wait_tty("1", "Backup complete", timeout=120)
        wait_tty("1", "Press Enter to return")
        names = machine.succeed(f"ls {backups}").split()
        print(names)
        assert len(names) == 2, names
        assert re.fullmatch(r"home-\d{8}-\d{6}\.tar\.zst\.age", names[0]), names
        assert names[1] == names[0] + ".sha256", names
        # Standard tools can verify it; no plaintext reached the stick.
        machine.succeed(f"cd {backups} && sha256sum -c {names[1]}")
        machine.succeed(f"head -c 21 {backups}/{names[0]} | grep -q age-encryption.org")
        machine.succeed("sync")
        machine.fail("grep -a -q MARKER-7c1e /dev/sda")
        machine.send_key("ret")
        wait_tty("1", "BACKUPS ON BACKUPSTICK")
        wait_tty("1", "UTC")

    with subtest("eject, unplug and plug the stick back in"):
        machine.send_chars("e")
        wait_tty("1", "SAFE TO UNPLUG")
        machine.fail(f"findmnt {media}")
        machine.send_key("ret")
        unplug()
        plug()
        machine.wait_until_succeeds("lsblk -no LABEL /dev/sda1 | grep -qx BACKUPSTICK")
        machine.send_chars("r")
        wait_tty("1", "BACKUPSTICK")
        machine.send_key("ret")
        wait_tty("1", "BACKUPS ON BACKUPSTICK")
        wait_tty("1", "UTC")

    with subtest("check: the copy read back from the stick is intact"):
        machine.send_key("down")
        machine.send_chars("c")
        wait_tty("1", "This copy is intact", timeout=60)
        machine.send_key("ret")
        wait_tty("1", "BACKUPS ON BACKUPSTICK")

    with subtest("restore with a wrong passphrase restores nothing"):
        machine.send_key("ret")
        wait_tty("1", "RESTORE A BACKUP")
        wait_tty("1", "You will need this")
        machine.send_chars("y")
        wait_tty("1", "Enter passphrase")
        machine.send_chars("not the passphrase\n")
        wait_tty("1", "Nothing was restored", timeout=120)
        machine.succeed("test -z \"$(ls -A /home/alice/Restored)\"")
        machine.send_key("ret")
        wait_tty("1", "BACKUPS ON BACKUPSTICK")

    with subtest("restore into a new folder; originals untouched; cache not saved"):
        machine.send_key("ret")
        wait_tty("1", "RESTORE A BACKUP")
        machine.send_chars("y")
        wait_tty("1", "Enter passphrase")
        machine.send_chars("correct horse battery\n")
        wait_tty("1", "Your files were restored into", timeout=120)
        dest = machine.succeed("ls -d /home/alice/Restored/backup-*").split()
        assert len(dest) == 1, dest
        assert machine.succeed(f"cd {dest[0]} && sha256sum {files}") == before
        assert machine.succeed(f"cd /home/alice && sha256sum {files}") == before
        machine.fail(f"test -e {dest[0]}/.cache")
        machine.succeed(f"test \"$(stat -c '%U %a' {dest[0]})\" = 'alice 700'")
        machine.send_key("ret")
        wait_tty("1", "BACKUPS ON BACKUPSTICK")

    with subtest("a damaged copy is detected and refused before asking for the passphrase"):
        machine.succeed(f"printf 'XXXXXXXXXXXXXXXX' | dd of={backups}/{names[0]} bs=1 seek=4096 conv=notrunc status=none")
        machine.send_chars("c")
        wait_tty("1", "DAMAGED", timeout=60)
        machine.send_key("ret")
        wait_tty("1", "BACKUPS ON BACKUPSTICK")
        machine.send_key("ret")
        wait_tty("1", "RESTORE A BACKUP")
        machine.send_chars("y")
        wait_tty("1", "this copy is damaged", timeout=60)
        machine.succeed("test \"$(ls /home/alice/Restored | wc -l)\" = 1")
        machine.send_key("ret")
        wait_tty("1", "BACKUPS ON BACKUPSTICK")
  '';
}
