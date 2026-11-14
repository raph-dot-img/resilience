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
