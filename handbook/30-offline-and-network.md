# Working offline and the network

Resilience OS assumes there is no Internet. It does not check for updates,
synchronise the clock, or contact any server in the background.

THE NETWORK LINE ON THE MAIN SCREEN
  OFFLINE                no cable or wireless connection is active
  LOCAL LINK ONLY        connected to a local network with no way beyond it
  ROUTE UP (UNVERIFIED)  a route to other networks exists; the Internet may
                         or may not be reachable
  ONLINE (time)          the Internet was reachable at the time shown
  ROUTE UP, NO INTERNET  the last test failed

TESTING INTERNET ACCESS
  Open NETWORKING & MESH > Network status & diagnostics and press p.
  The test opens one connection to well-known public addresses and looks
  up one name, each limited to a few seconds. It runs only when you ask.

JOINING A WI-FI NETWORK
  Open NETWORKING & MESH > Wi-Fi networks. Nearby networks are listed with
  their signal strength; the list updates by itself, and r looks again.
  Choose a network and press Enter, type its password (it is shown as
  stars) and press Enter again. The password is usually on a label on the
  router. Connecting can take up to 45 seconds.

  - If the password is wrong, nothing is saved; try again.
  - Networks you join are saved for YOUR account only, with their password.
    Next time, Enter connects without asking.
  - d disconnects. f forgets a network and its password.
  - Open networks (no password) are not encrypted: people nearby can read
    what you send unless the program encrypts it itself. You are warned
    first.
  - Company networks (802.1X) and old WEP networks cannot be joined here;
    ask an administrator.
  - If Wi-Fi is switched off, press w to switch it on. If no adapter is
    found, use a network cable.

  In Ultra-Stable mode you can join and leave Wi-Fi networks, but not
  switch the whole network off, share a connection (hotspot) or change
  networks set up by an administrator.

THE CLOCK
  Without the Internet the clock is never corrected automatically. If the
  date or time on the main screen is wrong, open SYSTEM MANAGEMENT >
  Date & time. The current time and the time zone are shown; type the
  correct local date and time as YYYY-MM-DD HH:MM (for example
  2026-11-14 09:30) and press Enter. The interface shows what it will set,
  in your time zone and in UTC, and asks for confirmation. Cancel is
  preselected.

  An ADMINISTRATOR (an Advanced account) must then type their password.
  If there are several administrators, first type the number of the one
  who is typing the password. The change is written to the audit log, and
  the computer's hardware clock keeps it after a restart. Dates before 2025
  are refused.

  If the clock is wrong again after every restart, the small battery on the
  motherboard is probably empty.

A local network without Internet is still useful: computers on it can talk
to each other. Local messaging between Resilience computers is planned.
