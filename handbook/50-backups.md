# Backups and USB sticks

WHY
  A computer can be lost, stolen, broken or wet. A backup is a copy of your
  files somewhere else. Keep TWO backups, on two sticks, in two places.

WHERE: SYSTEM MANAGEMENT > Backups & USB sticks
  Only USB sticks and memory cards are listed. The computer's own disks
  never are, and cannot be opened from here.
  Enter   open the selected stick
  e       eject it (wait for "SAFE TO UNPLUG" before pulling it out)
  r       look again after plugging a stick in

MAKE A BACKUP
  Open the stick and choose "+ Make a new backup of my files". Your whole
  home folder (notes, documents, settings) is saved into ONE file in the
  folder Resilience-backups on the stick. Your files are not changed.
  You will be asked for a PASSPHRASE, twice. The backup is encrypted with it:
    - without it, nobody can open the backup, not even you;
    - nobody can recover it for you;
    - leave it empty to get a strong generated passphrase shown on screen.
  Write the passphrase on paper and keep the paper apart from the stick.
  The folder .cache is not saved (it only holds temporary files).

CHECK A BACKUP
  Select a backup and press c. This reads the whole file back from the stick
  and compares it with its checksum, without the passphrase. "intact" means
  the copy is exactly what was written. "DAMAGED" means it is not: use
  another copy, and replace the stick.

RESTORE A BACKUP
  Select a backup and press Enter, then type its passphrase. The files are
  put into a NEW folder: Restored/backup-<date> in your home folder. Nothing
  that exists is overwritten. Move what you need from there, then delete the
  folder if you like. A damaged copy is refused. With a wrong passphrase,
  nothing is restored.
  Restoring files is separate from rolling the system back (see "When
  something goes wrong"): one never changes the other.

ON ANOTHER COMPUTER (Stable or Advanced account, or any Linux system with
age, zstd and tar)
  sha256sum -c home-DATE.tar.zst.age.sha256
  mkdir restored
  age -d home-DATE.tar.zst.age | zstd -d | tar -x -C restored

GOOD HABITS
  - Back up after important work, and at least every week.
  - Eject before unplugging, every time.
  - Check old backups now and then; cheap sticks wear out.
  - Sticks formatted as FAT32, exFAT, NTFS or ext4 work. Encrypted sticks
    are not supported yet.
