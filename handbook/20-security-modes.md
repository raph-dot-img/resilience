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
  Only an administrator can change an account's mode, by changing the
  system configuration (resilience.users.<name>.mode) and rebuilding. A
  guided mode-change tool is planned; see ROADMAP.md.
