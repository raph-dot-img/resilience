# Cryptography (ADR 0015).
#
# The Security section of the interface runs the unprivileged
# `resilience-crypto`, which uses age, minisign, GnuPG, coreutils, xkcdpass
# and openssl by absolute store path (tools in /etc/resilience/system.toml,
# core.nix). Nothing runs with privileges and nothing runs in the background.
#
# This module only puts the same tools on the command line for accounts
# that have a shell (Stable and Advanced).
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.resilience.crypto;
in
{
  options.resilience.crypto.enable = lib.mkOption {
    type = lib.types.bool;
    default = true;
    description = "age and minisign on the command line (the interface's Security section works either way; gpg is always there).";
  };

  config = lib.mkMerge [
    {
      # GnuPG (OpenPGP interoperability) is used by the interface in every
      # mode: its agent asks passphrases with pinentry-curses on the
      # console, and forgets them right after use (no cache), like age and
      # minisign. No key server is configured; gpg is also always called
      # with --no-auto-key-retrieve.
      programs.gnupg.agent = {
        enable = true;
        pinentryPackage = pkgs.pinentry-curses;
        settings = {
          default-cache-ttl = 0;
          max-cache-ttl = 0;
        };
      };
    }
    (lib.mkIf cfg.enable {
      environment.systemPackages = [
        pkgs.age
        pkgs.minisign
      ];
    })
  ];
}
