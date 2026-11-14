# Baseline hardening. See docs/THREAT_MODEL.md for the reasoning and limits.
{ lib, ... }:

{
  # Administration only through sudo, only for wheel, always with a password.
  security.sudo = {
    enable = true;
    wheelNeedsPassword = true;
    execWheelOnly = true;
  };
  # Root keeps NixOS's default locked password ("!"): no direct root login.
  # Verified by tests/vm/modes.nix.

  services.openssh.enable = lib.mkDefault false;

  boot.kernel.sysctl = {
    "kernel.kptr_restrict" = 2;
    "kernel.dmesg_restrict" = 1;
    "kernel.unprivileged_bpf_disabled" = 1;
    "net.core.bpf_jit_harden" = 2;
    "kernel.yama.ptrace_scope" = 1;
    "kernel.kexec_load_disabled" = 1;
    "fs.protected_fifos" = 2;
    "fs.protected_regular" = 2;
    "fs.suid_dumpable" = 0;
  };

  # Nothing that phones home.
  programs.command-not-found.enable = false;
  nix.channel.enable = false;
  nix.settings = {
    experimental-features = [
      "nix-command"
      "flakes"
    ];
    # Fail fast when offline instead of hanging on substituters.
    connect-timeout = 5;
    fallback = true;
  };
}
