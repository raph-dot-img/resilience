#!/usr/bin/env python3
"""Boot-test a Resilience ISO in a disposable, headless QEMU VM.

No root needed, nothing on the host is modified, the VM has no network
device. Screenshots (PNG) of the VGA console are written to OUTDIR so a
human (or a reviewer) can check what was displayed.

    python3 -I tests/iso/smoke.py ISO OUTDIR [--uefi OVMF.fd] [--mem 2048]
                                  [--keymap fr|us]

--keymap is the console layout of the ISO (resilience.keyboard.layout,
"us" by default): QEMU sends physical keys, so the text typed has to be
translated to the keys that produce it in that layout.

Steps: boot (GRUB default entry), wait, screenshot of the licence notice,
accept it (Tab, Space, Enter); screenshot; press '/' and type
'diagnostics', Enter, screenshot; Esc Esc; F5 screenshot; Esc, '/messages'
screenshot, Enter, screenshot (Local messages on the standard edition).
"""
import argparse
import os
import socket
import subprocess
import sys
import tempfile
import time


def monitor(sock_path, cmd):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(10)
    s.connect(sock_path)
    s.recv(4096)  # banner
    s.sendall((cmd + "\n").encode())
    time.sleep(0.3)
    try:
        out = s.recv(65536).decode(errors="replace")
    except socket.timeout:
        out = ""
    s.close()
    return out


# QEMU key names (US physical positions) that type each character.
US_KEYS = {" ": "spc", "/": "slash", "-": "minus", ".": "dot"}
# French AZERTY: A/Q and Z/W swapped, M on the US ';' key, '/' is Shift+':'
# (':' on the US '.' key), '-' on the 6 key, '.' is Shift+';' (US ',').
FR_KEYS = {
    "a": "q", "q": "a", "z": "w", "w": "z", "m": "semicolon",
    " ": "spc", "/": "shift-dot", "-": "6", ".": "shift-comma",
}
KEYMAPS = {"us": US_KEYS, "fr": FR_KEYS}


def keys(sock_path, text, keymap="us"):
    names = KEYMAPS[keymap]
    for ch in text:
        monitor(sock_path, "sendkey " + names.get(ch, ch))
        time.sleep(0.05)


def accept_licence(mon):
    """The licence notice (ADR 0024): Tab opens the original text, Space
    ticks "I certify that I have read", Enter confirms."""
    for k in ("tab", "spc", "ret"):
        monitor(mon, "sendkey " + k)
        time.sleep(1)
    time.sleep(2)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("iso")
    ap.add_argument("outdir")
    ap.add_argument("--uefi", help="path to OVMF.fd")
    ap.add_argument("--mem", default="2048")
    ap.add_argument("--boot-wait", type=int, default=75)
    ap.add_argument("--keymap", choices=sorted(KEYMAPS), default="us")
    a = ap.parse_args()
    os.makedirs(a.outdir, exist_ok=True)
    tmp = tempfile.mkdtemp(prefix="resilience-iso-")
    mon = os.path.join(tmp, "monitor.sock")
    serial = os.path.join(a.outdir, "serial.log")
    cmd = [
        "qemu-system-x86_64", "-m", a.mem, "-smp", "2",
        "-cdrom", a.iso, "-boot", "d", "-nic", "none",
        "-display", "none", "-vga", "std",
        "-monitor", f"unix:{mon},server,nowait",
        "-serial", f"file:{serial}",
    ]
    if os.access("/dev/kvm", os.W_OK):
        cmd += ["-enable-kvm", "-cpu", "host"]
    if a.uefi:
        cmd += ["-bios", a.uefi]
    print("+", " ".join(cmd), flush=True)
    vm = subprocess.Popen(cmd)
    t0 = time.monotonic()
    try:
        for _ in range(50):
            if os.path.exists(mon):
                break
            time.sleep(0.1)
        shots = []

        def shot(name):
            path = os.path.join(a.outdir, name + ".png")
            monitor(mon, f"screendump {path} -f png")
            shots.append(path)
            print(f"[{time.monotonic() - t0:5.1f}s] screenshot {path}", flush=True)

        time.sleep(8)
        shot("01-bootloader")
        time.sleep(a.boot_wait)
        shot("02-licence")
        accept_licence(mon)
        shot("02-home")
        keys(mon, "/diagnostics", a.keymap)
        time.sleep(1)
        monitor(mon, "sendkey ret")
        time.sleep(2)
        shot("03-network")
        monitor(mon, "sendkey esc")
        monitor(mon, "sendkey esc")
        time.sleep(1)
        monitor(mon, "sendkey f5")
        time.sleep(3)
        shot("04-status")
        monitor(mon, "sendkey esc")
        time.sleep(1)
        keys(mon, "/messages", a.keymap)
        time.sleep(1)
        shot("05-search-messages")
        monitor(mon, "sendkey ret")
        time.sleep(3)
        shot("06-messages")
        if vm.poll() is not None:
            print("QEMU exited early with", vm.returncode)
            return 1
        return 0
    finally:
        if vm.poll() is None:
            try:
                monitor(mon, "quit")
            except OSError:
                pass
            try:
                vm.wait(10)
            except subprocess.TimeoutExpired:
                vm.kill()


if __name__ == "__main__":
    sys.exit(main())
