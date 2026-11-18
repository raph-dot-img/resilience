# Offline updates (ADR 0023), on a VM that boots through systemd-boot
# (UEFI) like an installed system. The VM runs the current version and trusts a
# test-only release key (update-snakeoil-key.nix). One USB stick holds:
#
#   resilience-update/            0.99.0, this edition and variant, signed:
#                                 the update that is installed
#   refusals/older/               0.0.1, signed (downgrade / replay)
#   refusals/other-edition/       the standard edition, signed
#   refusals/wrong-key/           the good update signed by another key
#   refusals/tampered/            the good manifest with one byte
#                                 changed after signing
#
# The good bundle holds the new system's whole closure, as a real one does
# (`nix copy --from` needs every reference in the source); the import copies
# only what is missing, and the new system is never in the VM's store
# before the update. The refusal bundles hold only their signed manifest:
# they are refused before any system is imported. Covered: the helper is root only and audited, polkit asks an
# administrator's password, `check-update` (as the user) reports integrity
# and authenticity apart, every refusal leaves the system as it was, the
# real TUI installs the good update with Carol's password, the new version
# runs after a restart with the previous one still in the boot menu, and
# applying the same bundle again is refused.
{ self }:
{ hostPkgs, ... }:
let
  pkgs = hostPkgs;
  key = import ./update-snakeoil-key.nix;
  updateManifest = pkgs.callPackage ../../nix/lib/update-manifest.nix { };
  variant = "vmtest";
  newVersion = "0.99.0";
  marker = "resilience-update-test-marker-NEWSYSTEM";

  # A second key, made when the test is built: never trusted by the VM.
  otherKey = pkgs.runCommand "update-test-other-key" { nativeBuildInputs = [ pkgs.nix ]; } ''
    export HOME=$TMPDIR
    nix --extra-experimental-features nix-command key generate-secret --key-name resilience-vmtest-other-1 > $out
  '';
  snakeoilKey = pkgs.writeText "update-test-snakeoil-key" key.secret;

  # A signed bundle folder at $out/resilience-update: `roots` and their
  # closure.
  mkBundle =
    {
      name,
      roots,
      edition,
      version,
      manifest,
      secretKey ? snakeoilKey,
      tamper ? false,
      compression ? "none",
    }:
    pkgs.runCommand "update-test-bundle-${name}"
      {
        __structuredAttrs = true;
        exportReferencesGraph.closure = roots;
        nativeBuildInputs = [
          pkgs.nix
          pkgs.python3
          pkgs.zstd
        ];
      }
      ''
        export HOME=$TMPDIR NIX_STATE_DIR=$TMPDIR/state NIX_CONF_DIR=$TMPDIR
        b=$out/resilience-update
        mkdir -p $b
        out=$b python3 ${pkgs.path}/pkgs/build-support/binary-cache/make-binary-cache.py \
          --compression ${compression}
        mkdir -p $b/build-trace-v2 $b/realisations $b/debuginfo $b/log
        nix --extra-experimental-features nix-command store sign \
          --store "file://$b" --key-file ${secretKey} --all
        ${pkgs.lib.optionalString tamper ''
          # One byte of the manifest, after signing: same size, wrong hash.
          f=$(grep -l 'format = 1' $b/nar/*.nar | head -n1)
          test -n "$f"
          sed -i 's/format = 1/format = 2/' "$f"
        ''}
        cat > $b/resilience-update.toml <<EOF
        format = 1
        edition = "${edition}"
        version = "${version}"
        manifest = "${manifest}"
        EOF
      '';

  # Systems named by manifests that are refused before any system is
  # imported: never built, never on the stick.
  fakeSystem = "/nix/store/00000000000000000000000000000000-nixos-system-never-imported";
in
{
  name = "resilience-update";

  nodes.machine =
    {
      config,
      lib,
      extendModules,
      ...
    }:
    let
      newer = extendModules {
        modules = [
          {
            resilience.version = lib.mkForce newVersion;
            environment.etc."resilience-test-marker".text = "${marker}\n";
          }
        ];
      };
      newSystem = newer.config.system.build.toplevel;
      edition = config.resilience.edition;
      manifestFor =
        ed: v: sys:
        updateManifest {
          edition = ed;
          version = v;
          systems.${variant} = sys;
        };
      goodManifest = manifestFor edition newVersion newSystem;
      good = {
        inherit edition;
        version = newVersion;
        manifest = goodManifest;
      };
      # The good manifest alone: refused before the system is imported.
      goodManifestOnly = good // {
        roots = [ goodManifest ];
      };
      manifestOnly = name: ed: v: rec {
        inherit name;
        edition = ed;
        version = v;
        manifest = manifestFor ed v fakeSystem;
        roots = [ manifest ];
      };
      bundles = {
        good = mkBundle (
          good
          // {
            name = "good";
            roots = [
              goodManifest
              newSystem
            ];
            compression = "zstd";
          }
        );
        older = mkBundle (manifestOnly "older" edition "0.0.1");
        other-edition = mkBundle (manifestOnly "other-edition" "standard" newVersion);
        wrong-key = mkBundle (
          goodManifestOnly
          // {
            name = "wrong-key";
            secretKey = otherKey;
          }
        );
        tampered = mkBundle (
          goodManifestOnly
          // {
            name = "tampered";
            tamper = true;
          }
        );
      };
      stick =
        pkgs.runCommand "update-stick.img"
          {
            nativeBuildInputs = [
              pkgs.dosfstools
              pkgs.mtools
            ];
          }
          ''
            size=$(( $(du -smc ${lib.concatStringsSep " " (lib.attrValues bundles)} | tail -n1 | cut -f1) * 2 + 32 ))
            truncate -s ''${size}M $out
            mkfs.vfat -n UPDATESTICK $out
            mcopy -s -i $out ${bundles.good}/resilience-update ::/
            mmd -i $out ::/refusals
            ${lib.concatMapStrings (n: ''
              mmd -i $out ::/refusals/${n}
              mcopy -s -i $out ${bundles.${n}}/resilience-update ::/refusals/${n}/
            '') (lib.attrNames (removeAttrs bundles [ "good" ]))}
          '';
    in
    {
      imports = [
        self.nixosModules.default
        ../../nix/systems/common.nix
        ../../nix/profiles/minimal.nix
      ];
      options.resilienceTest = lib.mkOption {
        type = lib.types.attrs;
        internal = true;
        visible = false;
        description = "Test only: what the test script needs.";
      };
      config = {
        resilienceTest = {
          inherit stick newSystem;
        };
        virtualisation.memorySize = 1536;
        virtualisation.cores = 2;
        virtualisation.diskSize = 4096;
        virtualisation.qemu.networkingOptions = lib.mkForce [ "-nic none" ];
        virtualisation.qemu.options = [ "-device qemu-xhci,id=xhci" ];
        virtualisation.useBootLoader = true;
        virtualisation.useEFIBoot = true;
        boot.loader.systemd-boot.enable = true;
        boot.loader.efi.canTouchEfiVariables = true;
        users.users.root.hashedPassword = lib.mkForce null;
        users.mutableUsers = true;
        resilience.localAccounts = true;
        resilience.updates = {
          inherit variant;
          trustedKeys = [ key.public ];
        };
        resilience.users = {
          alice.mode = "ultra-stable";
          carol.mode = "advanced";
        };
        resilience.interface.autologinUser = "alice";
        users.users.carol.initialPassword = "carol-pw";
      };
    };

  testScript =
    { nodes, ... }:
    ''
      import base64, os, re, shutil, time as _time

      NEW_SYSTEM = "${nodes.machine.resilienceTest.newSystem}"
      MARKER = "${marker}"
      RUNNING = "${nodes.machine.resilience.version}"

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

      def audit():
          return machine.succeed("cat /var/log/resilience/audit.log")

      def default_link():
          return machine.succeed("readlink /nix/var/nix/profiles/system").strip()

      def unchanged():
          assert default_link() == "system-1-link", default_link()
          machine.fail(f"nix-store --check-validity {NEW_SYSTEM} 2>/dev/null")

      def boot():
          machine.start()
          machine.wait_for_unit("multi-user.target")
          machine.succeed("dmesg -n 1")
          wait_tty("1", "MAIN MENU")

      plugs = 0
      def plug():
          global plugs
          plugs += 1
          # One copy for every plug-in (the image is large; the build
          # directory may be in memory). Mounted read-only each time.
          img = os.path.join(os.environ.get("TMPDIR", "/tmp"), "updatestick.img")
          if not os.path.exists(img):
              shutil.copy("${nodes.machine.resilienceTest.stick}", img)
              os.chmod(img, 0o644)
          machine.send_monitor_command(f"drive_add 0 if=none,id=stick{plugs},format=raw,file={img}")
          machine.send_monitor_command(f"device_add usb-storage,bus=xhci.0,drive=stick{plugs},id=usbstick{plugs},removable=on")
          machine.wait_until_succeeds("test -b /dev/sda")
          machine.succeed("udevadm settle")

      def unplug():
          machine.send_monitor_command(f"device_del usbstick{plugs}")
          machine.wait_until_fails("test -b /dev/sda")

      # Mounted by root for the command-line checks: the helper only needs
      # the folder to be under /run/media.
      MEDIA = "/run/media/root/UPDATESTICK"
      GOOD = f"{MEDIA}/resilience-update"
      def bundle(name):
          return f"{MEDIA}/refusals/{name}/resilience-update"

      boot()
      helper = machine.succeed("sed -n 's/^helper = \"\\(.*\\)\"$/\\1/p' /etc/resilience/admin.toml").strip()

      with subtest(f"the system runs {RUNNING}, trusts the test key, and has no new system yet"):
          print(machine.succeed("cat /etc/resilience/system.toml"))
          machine.succeed("grep -q '^\\[updates\\]' /etc/resilience/system.toml")
          machine.succeed("grep -q 'resilience-vmtest-snakeoil-1:' /etc/resilience/admin.toml")
          machine.fail("grep -q resilience-vmtest-other /etc/resilience/admin.toml")
          # Not in nix.conf: the key trusts nothing but updates.
          machine.fail("grep -q snakeoil /etc/nix/nix.conf")
          machine.fail("test -e /etc/resilience-test-marker")
          unchanged()

      with subtest("helper: root only; polkit asks an administrator's password"):
          rc, out = machine.execute(f"runuser -u alice -- {helper} apply-update /tmp 2>&1")
          assert rc == 77, (rc, out)
          rc, out = machine.execute(f"{helper} apply-update 2>&1")
          assert rc == 64, (rc, out)
          pid = machine.succeed("pgrep -u alice -x resilience").split()[0]
          rc, out = machine.execute(f"pkcheck --action-id org.resilience.admin.apply-update --process {pid} 2>&1")
          assert rc == 2, (rc, out)
          info = machine.succeed("pkaction --verbose --action-id org.resilience.admin.apply-update")
          for kind in ["any", "inactive", "active"]:
              assert re.search(rf"implicit {kind}:\s+auth_admin\n", info), info

      plug()
      machine.succeed(f"mkdir -p {MEDIA} && mount -o ro /dev/sda {MEDIA}")
      print(machine.succeed(f"find {MEDIA} -maxdepth 3 | sort"))

      with subtest("check-update, as the user: the good bundle is intact and authentic"):
          rc, out = machine.execute(f"runuser -u alice -- {helper} check-update {GOOD} 2>&1")
          print(out)
          assert rc == 0, (rc, out)
          assert re.search(r"Intact .*yes", out) and re.search(r"Authentic .*yes", out), out
          assert "can be installed" in out, out
          unchanged()

      with subtest("check-update tells a damaged update from an unsigned one"):
          rc, out = machine.execute(f"runuser -u alice -- {helper} check-update {bundle('tampered')} 2>&1")
          print(out)
          assert rc == 2 and re.search(r"Intact .*NO", out), (rc, out)
          rc, out = machine.execute(f"runuser -u alice -- {helper} check-update {bundle('wrong-key')} 2>&1")
          print(out)
          assert rc == 2 and re.search(r"Authentic .*NO", out) and re.search(r"Intact .*yes", out), (rc, out)
          unchanged()

      with subtest("refusals: nothing is installed, every attempt is audited"):
          cases = [
              ("older", 2, f"not newer than the running {RUNNING}"),
              ("other-edition", 2, "for the standard edition"),
              ("wrong-key", 2, "not signed by a Resilience release key"),
              ("tampered", 2, "or it is damaged"),
          ]
          for name, code, reason in cases:
              rc, out = machine.execute(f"{helper} apply-update {bundle(name)} 2>&1")
              print(name, rc, out)
              assert rc == code and reason in out, (name, rc, out)
              unchanged()
          # Only removable media: a copy elsewhere is refused before Nix runs.
          machine.succeed(f"mkdir /root/copy && cp {GOOD}/resilience-update.toml /root/copy/")
          rc, out = machine.execute(f"{helper} apply-update /root/copy 2>&1")
          assert rc == 2 and "/run/media" in out, (rc, out)
          rc, out = machine.execute(f"{helper} apply-update {MEDIA}/nothing-here 2>&1")
          assert rc == 2 and "cannot be found" in out, (rc, out)
          log = audit()
          print(log)
          assert len(re.findall(r"op=apply-update .*result=refused", log)) == 6, log
          assert not re.search(r"op=apply-update .*result=error", log), log
          assert not re.search(r"op=apply-update .*result=ok", log), log
          unchanged()

      machine.succeed(f"umount {MEDIA}")
      unplug()

      with subtest("TUI: Alice installs the update with Carol's password"):
          machine.send_chars("/rollback")
          wait_tty("1", "Recovery")
          machine.send_key("ret")
          wait_tty("1", "RECOVERY & DIAGNOSTICS")
          wait_tty("1", r"\[3\] Update from a USB stick")
          machine.send_chars("3")
          wait_tty("1", "UPDATE FROM A USB STICK")
          plug()
          machine.send_chars("r")
          wait_tty("1", r"UPDATESTICK .* not open")
          machine.send_key("ret")
          wait_tty("1", r"update 0\.99\.0")
          wait_tty("1", "it is not checked")
          machine.send_key("ret")
          wait_tty("1", "INSTALL AN UPDATE")
          machine.send_chars("y")
          wait_tty("1", "AUTHENTICATING FOR org.resilience.admin.apply-update")
          wait_tty("1", "Choose identity|Password:")
          screen = tty_text("1")
          if "Choose identity" in screen:
              m = re.search(r"(\d)\. .*carol", screen)
              assert m, screen
              machine.send_chars(m.group(1) + "\n")
              wait_tty("1", "Password:")
          machine.send_chars("carol-pw\n")
          wait_tty("1", r"Resilience 0\.99\.0 is installed", timeout=600)
          wait_tty("1", "Press Enter to return")
          assert default_link() == "system-2-link", default_link()
          machine.succeed(f"test \"$(readlink -f /nix/var/nix/profiles/system)\" = {NEW_SYSTEM}")
          # The running system is unchanged until the restart.
          machine.fail("test -e /etc/resilience-test-marker")
          log = audit()
          assert re.search(rf"op=apply-update caller=alice .*from={re.escape(RUNNING)} .*UPDATESTICK/resilience-update.* to=0\.99\.0 result=ok", log), log
          machine.succeed("journalctl -t resilience-admin | grep -q 'op=apply-update.*result=ok'")
          machine.send_key("ret")
          wait_tty("1", "UPDATE FROM A USB STICK")

      with subtest(f"after a restart 0.99.0 runs; {RUNNING} is still in the boot menu"):
          machine.shutdown()
          boot()
          machine.succeed(f"grep -qx {MARKER} /etc/resilience-test-marker")
          machine.succeed("grep -q '^version = \"0.99.0\"' /etc/resilience/system.toml")
          machine.succeed(f"test \"$(readlink -f /run/booted-system)\" = {NEW_SYSTEM}")
          machine.succeed("test -e /nix/var/nix/profiles/system-1-link")
          machine.succeed("test -e /boot/loader/entries/nixos-generation-1.conf")
          machine.succeed("systemctl is-system-running --wait | grep -qx running")

      with subtest("the same bundle again is refused (no replay)"):
          plug()
          machine.succeed(f"mkdir -p {MEDIA} && mount -o ro /dev/sda {MEDIA}")
          rc, out = machine.execute(f"{helper} apply-update {GOOD} 2>&1")
          print(out)
          assert rc == 2 and "not newer than the running 0.99.0" in out, (rc, out)
          assert default_link() == "system-2-link"
    '';
}
