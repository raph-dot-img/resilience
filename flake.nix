{
  description = "Resilience OS — an offline-first, terminal-only operating system based on NixOS";

  inputs = {
    # Production systems track the NixOS stable release branch only.
    # Bump deliberately with `nix flake update` and record it in CHANGELOG.md.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  };

  outputs = { self, nixpkgs }: { };
}
