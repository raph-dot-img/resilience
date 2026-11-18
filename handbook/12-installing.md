# Installing Resilience on a computer

The live USB stick keeps nothing: at power-off, files, messages, keys and
Wi-Fi networks are gone. Installing copies Resilience onto the computer's
disk so that everything is kept. You need the live stick and about 20
minutes. Installing works without any network.

THE WHOLE DISK IS ERASED
  Installing uses one whole disk and erases everything on it, including
  any other system (Windows, another Linux). Copy what you want to keep
  first (Backups & USB sticks). Installing next to another system is not
  offered.
  At least 17 GB is needed. The live stick itself is never offered.

STARTING
  From the live system: SYSTEM > Install Resilience on this computer.
  (It is offered only on the live system.)
  Up/Down choose a line, Enter changes it:
    Disk         the disk to install on; the list shows what each one
                 holds now. Disks in use cannot be chosen.
    Encryption   ON by default: a passphrase is asked every time the
                 computer starts, and anyone who takes the disk cannot
                 read it. If the passphrase is lost, NOTHING on the disk
                 can be recovered: write it down and keep it safe.
    Keyboard     the layout at login, in the interface, and when the
                 passphrase is asked at start.
    Account      the accounts to create (at most 8):
                   Enter  login name     f  full name
                   m      security mode  d  remove     a  add an account
  Two accounts are proposed: an everyday account (Stable) and an
  administrator (Advanced) that you use only to type its password when
  the system asks for an administrator. Without an Advanced account,
  nobody can change modes, install packs or update the computer later.

CONFIRMING
  Install... shows a summary: the exact disk, what is on it now, the
  choices and the accounts. Nothing has changed yet. To go on, type the
  disk's name (such as sda or nvme0n1) and press Enter; Esc goes back.

DURING THE INSTALLATION
  The steps are numbered. You type, yourself:
    - the encryption passphrase: twice to set it, once more to open the
      disk;
    - each account's password, twice.
  Nothing you type is shown. Copying the system takes several minutes.
  At the end: shut down, remove the stick, start the computer.
  If something fails, the installer says which step; the disk is then not
  usable: start the installation again.

AFTER THE FIRST START
  Type the passphrase (if encryption is on), then log in with an account
  and its password. There is no automatic login on an installed computer.
  The root account stays locked, as on every Resilience system.
