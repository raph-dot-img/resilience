# All Resilience OS NixOS modules. Import this (or `self.nixosModules.default`)
# into a NixOS configuration; then choose an edition profile from nix/profiles.
{
  imports = [
    ./core.nix
    ./admin.nix
    ./media.nix
    ./packs.nix
    ./users.nix
    ./interface.nix
    ./security.nix
    ./networking.nix
    ./boot.nix
    ./power.nix
    ./applications
  ];
}
