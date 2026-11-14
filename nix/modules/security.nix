# Baseline hardening. See docs/THREAT_MODEL.md for the reasoning and limits.
{ config, lib, ... }:

let
  # Rate-limit password guessing (THREAT_MODEL T6): after 5 failures within
  # 15 minutes the account is locked for 5 minutes. Short enough not to lock a
  # beginner out for long; tallies live in /run and reset at reboot.
  faillockSettings = {
    deny = 5;
    fail_interval = 900;
    unlock_time = 300;
  };
  faillockFor =
    service:
    let
      rules = config.security.pam.services.${service}.rules;
      faillock = "${config.security.pam.package}/lib/security/pam_faillock.so";
    in
    {
      auth.resilience-faillock-preauth = {
        order = rules.auth.unix.order - 10;
        control = "required";
        modulePath = faillock;
        settings = faillockSettings // {
          preauth = true;
        };
      };
      auth.resilience-faillock-authfail = {
        order = rules.auth.unix.order + 10;
        control = "[default=die]";
        modulePath = faillock;
        settings = faillockSettings // {
          authfail = true;
        };
      };
      account.resilience-faillock = {
        order = rules.account.unix.order - 10;
        control = "required";
        modulePath = faillock;
      };
    };
  faillockServices = [
    "login"
    "su"
    "sudo"
    "polkit-1"
  ];
in
{
  security.pam.services = lib.genAttrs faillockServices (s: {
    rules = faillockFor s;
  });
  # `faillock --user NAME [--reset]` lets an administrator inspect or lift a lock.
  environment.systemPackages = [ config.security.pam.package ];

  # Administration only through sudo, only for wheel, always with a password.
  security.sudo = {
    enable = true;
    wheelNeedsPassword = true;
    execWheelOnly = true;
  };
  # Root is locked: no direct root login on any console. NixOS would write
  # "!" for an unset password only when creating the account; setting it
  # explicitly re-applies it on every activation and makes it checkable
  # (tests/eval/security-invariants.nix).
  users.users.root.hashedPassword = lib.mkDefault "!";

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
