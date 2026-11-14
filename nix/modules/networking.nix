# Offline-first networking: NetworkManager without connectivity checks, no
# time synchronisation by default, firewall on, no listening services.
{ lib, ... }:

{
  # Weaker than mkDefault so test frameworks and users override it freely.
  networking.hostName = lib.mkOverride 1400 "resilience";

  networking.networkmanager = {
    enable = lib.mkDefault true;
    # Never contact a connectivity-check server in the background.
    settings.connectivity.enabled = false;
  };
  networking.firewall = {
    enable = true;
    allowedTCPPorts = lib.mkDefault [ ];
    allowedUDPPorts = lib.mkDefault [ ];
  };

  # NTP would contact public servers periodically. Off by default; enable
  # explicitly when wanted (docs/OFFLINE_OPERATIONS.md).
  services.timesyncd.enable = lib.mkDefault false;
}
