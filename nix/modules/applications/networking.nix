{
  config,
  lib,
  pkgs,
  ...
}:

lib.mkIf
  (config.resilience.applications.networking.enable && config.networking.networkmanager.enable)
  {
    resilience.apps.nmtui = {
      name = "Network connections";
      category = "networking";
      description = "Connect to Wi-Fi or configure wired networks (NetworkManager). Esc goes back.";
      command = [ "${pkgs.networkmanager}/bin/nmtui" ];
      minMode = "stable";
      keywords = [
        "wifi"
        "wireless"
        "ethernet"
        "connect"
      ];
    };
  }
