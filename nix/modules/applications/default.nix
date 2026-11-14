# Optional application groups. Each group can be toggled; the edition
# profiles (nix/profiles) choose the defaults.
{ config, lib, ... }:

let
  edition = config.resilience.edition;
  groupOption =
    name: default:
    lib.mkOption {
      type = lib.types.bool;
      inherit default;
      description = "Whether to install the ${name} application group.";
    };
in
{
  imports = [
    ./productivity.nix
    ./system-tools.nix
    ./networking.nix
    ./entertainment.nix
  ];

  options.resilience.applications = {
    productivity.enable = groupOption "productivity" true;
    systemTools.enable = groupOption "system tools" true;
    networking.enable = groupOption "networking" true;
    entertainment.enable = groupOption "entertainment" (edition != "minimal");
  };
}
