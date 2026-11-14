# The offline library

Resource packs are large collections that work without any network:
Wikipedia, dictionaries, medical and practical guides, travel guides,
computer documentation. They are shared by every account on this computer.

READING
  Open KNOWLEDGE & DOCUMENTATION > Offline library. Choose a pack and
  press Enter. It opens in a text browser:
    Up / Down        move from link to link
    Enter or Right   follow the selected link
    Left             go back
    Space / b        next / previous page of text
    q, then y        quit and come back here
  In the library, s searches the whole text of the selected pack.
  The browser cannot open web sites or start other programs.

BUILT-IN DOCUMENTATION
  "System documentation (built in)" is always there, even without packs:
  this handbook, the NixOS manual and every NixOS setting. Press s on it
  and type a word (for example "wifi" or "firewall") to search all of it.

PACKS ON A USB STICK
  Packs on a stick can be read without installing them. In the Offline
  library press u, choose the stick and press Enter: its packs appear with
  the label "USB stick". Press c to check that a pack is exactly the
  expected file (this reads the whole file and takes a while for big
  packs). A file that is not in this system's list cannot be checked: open
  it only if you trust where it comes from.

CHOOSING WHAT TO GET
  Open KNOWLEDGE & DOCUMENTATION > Resource packs. Every pack is listed
  with its size:
    [*]  installed          [ ]  not installed
    [+]  will be added      [-]  will be removed
  Move to a pack and press Space to tick it. Enter shows its details. f
  shows only one language at a time. The top line shows how much will be
  added and how much space is free.
  Then press:
    d   to download the ticked packs (an Internet connection is needed)
    u   to copy them from a USB stick instead
  Not sure what to choose? Press s for suggested sets ("Essential",
  "Field guides", ...). Enter ticks the packs of a set; you can still
  untick some before pressing d or u.
  An administrator types their password once for the whole selection.

  Large packs take a long time: the complete English Wikipedia is about
  49 GiB. Start with a small one ("top articles"). You can stop a download
  with Ctrl-C; the next time, it continues where it stopped.

PREPARING A USB STICK ON ANOTHER COMPUTER
  Download the files from download.kiwix.org on any computer and copy them,
  with their names unchanged, to the top of a stick or into a folder named
  Resilience-packs. Press Enter on a pack to see the exact file name it
  needs. Every file is checked against its fingerprint (SHA-256) before it
  is installed: a damaged or altered file is refused, and nothing else is
  affected.

MEDICAL AND SAFETY CONTENT
  Health packs show a warning before they open. They do not replace a
  doctor, a pharmacist or emergency services, and may be out of date.

OLDER VERSIONS
  After a system update, newer versions of some packs may be offered. The
  older files stay readable until you remove them: they appear at the end
  of the Resource packs list as "old version". Tick them to free the space.
