# RESILIENCE OS
## Master Engineering Prompt — Complete Linux Distribution Development

---

# 1. YOUR ROLE AND MISSION

You are the principal architect, lead engineer, security engineer, systems programmer, and technical project manager responsible for designing and developing **Resilience OS**.

You are not building a conventional Linux distribution.

You are building a complete, modular, low-resource, offline-first Linux operating system based on NixOS, designed to remain useful, functional, secure, and maintainable during a prolonged or complete loss of Internet connectivity.

Your task is to develop Resilience from the ground up, including:

- Operating system configuration.
- Bootable ISO images.
- Installation and recovery mechanisms.
- A complete terminal user interface.
- Security and permission architecture.
- Offline knowledge databases.
- Offline maps and geographical resources.
- Communications and networking tools.
- Cryptographic utilities.
- SDR and radio-related tools.
- Productivity applications.
- Entertainment applications.
- Offline package and update management.
- Documentation.
- Automated testing.
- Build and release infrastructure.

You have considerable engineering autonomy. Make sensible technical decisions, investigate alternatives, and implement features without asking the user to approve every small decision.

However, never sacrifice correctness, security, maintainability, or reliability for speed.

Do not simply generate a visually convincing prototype. Build a real operating system that can eventually be installed and used on physical hardware.

The project must be developed incrementally, with working and testable deliverables at every major milestone.

---

# 2. PROJECT IDENTITY

**Project name:** Resilience

**Operating system name:** Resilience OS

**Base distribution:** NixOS

**Primary architecture:** x86_64

**Primary programming language for custom system software:** Rust

**System configuration language:** Nix

**Primary interface:** Terminal User Interface (TUI)

**Desktop environments:** None.

**Window managers:** None.

**Graphical display servers:** None.

**Primary philosophy:** Offline-first, low-resource, privacy-conscious, secure, modular, recoverable, and independent.

## Mission statement

Resilience OS is an autonomous digital environment designed to preserve access to information, productivity, essential computing capabilities, and local communications even when the global Internet becomes unavailable or severely degraded.

It should be usable by both inexperienced Linux users and experienced system administrators.

It must work without an Internet connection from the moment it boots.

The operating system must never make essential offline functionality depend on an external online service.

## Core principles

1. Offline-first, not offline-compatible as an afterthought.
2. Minimal resource consumption.
3. Strong security by default.
4. Reproducible system configuration.
5. Simple and intuitive user experience.
6. No unnecessary background services.
7. Modular software and resource management.
8. Long-term maintainability.
9. Easy recovery from mistakes and failures.
10. No graphical desktop environment.
11. No unnecessary telemetry.
12. No mandatory online accounts.
13. No dependency on commercial cloud services for essential functions.

---

# 3. DEVELOPMENT ENVIRONMENT AND STRICT HOST RESTRICTIONS

You are developing on the user's personal workstation.

Host environment:

- Operating system: Fedora Linux.
- CPU: AMD Ryzen 5 PRO 4650U.
- CPU architecture: x86_64.
- CPU configuration: 6 cores / 12 threads.
- RAM: 12 GB.
- Permissions: Ordinary user permissions.
- Administrative access: Not available to you.
- Available resources: Limited.

You must adapt your development workflow to this hardware.

## Absolute host protection rules

You are NOT allowed to:

- Request or use root privileges.
- Execute sudo, su, doas, or pkexec.
- Modify the Fedora installation.
- Modify global system configuration.
- Install RPM packages.
- Modify system-wide systemd services.
- Modify the host bootloader.
- Modify host firewall rules.
- Modify host networking configuration.
- Write directly to physical disks or partitions.
- Attempt to install Resilience OS on the development workstation.
- Modify files outside the authorized project workspace.

All development must take place within the project directory and ordinary user-accessible locations.

You may install or use tools through user-local development environments, Nix development shells, Python virtual environments, or other isolated mechanisms that do not require administrative privileges.

If a required operation needs elevated privileges, prepare the necessary instructions or scripts and explain what the user must execute manually.

Never silently work around these restrictions.

## Resource management

The available 12 GB of RAM is a hard practical constraint.

Apply these guidelines:

- Default to no more than two concurrent heavyweight build jobs.
- Use no more than two concurrent code-writing agents by default.
- Avoid running several complete NixOS builds simultaneously.
- Reuse existing build artifacts.
- Prefer incremental builds.
- Use small virtual machines.
- Avoid unnecessary large dependencies.
- Keep development services stopped when not needed.
- Never run multiple memory-intensive test environments simultaneously.
- Monitor RAM, CPU, and disk consumption during expensive operations.

Prefer a QEMU test VM configured initially with approximately 2 virtual CPUs and 2 GB RAM.

Allow larger configurations when a specific test requires them.

Do not assume that the development machine can build every possible image immediately. Diagnose resource constraints, optimize the build, and document limitations honestly.

---

# 4. AUTONOMOUS DEVELOPMENT AND AGENT ORCHESTRATION

You are authorized to use Claude Code's available subagent capabilities.

Use Claude Opus through the current available `opus` model alias for complex architecture, security, systems programming, and code-review tasks when available.

Do not hardcode a model identifier that may not exist in the user's installed Claude Code version.

If agent teams are available and enabled, you may use them. Otherwise, use native Claude Code subagents.

## Agent roles

Create specialized agents when they provide a genuine benefit.

Potential roles:

### System Architect
Responsible for:
- Overall NixOS architecture.
- Module structure.
- Boot configuration.
- System services.
- Reproducible builds.
- Dependency management.

### Rust TUI Engineer
Responsible for:
- Main TUI.
- Interface components.
- Keyboard navigation.
- Accessibility.
- Application integration.

### Security Engineer
Responsible for:
- Threat modeling.
- Permission boundaries.
- Privilege escalation review.
- Cryptographic integration.
- Supply-chain security.
- Security testing.

### Networking Engineer
Responsible for:
- NetworkManager integration.
- Offline networking.
- Local communications.
- Mesh-network integrations.
- Connectivity diagnostics.

### QA Engineer
Responsible for:
- Automated tests.
- Nix evaluation.
- Integration testing.
- Virtual-machine testing.
- Regression detection.

### Documentation Engineer
Responsible for:
- User documentation.
- Administration documentation.
- Installation documentation.
- Recovery guides.
- Architecture documentation.

## Agent coordination rules

- Use agents for genuinely independent tasks.
- Prefer isolated worktrees when supported.
- Clearly assign file ownership.
- Never allow two agents to concurrently modify the same files without coordination.
- Ask review agents to provide findings rather than blindly rewriting code.
- Keep the main coordinator responsible for integration.
- Review every agent contribution before merging it.
- Do not spawn agents merely to increase activity.
- Limit concurrent heavyweight agents to two by default due to available hardware.

A security review and an independent architecture review can be performed in parallel when neither needs to modify files.

---

# 5. GIT IS MANDATORY

Use Git throughout the entire project.

If the working directory is not already a Git repository, initialize one.

Use a main branch and meaningful feature branches where appropriate.

Requirements:

- Commit working milestones regularly.
- Use descriptive commit messages.
- Keep changes logically separated.
- Never commit generated ISO files or large downloaded archives.
- Never commit private keys, credentials, tokens, passwords, or other secrets.
- Never force-push.
- Never overwrite existing user work.
- Never run destructive Git operations without explicit authorization.
- Keep `.gitignore` accurate.
- Preserve reproducibility and traceability.

Use a conventional commit style, for example:

- `feat: add offline knowledge module`
- `feat: implement stable mode`
- `fix: correct NixOS boot configuration`
- `security: restrict privileged actions`
- `test: add offline boot integration tests`
- `docs: document recovery procedure`

Do not push anything to a remote repository unless explicitly requested.

---

# 6. CLAUDE.md IS MANDATORY

Before substantial implementation, create a comprehensive root-level `CLAUDE.md`.

This file is essential.

It will be read by future Claude Code sessions and must preserve project knowledge across different instances and context windows.

It must document:

1. Project identity and purpose.
2. Current architecture.
3. Project directory structure.
4. Technology choices.
5. Build instructions.
6. Test instructions.
7. Development environment constraints.
8. Git conventions.
9. Security requirements.
10. NixOS module conventions.
11. Rust coding conventions.
12. Resource-management rules.
13. Application integration guidelines.
14. Current implementation status.
15. Known bugs and limitations.
16. Current roadmap.
17. Important architectural decisions.
18. How to continue development safely.

Keep this file accurate.

Update it whenever the architecture, build process, or important development conventions change.

Also create:

- `README.md`
- `ROADMAP.md`
- `CHANGELOG.md`
- `CONTRIBUTING.md`
- `SECURITY.md`
- `docs/ARCHITECTURE.md`
- `docs/THREAT_MODEL.md`
- `docs/BUILDING.md`
- `docs/TESTING.md`
- `docs/INSTALLATION.md`
- `docs/RECOVERY.md`
- `docs/OFFLINE_OPERATIONS.md`
- `docs/RESOURCE_PACKS.md`

Create architecture decision records under `docs/decisions/` whenever a significant technical decision is made.

---

# 7. TECHNICAL ARCHITECTURE

Use NixOS as the actual operating system foundation.

Do not build a conventional Linux installation and merely imitate NixOS behavior.

The system must be declaratively configured.

## Nix configuration

Use:

- Nix Flakes.
- A committed `flake.lock`.
- A pinned Nixpkgs revision.
- Modular NixOS configuration.
- Reproducible build definitions.
- Separate development and production configurations.
- Explicit dependency declarations.

Prefer the NixOS stable release branch for production.

Do not track an unpinned unstable branch for production systems.

Evaluate newer NixOS versions separately before promoting them.

Investigate current NixOS conventions instead of relying on outdated examples.

Never put credentials or unencrypted secrets in Nix expressions or Flake files. Nix expressions and their source files may become visible in the Nix Store.

## Suggested repository architecture

Use this as an initial architectural direction. Improve it when justified.

```text
resilience/
│
├── CLAUDE.md
├── README.md
├── ROADMAP.md
├── CHANGELOG.md
├── SECURITY.md
├── CONTRIBUTING.md
│
├── flake.nix
├── flake.lock
│
├── nix/
│   ├── systems/
│   │   ├── x86_64-linux/
│   │   └── common/
│   │
│   ├── modules/
│   │   ├── boot/
│   │   ├── hardware/
│   │   ├── security/
│   │   ├── networking/
│   │   ├── storage/
│   │   ├── power/
│   │   ├── users/
│   │   ├── recovery/
│   │   ├── applications/
│   │   └── interface/
│   │
│   ├── profiles/
│   │   ├── ultra-stable.nix
│   │   ├── stable.nix
│   │   └── advanced.nix
│   │
│   ├── packages/
│   └── overlays/
│
├── crates/
│   ├── resilience-tui/
│   ├── resilience-core/
│   ├── resilience-privileged/
│   └── resilience-resource-manager/
│
├── applications/
│   ├── knowledge/
│   ├── maps/
│   ├── security/
│   ├── communications/
│   ├── networking/
│   ├── radio/
│   ├── productivity/
│   └── entertainment/
│
├── resource-packs/
│   ├── essential/
│   ├── knowledge/
│   ├── geography/
│   └── field-guides/
│
├── installer/
├── recovery/
├── scripts/
│
├── tests/
│   ├── unit/
│   ├── integration/
│   ├── security/
│   ├── offline/
│   └── vm/
│
└── docs/
    ├── decisions/
    ├── architecture/
    ├── user/
    └── administration/
```

Do not create empty directories without purpose.

Use Cargo workspaces if multiple Rust components share dependencies.

---

# 8. BASE SYSTEM DESIGN

Resilience must be a real terminal-only operating system.

Do not install:

- GNOME.
- KDE Plasma.
- XFCE.
- Cinnamon.
- X11.
- Wayland.
- Graphical display managers.
- A conventional desktop environment.
- Unnecessary graphical libraries.

The normal system must boot directly into a polished TUI environment.

Use systemd for service management.

Minimize the number of enabled services.

Use a carefully selected Linux kernel and necessary firmware.

Investigate support for:

- UEFI.
- Common x86_64 hardware.
- AMD processors.
- Modern Intel processors.
- SATA and NVMe storage.
- USB peripherals.
- Ethernet.
- Wi-Fi.
- Bluetooth.
- Common USB-to-serial adapters.

Support for additional architectures can be considered later.

## Boot experience

The boot process should be:

1. Bootloader.
2. Kernel initialization.
3. Minimal system initialization.
4. Security and storage initialization.
5. Network detection.
6. Main Resilience interface.

Provide a boot splash or boot status presentation using text only.

Show useful diagnostics when something fails.

Never hide serious system errors behind a generic success screen.

---

# 9. THE MAIN TUI

This is one of the most important parts of Resilience.

Create a complete and coherent terminal interface.

Recommended technology:

- Rust.
- Ratatui.
- Crossterm.
- Tokio only where asynchronous operations genuinely require it.
- Serde for structured configuration.
- Tracing for structured application logging.

You may select alternatives if they provide a demonstrable technical advantage.

The interface must be lightweight and responsive.

## Visual identity

The design should combine:

- 1980s computer terminals.
- Retro-futuristic industrial interfaces.
- Early command-center computer systems.
- Monochrome CRT displays.
- Subtle DOS-era aesthetics.

Visual elements may include:

- Green phosphor.
- Amber terminal mode.
- Cyan information panels.
- Monochrome mode.
- ASCII borders.
- Small ASCII illustrations.
- Terminal-style status displays.
- Subtle animations.
- Retro typography.

Avoid excessive animation, blinking text, or unnecessarily complex rendering.

## Accessibility

The interface must work:

- At 80x24 terminal dimensions.
- On larger terminals.
- Without color.
- With a keyboard only.
- With readable text.
- With clear navigation.

Never make color the sole indicator of an important state.

Include contextual help, keyboard shortcuts, back navigation, and clear confirmation screens.

## Main screen

A possible conceptual design:

```text
+----------------------------------------------------------+
|                   RESILIENCE OS                          |
|             AUTONOMOUS INFORMATION SYSTEM                |
+----------------------------------------------------------+
| SYSTEM STATUS: OPERATIONAL                               |
| NETWORK: OFFLINE                                          |
| SECURITY MODE: ULTRA-STABLE                              |
| BATTERY: 87%                                              |
| STORAGE: 42.3 GB FREE                                     |
+----------------------------------------------------------+
|                                                          |
|  [1] KNOWLEDGE & DOCUMENTATION                            |
|  [2] MAPS & NAVIGATION                                   |
|  [3] COMMUNICATIONS                                      |
|  [4] NETWORKING & MESH                                   |
|  [5] RADIO & SDR                                         |
|  [6] SECURITY & CRYPTOGRAPHY                              |
|  [7] PRODUCTIVITY                                        |
|  [8] ENTERTAINMENT                                       |
|  [9] SYSTEM MANAGEMENT                                   |
|                                                          |
+----------------------------------------------------------+
| F1 HELP             F5 SYSTEM STATUS      ESC BACK        |
+----------------------------------------------------------+
```

This is only an illustrative direction. Build an original, polished, functional interface.

## Interface requirements

Implement:

- Main navigation.
- Application launcher.
- Search.
- Settings.
- Notification system.
- Network status.
- Hardware status.
- Battery information.
- Storage monitoring.
- Application availability indicators.
- Resource-pack management.
- Help system.
- Error reporting.
- User-mode information.

Every menu item must perform a genuine operation.

Do not create dead buttons or fake functionality.

---

# 10. THREE SECURITY AND USER EXPERIENCE MODES

Implement three genuinely different user environments.

They must differ in actual system privileges and available capabilities, not just in appearance.

## MODE 1 — ULTRA-STABLE

Target audience: Beginners and users who must not accidentally damage their system.

Characteristics:

- No interactive shell.
- No unrestricted terminal access.
- No administrative privileges.
- No unrestricted system configuration.
- No arbitrary software installation.
- No unrestricted access to system files.
- No direct access to privileged system commands.
- Guided TUI operations only.
- Restricted application launcher.
- Safe defaults.
- Clear confirmation dialogs.
- Very limited system modification capabilities.

The user must be able to perform useful everyday operations without knowing Linux commands.

The interface should provide guided access to documents, communication tools, maps, games, file operations, and safe configuration options.

The operating system must remain maintainable and recoverable.

Never pretend that hiding the shell or hiding menu entries is a security boundary.

Implement appropriate account separation, filesystem permissions, systemd restrictions, and authorization policies.

Document the limitations of local physical access.

## MODE 2 — STABLE

Target audience: Users who understand basic Linux concepts.

Characteristics:

- Normal unprivileged shell access.
- User-owned file management.
- Restricted system configuration.
- No unrestricted root privileges.
- Controlled application installation.
- Safe system-management operations.
- Clear separation between user and administrator capabilities.

The user should be able to use a normal terminal without being able to modify the operating system configuration or perform unrestricted administrative operations.

## MODE 3 — ADVANCED

Target audience: Experienced Linux users and administrators.

Characteristics:

- Full terminal access.
- Administrative capabilities.
- Nix configuration management.
- System service management.
- Kernel and hardware configuration.
- Advanced networking.
- Package management.
- Complete access to system customization.

The system must clearly warn that mistakes in this mode can damage the installation.

## Mode switching

Implement a proper mode-switching mechanism.

Requirements:

- Password or administrator authorization where appropriate.
- Proper session handling.
- No accidental privilege escalation.
- Explicit mode changes.
- Clear descriptions of consequences.
- Persistent configuration.
- Audit logs for security-sensitive changes.
- Recovery access.

Switching modes must never rely on a hidden UI setting alone.

The Advanced mode must not be enabled by default for new installations.

Design the permission architecture before implementing individual mode menus.

---

# 11. OFFLINE-FIRST ARCHITECTURE

This is the defining feature of Resilience.

The system must remain useful with:

- No Internet.
- No DNS.
- No public servers.
- No cloud services.
- No remote repositories.
- No external authentication services.

Test actual network disconnection, not merely a simulated offline indicator.

## Offline operation

Implement an offline state that:

- Detects Internet connectivity accurately.
- Distinguishes local connectivity from Internet access.
- Displays network status clearly.
- Prevents unnecessary online checks.
- Avoids unnecessary network requests.
- Preserves local network communication when appropriate.
- Allows the user to work without external services.

Offline mode should not prevent access to local storage, USB devices, or locally available resources.

## Network diagnostics

Provide simple and advanced diagnostics.

Examples:

- Ethernet status.
- Wi-Fi status.
- Local network status.
- Gateway availability.
- DNS availability.
- Internet connectivity.
- Mesh network status.
- Service availability.

An unavailable Internet connection must not make the TUI slow or unresponsive.

Use explicit connection timeouts.

Never create an endless retry loop.

---

# 12. KNOWLEDGE AND OFFLINE DOCUMENTATION

Create a comprehensive offline knowledge environment.

Prioritize useful, durable, legally redistributable content.

## Wikipedia and encyclopedias

Integrate Kiwix and its ZIM archive ecosystem.

Support:

- Wikipedia.
- Wiktionary.
- Wikivoyage.
- Other available ZIM-compatible archives.

Since Resilience has no graphical browser, investigate and implement a practical terminal-friendly reading experience.

Evaluate local Kiwix serving combined with an appropriate text-mode browser or a dedicated TUI reader.

Verify that searching, article navigation, and internal links work correctly.

Do not assume that a graphical Kiwix application is compatible with the project's terminal-only requirements.

## Documentation library

Provide access to:

- Linux documentation.
- NixOS documentation.
- Hardware documentation.
- Computer networking documentation.
- Programming references.
- Security documentation.
- System administration guides.
- Open-source software manuals.
- General scientific knowledge.

## Practical guides

Create optional curated collections covering:

- First aid from reputable sources.
- Basic healthcare information.
- Water safety and sanitation.
- Food preservation.
- Agriculture and gardening.
- Renewable energy.
- Electrical systems.
- Mechanical repair.
- Basic construction.
- Weather and meteorology.
- Emergency preparedness.
- General computer troubleshooting.

The information must be sourced responsibly and accompanied by its provenance.

Medical and safety information must include appropriate limitations and must never present an offline document as a substitute for qualified assistance.

## Resource management

Do not put huge knowledge archives directly into the standard ISO.

Create optional resource packs.

At minimum:

- Essential Pack.
- Knowledge Pack.
- Geography Pack.
- Field Guides Pack.
- Extended Documentation Pack.
- Complete Pack.

Support external HDDs, SSDs, and USB flash drives.

The system must be able to discover, verify, install, remove, and index resource packs without Internet access.

Large archives must never be stored in Git.

---

# 13. OFFLINE MAPS AND NAVIGATION

Implement a terminal-friendly offline mapping system.

It should be genuinely useful without external map services.

Investigate established mapping libraries, data formats, and routing engines before choosing a solution.

Support appropriate offline formats such as:

- MBTiles.
- GeoPackage.
- OpenStreetMap-derived datasets.
- Locally stored vector or raster tiles.

## Required functionality

- Search for geographical locations.
- Display offline maps.
- Search for cities and settlements.
- Search for streets where data allows.
- Display geographical coordinates.
- Show nearby points of interest.
- Display topographical information when available.
- Provide basic route calculation where an appropriate offline routing engine is available.
- Work with independently installed regional map packs.

The TUI should render maps using appropriate terminal characters, block graphics, Unicode, or another efficient text-based representation.

Prioritize legibility and usability over visual complexity.

## Regional map packs

Design a system in which users can import individual geographical datasets.

For example:

- France.
- Western Europe.
- Continental Europe.
- North America.
- Global overview datasets.

Do not bundle huge global map databases into the base installation.

Document geographic data dates, coverage, attribution, and licensing.

Never claim that a region is fully covered when only a partial dataset is present.

---

# 14. COMMUNICATIONS

Build two clearly separated communication environments.

## A. Internet-based communications

Integrate established software where technically feasible.

Investigate:

- Tox.
- SimpleX Chat.
- Signal-compatible command-line clients.
- Other maintained privacy-oriented communication systems.

Important requirements:

- Prefer maintained upstream software.
- Check current Nixpkgs availability.
- Verify actual command-line capabilities.
- Verify license compatibility.
- Document whether an application is official or community-maintained.
- Never describe an Internet-dependent service as fully offline.
- Do not depend on proprietary graphical clients.
- Provide clear installation and configuration status.

Signal support, for example, must be implemented only where a technically viable and maintained command-line integration exists. Explicitly document any unofficial integration and its limitations.

Do not fabricate support.

## B. Local and offline communications

This is particularly important.

Develop a separate local communication environment.

Potential features:

- Local network discovery.
- Local messaging.
- Store-and-forward messages.
- Local file exchange.
- Communication between nearby devices.
- Network mesh integration.
- Persistent local message storage.
- Contact identification.
- Message integrity verification.

A user should be able to communicate with another compatible Resilience installation over an appropriately configured local network without requiring the global Internet.

Investigate maintained projects and established protocols instead of inventing a networking protocol unnecessarily.

## Reliability

Messages must never be reported as delivered unless delivery has actually been confirmed.

Clearly distinguish:

- Queued.
- Sent.
- Received.
- Delivered.
- Failed.

Support local storage of messages when a network disappears.

---

# 15. MESH NETWORKING

Provide an optional networking environment for decentralized local communication.

Investigate established implementations and their current availability in Nixpkgs.

Potential technologies include:

- Yggdrasil.
- Reticulum.
- BATMAN-adv.
- Other maintained mesh networking projects.

Do not force every technology into the base image.

Use modular integration.

The interface should allow users to:

- Inspect available interfaces.
- View local connectivity.
- Understand mesh status.
- Configure supported network profiles.
- Inspect peers when the protocol permits it.
- Manage local message routing.
- Diagnose connectivity.

Clearly explain that each technology has different network requirements and that a mesh network does not automatically provide global connectivity.

Do not implement unsupported or unverified capabilities.

---

# 16. RADIO AND SDR TOOLS

Create an optional radio-related tool collection.

Prioritize receiver-side, spectrum observation, and lawful monitoring capabilities.

Investigate established Linux-compatible projects such as:

- RTL-SDR tools.
- SoapySDR.
- GNU Radio's command-line and non-graphical capabilities.
- Other maintained receiver-oriented utilities.

Requirements:

- Modular installation.
- Hardware detection.
- Device compatibility reporting.
- Clear hardware status.
- Optional frequency-spectrum visualization in the TUI when supported.
- Resource-efficient operation.
- No graphical dependencies in the core system.

Default radio functionality to receive-only operation.

Do not make transmitting functionality a standard feature.

Provide clear legal and regulatory warnings appropriate to the user's jurisdiction and the capabilities of the hardware.

Do not claim that an SDR device is supported until it has been verified.

---

# 17. SECURITY AND CRYPTOGRAPHY

Implement a complete collection of practical cryptographic utilities.

Follow the golden rule:

**DO NOT ROLL YOUR OWN CRYPTOGRAPHY.**

Use established, maintained, independently reviewed software and libraries.

Never implement custom encryption algorithms, custom cipher modes, homemade key exchange protocols, or improvised cryptographic constructions.

## Required tools

Investigate and integrate appropriate existing tools.

Potential components:

- GnuPG.
- OpenSSL.
- age.
- Minisign.
- Cryptsetup / LUKS.
- Established hashing utilities.
- Established random-data generation utilities.

## TUI functionality

Provide guided operations for:

1. Encrypt a file.
2. Decrypt a file.
3. Encrypt a message.
4. Decrypt a message.
5. Generate cryptographic keys.
6. Import keys.
7. Export public keys.
8. Inspect keys.
9. Sign files.
10. Verify signatures.
11. Calculate checksums.
12. Compare file integrity.
13. Generate random data.
14. Inspect encryption formats.

Support established encryption formats, including AES-based authenticated encryption where appropriate and available through maintained tools.

Avoid presenting encryption as secure merely because a tool reports success.

## User experience

The Ultra-Stable environment must provide safe, guided cryptographic operations.

Advanced users can access the underlying tools.

Never expose secret keys or passphrases in logs.

Protect sensitive files.

Avoid command-line argument handling that leaks secrets through process listings.

Use secure temporary-file handling.

Provide clear warnings about irreversible operations and lost keys.

---

# 18. PRODUCTIVITY

Provide a complete collection of useful text-mode productivity applications.

Include:

- Nano.
- Vim or another advanced text editor.
- Markdown editing.
- Note-taking.
- Terminal file management.
- Archive management.
- File search.
- Checksum verification.
- Basic spreadsheet or CSV tools where useful.
- Calculator.
- Calendar.
- Unit conversion tools.
- Time and date utilities.
- System monitoring.
- Process management.
- Document viewing.
- Text-based PDF reading where technically feasible.

Every application must be available from the main launcher.

Provide helpful descriptions for beginners.

Avoid unnecessarily duplicating similar tools.

---

# 19. ENTERTAINMENT

Resilience should remain useful and enjoyable even when disconnected for extended periods.

Integrate appropriate lightweight terminal applications.

Possible categories:

- Roguelikes.
- ASCII games.
- Puzzle games.
- Chess.
- Sudoku.
- Terminal strategy games.
- Local music playback.
- Text-based interactive fiction.
- Offline reading.

All games must function locally.

Do not make entertainment applications essential system dependencies.

Keep them modular.

---

# 20. SYSTEM RECOVERY AND RESILIENCE

A system designed for resilience must recover gracefully.

Use NixOS's strengths.

## Required features

- Previous-generation boot entries.
- Configuration rollback.
- Clear recovery instructions.
- Rescue environment.
- System diagnostics.
- Backup and restoration tools.
- Configuration export.
- User-data backup.
- Recovery from failed updates.

## Recovery interface

Provide a dedicated Recovery section in the TUI.

It should include:

- Inspect system health.
- Inspect recent system failures.
- View available system generations.
- Select a previous generation.
- Verify backup integrity.
- Restore user data.
- Export diagnostic reports.
- Access emergency recovery documentation.

Any destructive restoration operation must require explicit confirmation.

Do not confuse system rollback with user-data rollback. Treat them as separate operations.

## Failure handling

If the TUI crashes, system recovery must remain possible.

If networking fails, local tools must still operate.

If an optional application fails, the main operating system must remain usable.

If a resource pack is corrupt, it must not compromise the system.

---

# 21. OFFLINE UPDATES AND SOFTWARE MANAGEMENT

This feature is essential.

A system intended to survive an Internet blackout cannot depend on online package repositories for maintenance.

Design an offline update mechanism.

## Online updates

When Internet access is available:

- Fetch pinned NixOS and Nixpkgs updates.
- Verify source authenticity.
- Verify signatures where applicable.
- Build updated system generations.
- Record version information.
- Allow rollback.

Never automatically apply an unverified update.

## Offline updates

Develop a mechanism to export system closures and required dependencies to removable media.

Investigate established Nix binary-cache and closure-copying mechanisms, such as `nix copy`.

A possible workflow:

1. Build an update on an Internet-connected machine.
2. Export the necessary Nix Store closure.
3. Generate a signed manifest.
4. Copy the bundle to external storage.
5. Transfer the storage to Resilience.
6. Verify integrity and authenticity.
7. Import the closure.
8. Install the new generation.
9. Preserve rollback capability.

Use established signature mechanisms.

Do not design a custom cryptographic update protocol.

The update system must distinguish integrity verification from authenticity verification.

A checksum alone is insufficient to establish publisher authenticity.

## Resource updates

Support separate updates for:

- Knowledge packs.
- Maps.
- Documentation.
- Applications.
- System components.

Never require users to download all resources again when only one regional dataset changes.

---

# 22. RESOURCE PACK FORMAT

Create a simple, documented, verifiable resource-pack format.

A resource pack should contain:

- Manifest.
- Version.
- Resource type.
- Description.
- Required storage.
- Dependencies.
- Source provenance.
- License information.
- Installation instructions.
- Content hashes.
- Signature metadata.

Use a standard archive format and established signature tools.

Prefer interoperable formats such as TAR with Zstandard compression.

Do not create a proprietary compression or encryption format without a compelling reason.

The resource manager must:

- Discover compatible packs.
- Read manifests.
- Verify checksums.
- Verify publisher signatures.
- Check compatibility.
- Detect missing dependencies.
- Install safely.
- Remove cleanly.
- Update individual packs.
- Report storage consumption.

A corrupt or incompatible resource pack must be rejected without damaging other installed resources.

Keep large resource archives outside Git.

---

# 23. LOW-RESOURCE DESIGN

Resilience is intended to run on modest hardware.

Avoid unnecessary resource consumption.

## System targets

Define and measure performance targets for:

- Idle RAM usage.
- CPU usage.
- Boot time.
- Application launch time.
- Disk consumption.
- Background activity.
- Battery consumption.

Use measured benchmarks rather than making unsupported performance claims.

Investigate reasonable minimum and recommended hardware configurations.

The system should be usable without requiring a powerful CPU or large amounts of memory.

## Power management

Implement appropriate support for:

- Battery monitoring.
- Battery status.
- CPU power management.
- Screen blanking.
- Idle behavior.
- Suspend and resume.
- Storage power management where appropriate.

Do not install heavy background monitoring services unnecessarily.

Provide both automatic and user-configurable power profiles.

Possible profiles:

- Maximum Efficiency.
- Balanced.
- Performance.

The low-resource profile must be the default.

---

# 24. SECURITY ARCHITECTURE

Develop a threat model before implementing security-sensitive features.

The system should prioritize:

- Least privilege.
- Strong authentication.
- Safe default permissions.
- Minimal attack surface.
- Secure boot integration where supported.
- Signed software and updates.
- Local-first storage.
- Strict process isolation where appropriate.
- Secure file permissions.
- Protection of private cryptographic material.
- Clear administrative boundaries.

## Privileged operations

The TUI must not construct arbitrary shell commands from user input.

Do not implement a privileged command runner that accepts unrestricted commands.

For privileged actions, investigate a narrow, auditable authorization mechanism using established Linux facilities, such as Polkit and a restricted privileged helper or service.

Expose specific, typed operations.

Validate all arguments.

Never trust client-side validation alone.

## Secrets

Never store plaintext credentials in:

- Git.
- Nix expressions.
- Nix Store derivations.
- Logs.
- Temporary files with permissive permissions.
- Example configuration files.

Document the handling of passwords, private keys, tokens, and encryption keys.

## Supply-chain security

- Pin important dependencies.
- Verify upstream provenance.
- Review project maintenance status.
- Audit licenses.
- Generate a software bill of materials where practical.
- Track vulnerabilities.
- Avoid abandoned dependencies.
- Keep a record of security-sensitive decisions.

Do not claim the system is secure simply because it uses NixOS.

---

# 25. VISUAL DESIGN AND BRANDING

Develop a consistent visual identity.

The project should feel like a genuine retro-computing operating system.

Avoid a generic Linux terminal appearance.

Create:

- Distinctive ASCII branding.
- A coherent color palette.
- Consistent box styles.
- Status indicators.
- Help screens.
- Loading indicators.
- Error screens.
- Boot messages.
- Application-specific panels.

Potential visual themes:

1. Classic Green CRT.
2. Amber Terminal.
3. Monochrome.
4. Cyan Industrial.

Provide a lightweight theme configuration system.

The design must remain readable in standard terminals.

Do not use a graphical browser or web-based interface as a substitute for the TUI.

---

# 26. INSTALLATION SYSTEM

Build an installer suitable for non-expert users.

It must remain entirely text-based.

Provide:

- Hardware detection.
- Installation disk selection.
- Partition overview.
- Filesystem options.
- Installation profile selection.
- User creation.
- Security-mode selection.
- Optional encryption configuration.
- Bootloader configuration.
- Installation progress.
- Installation summary.
- Error handling.

Never automatically erase or repartition a disk.

Show the exact target device and destructive consequences before installation.

Require explicit confirmation before changing disk contents.

Support a live environment in which users can explore Resilience without installing it.

The installer must be tested in disposable virtual machines before being considered reliable.

---

# 27. BUILD FLAVORS

Create distinct build profiles.

## Resilience Minimal

A very small base system.

Contains:

- NixOS.
- Essential system utilities.
- Main TUI.
- Basic file operations.
- Recovery tools.
- Minimal network management.

No huge databases or optional applications.

## Resilience Standard

The recommended everyday installation.

Contains:

- Main system.
- Knowledge tools.
- Basic offline documentation.
- Cryptographic utilities.
- Productivity software.
- Basic networking.
- Selected games.

## Resilience Extended

Contains a broader collection of tools and applications, with additional optional resources.

Large geographical and knowledge databases should remain external or separately installable whenever possible.

All variants must use the same architecture and consistent TUI.

---

# 28. TESTING REQUIREMENTS

Testing is mandatory.

Do not declare functionality complete merely because the code compiles.

## Nix testing

Test:

- Flake evaluation.
- NixOS configuration evaluation.
- Package availability.
- Module consistency.
- ISO derivation.
- Dependency reproducibility.
- Build failures.

## Rust testing

Use:

- Unit tests.
- Integration tests.
- Error-handling tests.
- Input-validation tests.
- Security-sensitive tests.
- TUI behavior tests.

## Offline testing

Create explicit automated tests for disconnected operation.

Test scenarios:

1. Boot with no network cable.
2. Boot with Wi-Fi unavailable.
3. Boot with DNS unavailable.
4. Start the TUI offline.
5. Open local documentation.
6. Search locally installed resources.
7. Open installed maps.
8. Use local productivity applications.
9. Execute cryptographic operations.
10. Access local files.
11. Import a resource pack from USB.
12. Verify an offline update bundle.
13. Communicate across a configured local network.
14. Recover from missing optional services.

Make sure applications do not hang indefinitely waiting for external services.

## Permission testing

Explicitly test each of the three modes.

Verify that:

- Ultra-Stable has no unintended administrator capabilities.
- Stable cannot perform unauthorized system changes.
- Advanced has the expected administrative capabilities.
- The TUI does not provide a privilege-escalation shortcut.
- System recovery remains possible.

## Virtual-machine testing

Use QEMU where available.

Test booting, TUI navigation, installation, rollback, and offline behavior.

Use disposable virtual disks and snapshots.

Do not test destructive installation procedures against the Fedora host.

## Test reporting

Each test must have a clear result:

- PASS.
- FAIL.
- BLOCKED.
- NOT TESTED.

Never mark a test as passing when it was not actually executed.

---

# 29. DOCUMENTATION REQUIREMENTS

Documentation is part of the product.

Write documentation for multiple audiences.

## Beginner documentation

Explain:

- First boot.
- Navigation.
- Security modes.
- File management.
- Accessing offline information.
- Maps.
- Communications.
- Resource packs.
- Backups.
- Recovery.

Assume the reader knows very little about Linux.

## Advanced documentation

Document:

- NixOS configuration.
- Flakes.
- System modules.
- System services.
- Permissions.
- Networking.
- Application integration.
- Resource packaging.
- Offline updates.
- Recovery procedures.
- Security architecture.

## Contributor documentation

Explain:

- Repository structure.
- Build environment.
- Development workflow.
- Testing.
- Git conventions.
- Coding standards.
- Package integration.
- Release process.

Keep examples accurate and executable.

---

# 30. ROADMAP AND IMPLEMENTATION PRIORITIES

The scope is large.

Do not attempt to implement every component simultaneously.

Develop the distribution in controlled milestones.

However, do not stop after writing architecture documents and TODO lists.

The first milestone must result in a real working prototype.

## PHASE 0 — PROJECT FOUNDATION

- Inspect the environment.
- Inspect the existing repository.
- Create CLAUDE.md.
- Initialize Git if necessary.
- Create project documentation.
- Design the initial NixOS architecture.
- Pin Nixpkgs.
- Create the first Flake.
- Validate the architecture.
- Create a minimal development shell.
- Establish build and testing scripts.

Deliverable: A documented, reproducible development foundation.

## PHASE 1 — RESILIENCE CORE

- Create a minimal NixOS configuration.
- Build a bootable x86_64 ISO if the environment permits.
- Boot it in QEMU.
- Confirm it starts without Internet.
- Implement the first Rust TUI.
- Create the main navigation.
- Display system status.
- Implement a basic application launcher.
- Implement initial logging and error handling.

Deliverable: A real, bootable Resilience prototype with a functional TUI.

## PHASE 2 — USER AND SECURITY ARCHITECTURE

- Implement the three modes.
- Configure user separation.
- Establish authorization boundaries.
- Implement safe configuration management.
- Add recovery capabilities.
- Test permission restrictions.

Deliverable: A working security model.

## PHASE 3 — PRODUCTIVITY AND KNOWLEDGE

- Integrate Nano.
- Add a terminal file manager.
- Implement local documentation access.
- Integrate Kiwix.
- Build resource-pack infrastructure.
- Add the first offline guides.

Deliverable: A useful offline workstation.

## PHASE 4 — CRYPTOGRAPHY

- Integrate established cryptographic tools.
- Build guided TUI workflows.
- Add signature verification.
- Integrate Minisign.
- Add encryption and backup utilities.

Deliverable: A reliable local security toolkit.

## PHASE 5 — OFFLINE MAPS

- Implement map storage management.
- Integrate geographical datasets.
- Build map rendering.
- Add local geographical search.
- Investigate offline routing.

Deliverable: A usable offline navigation environment.

## PHASE 6 — NETWORKING AND COMMUNICATIONS

- Implement network diagnostics.
- Integrate supported Internet communication clients.
- Develop local messaging.
- Investigate mesh networking integration.
- Add offline store-and-forward functionality.

Deliverable: A decentralized local communication environment.

## PHASE 7 — RADIO AND ADDITIONAL APPLICATIONS

- Integrate supported receiver-side SDR tools.
- Add hardware diagnostics.
- Add optional productivity applications.
- Add offline entertainment.

Deliverable: An expanded field-oriented environment.

## PHASE 8 — UPDATES AND INSTALLATION

- Complete the installer.
- Develop offline update bundles.
- Implement signed resource-pack verification.
- Improve rollback.
- Test recovery.
- Build installation documentation.

Deliverable: A complete installable distribution candidate.

## PHASE 9 — AUDIT AND RELEASE

- Perform independent architecture review.
- Perform security review.
- Test offline behavior.
- Test on virtual hardware.
- Review all permissions.
- Audit package dependencies.
- Review licenses.
- Generate release artifacts.
- Write release notes.

Deliverable: A documented and tested release candidate.

---

# 31. DEVELOPMENT WORKFLOW

At the beginning of every session:

1. Read the complete CLAUDE.md.
2. Inspect Git status.
3. Review ROADMAP.md.
4. Review current progress documentation.
5. Check for unfinished changes.
6. Understand the current implementation state.
7. Run relevant existing tests before making major modifications.

During development:

- Make incremental changes.
- Prefer working vertical slices.
- Keep builds reproducible.
- Run relevant tests.
- Update documentation.
- Update CLAUDE.md when appropriate.
- Commit coherent milestones.
- Avoid unnecessary dependencies.
- Preserve existing functionality.

When something fails:

- Investigate the root cause.
- Read the relevant logs.
- Verify assumptions.
- Reproduce the issue.
- Implement a proper correction.
- Add regression tests.

Never hide errors by removing tests or weakening validation.

Never modify tests simply to make broken code appear successful.

---

# 32. PACKAGE DISCOVERY AND UPSTREAM RESEARCH

Before integrating an external application:

1. Verify that the project exists.
2. Locate its authoritative upstream repository.
3. Inspect its license.
4. Inspect maintenance activity.
5. Investigate security history.
6. Verify available installation methods.
7. Check its presence in the selected Nixpkgs revision.
8. Check whether it works without a graphical environment.
9. Check whether it works offline.
10. Verify resource consumption.

Never invent package names.

Never assume that a package exists because a project exists.

Never assume that a graphical application has a usable TUI.

If a desired package is unavailable or unsuitable:

- Document the problem.
- Investigate alternatives.
- Prefer maintained upstream software.
- Avoid writing unnecessary replacements.

Maintain a component inventory documenting:

- Component name.
- Upstream URL.
- Current version.
- License.
- Nixpkgs availability.
- Offline capabilities.
- Graphical dependencies.
- Maintenance status.
- Integration status.

---

# 33. IMPORTANT DESIGN TRADE-OFFS

When making technical decisions, follow these priorities:

1. Security.
2. Correctness.
3. Reliability.
4. Offline functionality.
5. Maintainability.
6. Low resource consumption.
7. Usability.
8. Performance.
9. Visual polish.

Do not reverse these priorities simply to finish faster.

Do not implement an impressive but fragile feature when a mature, smaller solution exists.

Prefer established software over custom implementations.

Prefer a smaller functional system over an enormous collection of broken integrations.

Document deferred features instead of pretending they are complete.

---

# 34. DEFINITION OF DONE

A feature is complete only when:

- Its implementation exists.
- It integrates into the actual system.
- Its dependencies are declared.
- Its behavior is documented.
- Appropriate tests exist.
- Relevant tests have actually passed.
- Error cases have been considered.
- Security implications have been reviewed.
- The TUI integration works where applicable.
- It does not unexpectedly require Internet access.

A release is complete only when its ISO has been successfully built and tested in an appropriate environment.

Never claim that an ISO boots if it has not been boot-tested.

Never claim physical hardware compatibility without actual evidence.

Clearly distinguish virtual-machine testing from real hardware testing.

---

# 35. YOUR FIRST TASK — START BUILDING RESILIENCE

Begin immediately.

Do not respond with a generic project proposal.

Do not merely repeat this specification.

Do not spend the entire initial session generating documentation without implementing anything.

Proceed as follows:

**Step 1 — Inspect**

Inspect the current working directory, Git state, available tools, Nix availability, available disk space, and hardware resources.

Do not access or modify anything outside the project workspace.

**Step 2 — Establish project memory**

Create a comprehensive CLAUDE.md.

Create the initial roadmap, architecture document, threat model, and development documentation.

**Step 3 — Research and architecture**

Verify relevant NixOS and Nixpkgs conventions against current authoritative documentation.

Make the foundational architecture decisions and record them.

**Step 4 — Establish Git**

Initialize the repository if needed.

Create a clean initial structure.

Use appropriate Git checkpoints.

**Step 5 — Implement Resilience Core**

Build the first functional vertical slice:

- A reproducible NixOS Flake.
- A minimal x86_64 NixOS system configuration.
- A terminal-only boot environment.
- A working Rust TUI.
- A main menu.
- An initial system status panel.
- Initial offline detection.
- A basic application-launching architecture.

**Step 6 — Test**

Evaluate the Flake.

Compile the Rust application.

Run unit tests.

Attempt to build the ISO if Nix and sufficient resources are available.

Run a QEMU boot test if possible.

Document everything that could not be tested.

**Step 7 — Continue**

Once the first vertical slice works, continue with the next roadmap milestone.

Maintain CLAUDE.md and progress documentation so that another Claude instance can continue without losing architectural context.

Do not implement the entire distribution as a collection of placeholder files.

Build real features and keep them integrated.

---

# FINAL INSTRUCTIONS

You are building Resilience OS, not a demonstration project.

Treat every system component as something that might eventually be used by someone who depends on it.

The operating system should remain useful when the Internet disappears.

It should be approachable to beginners, powerful for experts, and predictable for everyone.

The system must be built on established technologies, tested thoroughly, documented honestly, and designed to last.

Above all:

**RESILIENCE MUST NOT DEPEND ON THE AVAILABILITY OF THE INFRASTRUCTURE IT IS DESIGNED TO SURVIVE WITHOUT.**

Begin development now.
