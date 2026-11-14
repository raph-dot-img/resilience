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

A local network without Internet is still useful: computers on it can talk
to each other. Local messaging between Resilience computers is planned.
