# Community network

Computers nearby can share what they have: their library (encyclopedias,
guides), their resource packs and maps, a folder of files and a few web
pages. A school or a shelter with one well-stocked computer can then
serve a whole street, without the Internet.
Open COMMUNICATIONS > Community network.

FINDING COMPUTERS THAT SHARE
  Computers that share appear in the list a little after you join the
  same network (the same Wi-Fi, a cable, the mesh or a hotspot), if they
  have local messaging. Each line shows its name, what it shares and its
  address. "not reachable from here" means it was heard but is on
  another network.
    Enter        open the selected computer
    a            type the address of a computer (for example
                 192.168.1.20, or fe80::1%bat0 on the mesh)
    r            refresh the list
  A name in quotes with a question mark, like "School library"?, is only
  what that computer says about itself. Anyone can announce any name.

WHAT YOU CAN DO WITH A COMPUTER
  Its front page    what it shares, in the text browser
  Its library       read its encyclopedias and guides; q comes back here
  Its pages         notices, local news, guides it publishes
  Its shared folder copy one file into your Received folder
  Get packs from it tick packs with Space, then Enter. An administrator's
                    password is needed, as for any pack.
  Packs you get from another computer are checked against this system's
  catalog before they are installed: a pack that does not match exactly
  is refused and deleted. Pages and files are NOT checked: they are as
  trustworthy as whoever runs that computer. A copied file never opens
  or runs by itself; read it with Find & view files. Nothing in your
  Received folder is ever replaced ("name (2)" instead).
  A big pack over Wi-Fi can take hours (the mesh is slower still): a USB
  stick is often faster.

SHARING THIS COMPUTER (administrators)
  Choose "Share this computer's resources":
    Network      the network to share on (Enter: the next one). One at a
                 time; never all of them.
    Name         the name others see: a place, not a person
    Packs        others can download your packs (they check them)
    Library      others read your packs in their browser
    Shared folder  /var/lib/resilience/shared/files
    Pages          /var/lib/resilience/shared/pages (index.html first)
  Then "Share now..." and an administrator's password.
  EVERYTHING YOU SHARE CAN BE READ BY ANYONE ON THAT NETWORK. Share only
  what you would put on a public notice board. Only files that everyone
  on this computer may read are served: a private file (readable by its
  owner only) in the shared folders is not. The form counts both.
  Administrators (Advanced accounts) put files in the shared folders
  themselves, for example from the Terminal shell.
  Nobody can send anything to this computer: it only serves.
  While it shares, a line at the top of every screen says so. It stops
  when you stop it (Enter on the first line of Community network, and the
  password again), when that network changes or goes, and when the
  computer restarts.

CREATING A NETWORK
  When there is no Wi-Fi router:
    Hotspot      this computer becomes a Wi-Fi network that phones and
                 other computers can join. A new password is shown when
                 it starts: give it to the people who should join. It
                 never gives anyone access to the Internet through this
                 computer. Phones on it can open
                 http://resilience.home.arpa/ to read what this computer
                 shares. Needs an administrator's password.
    Mesh         computers relay for each other (see Local messages).
  The Wi-Fi card does one of these at a time, and cannot join another
  Wi-Fi network meanwhile; a cable still works. Both stop at restart.

FOR EXPERIENCED USERS
  The hotspot's DNS answers names in home.arpa only (nothing is
  forwarded). More names can be declared in the configuration:
    resilience.share.hotspot.dnsRecords = { printer = "10.42.0.20"; };
  A computer left on in a shelter can share at every start:
    resilience.share.autoStart = {
      interface = "wlan0"; services = [ "library" "packs" ];
      name = "Shelter library"; };
  The services listen on TCP port 80 of the chosen network only.
