# Offline-first networking: NetworkManager without connectivity checks, no
# time synchronisation by default, firewall on, no listening services.
#
# Wi-Fi for accounts outside the networkmanager group (Ultra-Stable,
# ADR 0010): NetworkManager's upstream polkit defaults let any active
# console session switch networking off, share a hotspot or enable
# connectivity checks without a password. The rule below keeps only what
# joining a Wi-Fi network needs and denies every other NetworkManager action
# (including ones added in future versions). It only ever removes rights.
{ config, lib, ... }:

let
  nmAllowedWithoutGroup = [
    "org.freedesktop.NetworkManager.wifi.scan"
    # Activate and deactivate connections.
    "org.freedesktop.NetworkManager.network-control"
    # Profiles restricted to this account (connection.permissions user:NAME).
    "org.freedesktop.NetworkManager.settings.modify.own"
    # The Wi-Fi radio switch, to turn Wi-Fi back on.
    "org.freedesktop.NetworkManager.enable-disable-wifi"
  ];
in

{
  # Weaker than mkDefault so test frameworks and users override it freely.
  networking.hostName = lib.mkOverride 1400 "resilience";

  networking.networkmanager = {
    enable = lib.mkDefault true;
    # Never contact a connectivity-check server in the background.
    settings.connectivity.enabled = false;
    # Wired networks with and without a router: NetworkManager's own
    # automatic profile ("Wired connection 1") gives up when no DHCP server
    # answers (a cable between two computers, a switch with no router), and
    # link-local IPv4 alone does not keep it up (vm-cable). Instead, two
    # profiles for any wired card: "Wired" (DHCP and router advertisements)
    # first, once; when it fails, "Wired without router" (link-local IPv4
    # and IPv6 only), which comes up at once, so that local messaging can
    # use the cable.
    settings.main.no-auto-default = lib.mkDefault "*";
    ensureProfiles.profiles = {
      resilience-wired = {
        connection = {
          id = "Wired";
          uuid = "6d0f4c84-2f6a-4f53-9a0b-7f0b6c9a5e11";
          type = "ethernet";
          autoconnect = "true";
          autoconnect-priority = "10";
          autoconnect-retries = "1";
        };
        ipv4.method = "auto";
        ipv6.method = "auto";
      };
      resilience-wired-local = {
        connection = {
          id = "Wired without router";
          uuid = "0b9f2e61-7c1d-4b8a-a3e2-5d6c8f1e2a47";
          type = "ethernet";
          autoconnect = "true";
          autoconnect-priority = "-10";
        };
        ipv4.method = "link-local";
        ipv6.method = "link-local";
      };
    };
  };
  security.polkit.extraConfig = lib.mkIf config.networking.networkmanager.enable ''
    // Resilience (ADR 0010): NetworkManager for accounts without the
    // networkmanager group: only what joining Wi-Fi needs.
    polkit.addRule(function(action, subject) {
      if (action.id.indexOf("org.freedesktop.NetworkManager.") == 0 &&
          !subject.isInGroup("networkmanager") && !subject.isInGroup("wheel")) {
        var allowed = ${builtins.toJSON nmAllowedWithoutGroup};
        if (allowed.indexOf(action.id) < 0) {
          return polkit.Result.NO;
        }
      }
    });
  '';

  # ModemManager (cellular modems) is a resident service with a ~70 MiB
  # closure that most machines do not need. Enable it for 3G/4G/5G modems.
  # NetworkManager enables it with mkDefault; 900 beats that but still
  # yields to a plain `networking.modemmanager.enable = true;`.
  networking.modemmanager.enable = lib.mkOverride 900 false;

  networking.firewall = {
    enable = true;
    allowedTCPPorts = lib.mkDefault [ ];
    allowedUDPPorts = lib.mkDefault [ ];
  };

  # NTP would contact public servers periodically. Off by default; enable
  # explicitly when wanted (docs/OFFLINE_OPERATIONS.md).
  services.timesyncd.enable = lib.mkDefault false;
}
