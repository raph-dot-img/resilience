# Security modes

Every account on Resilience OS has one of three modes. The mode is part of
the account itself (its system groups), not a setting in the interface, so it
cannot be changed by accident or by a program.

ULTRA-STABLE  (default for new accounts)
  For beginners, children, shared computers, and anyone who must not break
  the system. Guided operations only: no command shell, no administrative
  rights, no software installation. Some programs run in a restricted mode
  (for example the process monitor is read-only).

STABLE
  For people who know basic Linux. Adds a normal command-line shell and
  more tools, still without permission to change the operating system.

ADVANCED
  For administrators. Adds sudo (with your own password), which allows full
  control of the system. Mistakes in this mode can damage the installation;
  earlier system generations stay available in the boot menu.

WHAT MODES DO AND DO NOT PROTECT
  Modes protect the system from mistakes and from programs running under a
  less-privileged account. They do not protect against someone with
  physical access to the machine who can boot other media or remove the
  disk; full-disk encryption (planned for the installer) addresses that.

CHANGING A MODE
  SYSTEM MANAGEMENT > Users & security modes lists every account. Choose an
  account, press Enter, and choose the new mode. The interface explains
  what will change, then asks for confirmation. Cancel is preselected.

  Next, an ADMINISTRATOR (an Advanced account) must type their own
  password. Without it nothing changes, even if you are asking about your
  own account. Every change is written to the audit log and stays in
  effect after a restart.

  - More rights apply the next time that person logs in.
  - Fewer rights apply immediately: all of that account's sessions are
    closed, including yours if you lowered your own mode.
  - The account that logs in automatically always stays Ultra-Stable.
  - The last administrator cannot be lowered, so the computer always keeps
    someone who can repair it.

  Administrators: type your password only on a screen you trust.

TOO MANY WRONG PASSWORDS
  After 5 wrong passwords within 15 minutes, the account is locked for
  5 minutes. Wait, or ask an administrator to run
  `faillock --user NAME --reset`.
