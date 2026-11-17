# First boot on real hardware: checklist

For trying Resilience OS on a real x86_64 PC (laptop or desktop).
Nothing here writes to the laptop's internal disk: the Resilience ISO is a
live system, and the hardware report runs from a live system too. Note
every answer; the results go to `docs/TESTING.md` (PASS / FAIL / NOT
TESTED, never guessed) and the hardware facts to a file under
`docs/hardware/`.

Bring: two USB sticks (≥ 2 GB each), the laptop's charger, a phone to
photograph screens and firmware menus, and a stopwatch.

## 0. Prepare the sticks (on the development machine)

1. Build the ISO (`nix build .#iso`) and write down its SHA-256
   (`sha256sum result/iso/*.iso`).
2. Stick A: the Resilience ISO. Stick B: the official NixOS 26.05 minimal
   ISO, to run the hardware report (the Resilience live ISO gives no
   shell, by design). Writing a stick needs root on your machine, so do it
   yourself, for example `sudo dd if=FILE.iso of=/dev/sdX bs=4M
   conv=fsync status=progress`. Check `/dev/sdX` with `lsblk` first: dd
   destroys whatever it is pointed at.
3. Copy `scripts/hw-report.sh` onto a third small stick, or onto a FAT
   partition of stick B if it has one. As a last resort, type it in.

## 1. Before booting: the laptop and its firmware

| # | Check | Result |
|---|---|---|
| 1.1 | Make and model (label underneath or on the case), year if known | |
| 1.2 | Keyboard layout printed on the keys (AZERTY FR? QWERTY US?) | |
| 1.3 | The battery holds a charge; the charger works | |
| 1.4 | Key to open the firmware setup and the boot menu (F2, F12, Esc, Del, Enter on ThinkPads…) | |
| 1.5 | Firmware type: UEFI only, legacy BIOS only, or UEFI with CSM/legacy | |
| 1.6 | Secure Boot: present? **Turn it off**: the ISO is not signed for Secure Boot | |
| 1.7 | Firmware password set by a previous owner? (it can block all of the above) | |
| 1.8 | Boot from USB allowed? | |
| 1.9 | Photograph the firmware's information page (CPU, RAM, disk, firmware version) | |

A 32-bit UEFI (some Atom-based laptops from around 2014) cannot boot the
ISO in UEFI mode: it carries only `BOOTX64.EFI`. Use legacy/CSM if the
firmware offers it.

## 2. Hardware report (stick B)

1. Boot stick B. At its shell: `sudo bash /path/to/hw-report.sh >
   hw-report.md`. As root the report adds memory slots and missing
   firmware messages; it is read-only either way.
2. If the minimal ISO has no Wi-Fi driver for this card, the report still
   says which card it is: that is the point.
3. Copy `hw-report.md` back to a stick (or photograph it), read it for
   anything private, and commit it as `docs/hardware/<model>.md`.

## 3. Boot Resilience OS (stick A)

Do this in **UEFI mode, then again in legacy mode** if the firmware has
both. Run the stopwatch from power-on.

| # | Check | UEFI | Legacy |
|---|---|---|---|
| 3.1 | The boot menu appears (GRUB in UEFI, syslinux in legacy) | | |
| 3.2 | The default entry boots without touching anything (after a few seconds; 10 s in legacy) | | |
| 3.3 | Time from power-on to the Resilience main menu | | |
| 3.4 | The screen stays readable during boot (no black screen, no garbage) | | |
| 3.5 | Main menu: STATUS shows SYSTEM [OK] OPERATIONAL | | |
| 3.6 | Text size: comfortable to read at the laptop's native resolution? (photo) | | |
| 3.7 | Colours: the eight colours render; in Settings, the Monochrome theme is readable | | |
| 3.8 | Box-drawing characters (frames, ▀▄ in the logo) drawn correctly, no `?` | | |

If the screen goes black or garbled after the boot menu: reboot, choose
**Options**, then **No modesetting** (`nomodeset`); both boot menus have
it. Note that it was needed: it means the
graphics driver needs attention in the system configuration.

## 4. Inside the interface

| # | Check | Result |
|---|---|---|
| 4.1 | Arrows, Enter, Esc, Tab, PgUp/PgDn, Home/End | |
| 4.2 | F1 (help) and F5 (status): do F-keys need Fn? Is there an Fn-lock? | |
| 4.3 | Layout: the console keymap is **US**. On an AZERTY keyboard, letters A/Q, Z/W and M are moved, and `/` (search) is elsewhere. Type `/` then `diagnostics` and note what appears | |
| 4.4 | F5 System status: battery detected, percentage plausible, charging/discharging changes when the charger is unplugged | |
| 4.5 | Network diagnostics: interfaces listed (wired, Wi-Fi). "No network hardware" here with a Wi-Fi card means a missing driver or firmware | |
| 4.6 | Wi-Fi: "Wi-Fi networks" (search `/wifi`) lists nearby networks. Joining one is optional | |
| 4.7 | Insert a USB stick: Backups & USB sticks lists it; eject works | |
| 4.8 | Lid close, then open: does the screen come back? | |
| 4.9 | Backlight: do the brightness keys work? | |
| 4.10 | Ctrl+Alt+F2: a login prompt appears on tty2; Alt+F1 returns to the interface | |
| 4.11 | "Power off" from the interface: the laptop switches off completely | |
| 4.12 | Leave it on the main menu, on battery, 30 minutes: battery percentage before and after | |
| 4.13 | Fan noise and heat after 10 minutes idle (note it) | |

## 5. Write it down

- `docs/hardware/<model>.md`: the report from step 2 plus the answers to
  1.x that the script cannot detect (layout, ports on the case, condition).
- `docs/TESTING.md`, "ISO boot": one row per firmware mode, PASS / FAIL,
  with the ISO's SHA-256 and the date.
- Anything that failed: as it happened, with the photo, before trying to
  fix it.
