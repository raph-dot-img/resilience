#!/usr/bin/env python3
"""Install Resilience from a live ISO onto an empty virtual disk, then boot it.

Headless, disposable QEMU, no network device, no root. The ISO is attached
as a USB stick (snapshot=on: the file is never written), as on a real
computer, so the installer must show it "in use". The target is a new
qcow2 disk in WORKDIR (20 GiB sparse; a few GiB are written, kept at the
end: put WORKDIR on a real disk, not a tmpfs, and delete it afterwards).

    python3 -I tests/iso/install.py ISO WORKDIR [--uefi OVMF.fd]
        [--encrypt] [--keymap us|fr] [--mem 2048] [--boot-only]

Keys are sent as physical (US) keys both while installing and when the
installed system starts, so passphrase and passwords are the same
characters in whichever layout the installed system loads.

The screen is not read: screenshots (PNG) are taken after each step into
WORKDIR and must be looked at. The copy is waited for by watching the
target disk's write counter (QEMU `info blockstats`): passwd waits for
the first password once writing stops.
"""
import argparse
import importlib.util
import os
import re
import subprocess
import sys
import tempfile
import time

_spec = importlib.util.spec_from_file_location(
    "smoke", os.path.join(os.path.dirname(os.path.abspath(__file__)), "smoke.py")
)
smoke = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(smoke)
monitor = smoke.monitor

PASSPHRASE = "quiz maze wave 42"
PW_USER = "amazing quokka"
PW_ADMIN = "zebra mqwa admin"


def keys(mon, text):
    smoke.keys(mon, text, "us")


def key(mon, name, n=1):
    for _ in range(n):
        monitor(mon, "sendkey " + name)
        time.sleep(0.15)


def written(mon):
    """Bytes written to the target disk so far, -1 if not found.

    QEMU 10 prints one block per device: "target: ..." then a table whose
    "Write:" row starts with the byte count (older ones: "wr_bytes=N").
    """
    out = monitor(mon, "info blockstats")
    m = re.search(r"^target:.*?wr_bytes=(\d+)", out, re.M)
    if m:
        return int(m.group(1))
    m = re.search(r"^target:.*?^Write:\s+(\d+)", out, re.M | re.S)
    return int(m.group(1)) if m else -1


class Vm:
    def __init__(self, a, name, drives):
        self.dir = tempfile.mkdtemp(prefix="resilience-install-")
        self.mon = os.path.join(self.dir, "monitor.sock")
        self.out = a.workdir
        self.t0 = time.monotonic()
        cmd = [
            "qemu-system-x86_64", "-m", a.mem, "-smp", "2", "-nic", "none",
            "-display", "none", "-vga", "std", "-device", "qemu-xhci,id=xhci",
            "-monitor", f"unix:{self.mon},server,nowait",
            "-serial", f"file:{os.path.join(a.workdir, name + '-serial.log')}",
        ] + drives
        if os.access("/dev/kvm", os.W_OK):
            cmd += ["-enable-kvm", "-cpu", "host"]
        if a.uefi:
            cmd += ["-bios", a.uefi]
        print("+", " ".join(cmd), flush=True)
        self.proc = subprocess.Popen(cmd)
        for _ in range(50):
            if os.path.exists(self.mon):
                break
            time.sleep(0.1)

    def shot(self, name):
        # The console blanks after 10 minutes without input; Shift wakes it
        # and types nothing.
        monitor(self.mon, "sendkey shift")
        time.sleep(0.5)
        path = os.path.join(self.out, name + ".png")
        monitor(self.mon, f"screendump {path} -f png")
        print(f"[{time.monotonic() - self.t0:6.1f}s] screenshot {path}", flush=True)

    def alive(self):
        if self.proc.poll() is not None:
            raise SystemExit(f"QEMU exited with {self.proc.returncode}")

    def quit(self):
        if self.proc.poll() is None:
            try:
                monitor(self.mon, "quit")
            except OSError:
                pass
            try:
                self.proc.wait(15)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def wait_writes_stop(vm, least, quiet=20, timeout=1800):
    """Wait until at least `least` bytes were written, then `quiet` s of none."""
    deadline = time.monotonic() + timeout
    last, since = -1, time.monotonic()
    while time.monotonic() < deadline:
        vm.alive()
        w = written(vm.mon)
        if w != last:
            last, since = w, time.monotonic()
        elif w >= least and time.monotonic() - since >= quiet:
            print(f"  {w / 2**20:.0f} MiB written, writing stopped", flush=True)
            return
        time.sleep(5)
    raise SystemExit("timed out waiting for the copy")


def install(a, disk):
    stick = [
        "-drive", f"file={a.iso},if=none,id=stick,format=raw,snapshot=on",
        "-device", "usb-storage,bus=xhci.0,drive=stick,bootindex=0",
        "-drive", f"file={disk},if=none,id=target,format=qcow2",
        "-device", "virtio-blk-pci,drive=target,bootindex=1",
    ]
    vm = Vm(a, "install", stick)
    try:
        time.sleep(a.boot_wait)
        vm.shot("01-home")
        keys(vm.mon, "/install res")
        time.sleep(1)
        key(vm.mon, "ret")
        time.sleep(2)
        vm.shot("02-install")
        key(vm.mon, "ret")
        time.sleep(3)
        vm.shot("03-disks")
        key(vm.mon, "ret")
        time.sleep(1)
        key(vm.mon, "down")
        if not a.encrypt:
            key(vm.mon, "ret")
        key(vm.mon, "down")
        key(vm.mon, "ret")
        time.sleep(1)
        vm.shot("04-layouts")
        if a.keymap == "fr":
            key(vm.mon, "down", 2)
        else:
            key(vm.mon, "home")
        key(vm.mon, "ret")
        time.sleep(1)
        vm.shot("05-form")
        key(vm.mon, "end")
        key(vm.mon, "ret")
        time.sleep(1)
        vm.shot("06-summary")
        keys(vm.mon, "vda")
        key(vm.mon, "ret")
        time.sleep(8)
        vm.shot("07-started")
        vm.alive()
        if a.encrypt:
            keys(vm.mon, PASSPHRASE)
            key(vm.mon, "ret")
            time.sleep(2)
            keys(vm.mon, PASSPHRASE)
            key(vm.mon, "ret")
            time.sleep(20)  # luksFormat: key derivation, a few seconds
            vm.shot("08-open")
            keys(vm.mon, PASSPHRASE)
            key(vm.mon, "ret")
            time.sleep(10)
        wait_writes_stop(vm, least=1 << 30)
        vm.shot("09-password-user")
        for pw in (PW_USER, PW_USER):
            keys(vm.mon, pw)
            key(vm.mon, "ret")
            time.sleep(3)
        time.sleep(5)
        vm.shot("10-password-admin")
        for pw in (PW_ADMIN, PW_ADMIN):
            keys(vm.mon, pw)
            key(vm.mon, "ret")
            time.sleep(3)
        wait_writes_stop(vm, least=0, quiet=15, timeout=600)
        vm.shot("11-installed")
    finally:
        vm.quit()


def boot(a, disk):
    vm = Vm(a, "boot", [
        "-drive", f"file={disk},if=none,id=target,format=qcow2",
        "-device", "virtio-blk-pci,drive=target,bootindex=0",
    ])
    try:
        if a.encrypt:
            time.sleep(40)
            vm.shot("12-passphrase")
            keys(vm.mon, PASSPHRASE)
            key(vm.mon, "ret")
        time.sleep(a.boot_wait)
        vm.shot("13-login")
        keys(vm.mon, "user")
        key(vm.mon, "ret")
        time.sleep(3)
        keys(vm.mon, PW_USER)
        key(vm.mon, "ret")
        time.sleep(20)
        vm.shot("14-session")
        smoke.keys(vm.mon, "/diagnostics", a.keymap)  # text for the TUI
        time.sleep(1)
        key(vm.mon, "ret")
        time.sleep(3)
        vm.shot("15-diagnostics")
        vm.alive()
    finally:
        vm.quit()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("iso")
    ap.add_argument("workdir")
    ap.add_argument("--uefi", help="path to OVMF.fd")
    ap.add_argument("--mem", default="2048")
    ap.add_argument("--boot-wait", type=int, default=75)
    ap.add_argument("--encrypt", action="store_true")
    ap.add_argument("--keymap", choices=["us", "fr"], default="us")
    ap.add_argument("--boot-only", action="store_true",
                    help="only start the disk installed by an earlier run")
    a = ap.parse_args()
    os.makedirs(a.workdir, exist_ok=True)
    disk = os.path.join(a.workdir, "target.qcow2")
    if not a.boot_only:
        if os.path.exists(disk):
            os.unlink(disk)
        subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", disk, "20G"], check=True)
        install(a, disk)
    boot(a, disk)
    print(f"Kept {disk} (a few GiB): delete it when done.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
