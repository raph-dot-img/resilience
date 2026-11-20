# A Wi-Fi mesh for local messaging without a router (ADR 0021): BATMAN-adv
# over Wi-Fi ad hoc (IBSS). Available where local messaging is, off until
# someone turns it on (`resilience-admin set-mesh on|off`, allowed to the
# person at the console), or always on with `autoStart`.
#
#   resilience-mesh.service  root, oneshot, kept active: the card leaves
#       NetworkManager, joins the IBSS cell and becomes the only member of
#       bat0, which has only an IPv6 link-local address. Stopping it undoes
#       everything. Both restart the Reticulum instance, which lists its
#       interfaces only when it starts.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  inherit (lib) mkOption types;
  cfg = config.resilience.mesh;
  nm = config.networking.networkmanager.enable;
  path = lib.makeBinPath [
    pkgs.iproute2
    pkgs.iw
    pkgs.kmod
    pkgs.util-linux
    pkgs.coreutils
    pkgs.procps
    pkgs.gnugrep
    config.systemd.package
  ];
  nmcli = "${config.networking.networkmanager.package}/bin/nmcli";
  devices = lib.concatStringsSep " " cfg.devices;

  start = pkgs.writeShellScript "resilience-mesh-start" ''
    set -eu
    export PATH=${path}
    devs="${devices}"
    if [ -z "$devs" ]; then
      for d in /sys/class/net/*; do
        if [ -d "$d/wireless" ] || [ -e "$d/phy80211" ]; then
          devs=$(basename "$d"); break
        fi
      done
    fi
    if [ -z "$devs" ]; then
      echo "no Wi-Fi card: the mesh needs one" >&2
      exit 1
    fi
    if [ -e /run/resilience-hotspot/device ]; then
      echo "the hotspot is on: turn it off first (the Wi-Fi card cannot do both)" >&2
      exit 1
    fi
    # Written first: a start that fails half way is undone from it.
    echo "$devs" > /run/resilience-mesh/devices
    modprobe batman_adv
    ip link show bat0 > /dev/null 2>&1 || ip link add bat0 type batadv
    # Only a link-local address: no router advertisements, no SLAAC.
    sysctl -q -w net.ipv6.conf.bat0.accept_ra=0 net.ipv6.conf.bat0.autoconf=0 \
      net.ipv6.conf.bat0.forwarding=0 net.ipv4.conf.bat0.forwarding=0
    for dev in $devs; do
      ${lib.optionalString nm ''
        # Remembered, so that stopping gives each card back as it was.
        if ${nmcli} -g GENERAL.STATE device show "$dev" 2>/dev/null | grep -q unmanaged; then
          echo "$dev" >> /run/resilience-mesh/unmanaged
        fi
        ${nmcli} device set "$dev" managed no || true
      ''}
      ip link set "$dev" down
      if [ -d "/sys/class/net/$dev/wireless" ] || [ -e "/sys/class/net/$dev/phy80211" ]; then
        rfkill unblock wlan || true
        iw dev "$dev" set type ibss
        # Room for BATMAN-adv's header; bat0 fragments when it cannot.
        ip link set "$dev" mtu 1532 2>/dev/null || true
        ip link set "$dev" up
        iw dev "$dev" ibss join ${lib.escapeShellArg cfg.name} ${toString cfg.frequency} fixed-freq ${cfg.cellId}
      else
        ip link set "$dev" mtu 1532 2>/dev/null || true
        ip link set "$dev" up
      fi
      ip link set "$dev" master bat0
    done
    ip link set bat0 up
    ${lib.optionalString config.resilience.comms.enable "systemctl --no-block try-restart resilience-reticulum.service"}
  '';

  stop = pkgs.writeShellScript "resilience-mesh-stop" ''
    set -u
    export PATH=${path}
    devs=$(cat /run/resilience-mesh/devices 2>/dev/null || true)
    for dev in $devs; do
      ip link set "$dev" nomaster 2>/dev/null
      if [ -d "/sys/class/net/$dev/wireless" ] || [ -e "/sys/class/net/$dev/phy80211" ]; then
        iw dev "$dev" ibss leave 2>/dev/null
        ip link set "$dev" down
        iw dev "$dev" set type managed
        ip link set "$dev" mtu 1500 2>/dev/null
      fi
      ${lib.optionalString nm ''
        grep -qx "$dev" /run/resilience-mesh/unmanaged 2>/dev/null || ${nmcli} device set "$dev" managed yes
      ''}
    done
    rm -f /run/resilience-mesh/devices /run/resilience-mesh/unmanaged
    if ip link show bat0 > /dev/null 2>&1; then
      ip link del bat0
      ${lib.optionalString config.resilience.comms.enable "systemctl --no-block try-restart resilience-reticulum.service"}
    fi
    exit 0
  '';
in
{
  options.resilience.mesh = {
    enable = mkOption {
      type = types.bool;
      default = config.resilience.comms.enable;
      defaultText = lib.literalExpression "config.resilience.comms.enable";
      description = ''
        Offer a Wi-Fi mesh for local messaging without a router (ADR 0021):
        BATMAN-adv over Wi-Fi ad hoc. Nothing happens until someone turns it
        on (`resilience-admin set-mesh on`, from the interface) or
        `autoStart` is set. While it is on, the Wi-Fi card cannot join
        ordinary Wi-Fi networks, the computer relays other computers'
        traffic, and anyone in radio range can join the mesh.
      '';
    };
    autoStart = mkOption {
      type = types.bool;
      default = false;
      description = "Turn the mesh on at every boot (for a relay computer left on).";
    };
    name = mkOption {
      type = types.strMatching "[A-Za-z0-9._-]{1,32}";
      default = "resilience-mesh";
      description = "Name of the Wi-Fi ad hoc cell. Every computer of a mesh needs the same.";
    };
    frequency = mkOption {
      type = types.ints.between 2412 2462;
      default = 2412;
      description = "Frequency of the cell in MHz (2.4 GHz channels 1 to 11; default channel 1).";
    };
    cellId = mkOption {
      type = types.strMatching "([0-9a-f]{2}:){5}[0-9a-f]{2}";
      default = "02:52:45:53:4d:31";
      description = "Fixed cell id (BSSID), so that cells formed separately merge.";
    };
    devices = mkOption {
      type = types.listOf (types.strMatching "[A-Za-z0-9._-]{1,15}");
      default = [ ];
      example = [
        "wlan0"
        "eth1"
      ];
      description = ''
        Network interfaces that join the mesh. Empty: the first Wi-Fi card.
        Ethernet interfaces can be listed to join a mesh by cable.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    boot.kernelModules = [ "batman_adv" ];
    environment.systemPackages = [ pkgs.batctl ];
    networking.networkmanager.unmanaged = [ "interface-name:bat0" ];

    systemd.services.resilience-mesh = {
      description = "Wi-Fi mesh of Resilience (BATMAN-adv over Wi-Fi ad hoc)";
      wantedBy = lib.mkIf cfg.autoStart [ "multi-user.target" ];
      after = [
        "network.target"
        "NetworkManager.service"
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        RuntimeDirectory = "resilience-mesh";
        RuntimeDirectoryPreserve = "no";
        ExecStart = start;
        # On stop, and after a start that failed half way.
        ExecStopPost = stop;
        TimeoutSec = 60;
      };
    };
  };
}
