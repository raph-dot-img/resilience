# A hotspot: this computer as a Wi-Fi network for others (ADR 0025 §6),
# turned on and off only by `resilience-admin set-hotspot on|off`
# (polkit auth_admin). The helper makes a new password each time, leaves
# it in a root-only file and shows it once.
#
#   resilience-hotspot.service  root, oneshot, kept active: writes a
#       NetworkManager profile (WPA2, `ipv4.method shared` = DHCP and DNS
#       by NetworkManager's dnsmasq, IPv6 off) into the root-only
#       in-memory profile directory (the password never appears on a
#       command line), opens DHCP and DNS on that card only, drops every
#       packet forwarded from or to it (NetworkManager's shared mode would
#       route the hotspot to the Internet), and brings it up.
#   /etc/NetworkManager/dnsmasq-shared.d/resilience.conf: the hotspot's
#       DNS answers names in home.arpa only (`resilience.home.arpa` = this
#       computer, plus the records of `dnsRecords`) and forwards nothing.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  inherit (lib) mkOption types;
  cfg = config.resilience.share.hotspot;
  nmcli = "${config.networking.networkmanager.package}/bin/nmcli";
  address = "10.42.0.1";
  profile = "/run/NetworkManager/system-connections/resilience-hotspot.nmconnection";
  path = lib.makeBinPath [
    pkgs.coreutils
    pkgs.gnugrep
    pkgs.iptables
    pkgs.util-linux
  ];

  # DHCP and DNS on the card, nothing forwarded from or to it.
  rules = pkgs.writeShellScript "resilience-hotspot-firewall" ''
    set -eu
    export PATH=${path}
    iptables -w -F resilience-hotspot
    for p in "udp 67" "udp 53" "tcp 53"; do
      set -- $p
      iptables -w -A resilience-hotspot -i "$DEV" -p "$1" --dport "$2" -j nixos-fw-accept
    done
    for t in iptables ip6tables; do
      $t -w -F resilience-hotspot-fwd
      $t -w -A resilience-hotspot-fwd -i "$DEV" -j DROP
      $t -w -A resilience-hotspot-fwd -o "$DEV" -j DROP
    done
  '';

  start = pkgs.writeShellScript "resilience-hotspot-start" ''
    set -eu
    export PATH=${path}
    d=/run/resilience-hotspot
    dev=${lib.escapeShellArg (if cfg.device != null then cfg.device else "")}
    if [ -z "$dev" ]; then
      for x in /sys/class/net/*; do
        if [ -d "$x/wireless" ] || [ -e "$x/phy80211" ]; then
          dev=$(basename "$x"); break
        fi
      done
    fi
    if [ -z "$dev" ]; then
      echo "no Wi-Fi card: the hotspot needs one" >&2
      exit 1
    fi
    if [ -e /run/resilience-mesh/devices ]; then
      echo "the mesh network is on: turn it off first" >&2
      exit 1
    fi
    psk=$(cat "$d/password")
    case "$psk" in
      ""|*[!a-z0-9-]*) echo "no valid password" >&2; exit 1 ;;
    esac
    # Written first: a start that fails half way is undone from it.
    echo "$dev" > "$d/device"
    uuid=$(cat /proc/sys/kernel/random/uuid)
    install -d -m 0700 /run/NetworkManager/system-connections
    umask 077
    cat > ${profile}.tmp <<EOF
    [connection]
    id=resilience-hotspot
    uuid=$uuid
    type=wifi
    interface-name=$dev
    autoconnect=false
    [wifi]
    mode=ap
    ssid=${cfg.ssid}
    band=bg
    [wifi-security]
    key-mgmt=wpa-psk
    proto=rsn
    pairwise=ccmp
    group=ccmp
    psk=$psk
    [ipv4]
    method=shared
    address1=${address}/24
    [ipv6]
    method=disabled
    EOF
    mv ${profile}.tmp ${profile}
    DEV="$dev" ${rules}
    rfkill unblock wlan || true
    ${nmcli} device set "$dev" managed yes || true
    # Just unblocked or just given back by the mesh: wait until usable.
    for _ in $(seq 40); do
      case "$(${nmcli} -g GENERAL.STATE device show "$dev" 2>/dev/null)" in
        ""|*unavailable*|*unmanaged*) sleep 0.5 ;;
        *) break ;;
      esac
    done
    ${nmcli} connection reload
    if ! ${nmcli} --wait 45 connection up id resilience-hotspot; then
      ${nmcli} device >&2 || true
      exit 1
    fi
    # For the interface (no secret): which card, which network.
    install -d -m 0755 /run/resilience-share
    printf 'device = "%s"\nssid = "%s"\n' "$dev" ${lib.escapeShellArg cfg.ssid} > /run/resilience-share/hotspot.tmp
    chmod 0644 /run/resilience-share/hotspot.tmp
    mv /run/resilience-share/hotspot.tmp /run/resilience-share/hotspot
  '';

  stop = pkgs.writeShellScript "resilience-hotspot-stop" ''
    set -u
    export PATH=${path}
    ${nmcli} --wait 20 connection down id resilience-hotspot 2>/dev/null
    rm -f ${profile} ${profile}.tmp /run/resilience-share/hotspot
    ${nmcli} connection reload 2>/dev/null
    for t in iptables ip6tables; do
      $t -w -F resilience-hotspot-fwd 2>/dev/null
    done
    iptables -w -F resilience-hotspot 2>/dev/null
    rm -f /run/resilience-hotspot/device
    exit 0
  '';

  reload = pkgs.writeShellScript "resilience-hotspot-reload" ''
    set -eu
    DEV=$(cat /run/resilience-hotspot/device) ${rules}
  '';

  records = lib.concatStrings (
    lib.mapAttrsToList (n: ip: "address=/${n}.home.arpa/${ip}\n") cfg.dnsRecords
  );
in
{
  options.resilience.share.hotspot = {
    enable = mkOption {
      type = types.bool;
      default = config.resilience.share.enable && config.networking.networkmanager.enable;
      defaultText = lib.literalExpression "config.resilience.share.enable";
      description = ''
        Offer a hotspot (ADR 0025 §6): an administrator can turn this
        computer into a Wi-Fi network (WPA2, a new password each time) on
        which others get an address and can reach what it shares. It never
        routes them to other networks. Off until turned on.
      '';
    };
    ssid = mkOption {
      type = types.strMatching "[A-Za-z0-9_-]{1,32}";
      default = "Resilience-hotspot";
      description = "Name of the hotspot's Wi-Fi network.";
    };
    device = mkOption {
      type = types.nullOr (types.strMatching "[A-Za-z][A-Za-z0-9_.]{0,14}");
      default = null;
      description = "The Wi-Fi card to use; null: the first one.";
    };
    dnsRecords = mkOption {
      type = types.attrsOf (
        types.strMatching "(25[0-5]|2[0-4][0-9]|1?[0-9]{1,2})(\\.(25[0-5]|2[0-4][0-9]|1?[0-9]{1,2})){3}"
      );
      default = { };
      example = {
        library = "10.42.0.1";
        printer = "10.42.0.20";
      };
      description = ''
        Expert option: more names answered by the hotspot's DNS, as
        `NAME.home.arpa` (RFC 8375) = IPv4 address. `resilience.home.arpa`
        is always this computer (${address}). Names in home.arpa only;
        nothing is forwarded to other DNS servers.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.all (n: builtins.match "[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?" n != null) (
          lib.attrNames cfg.dnsRecords
        );
        message = "resilience.share.hotspot.dnsRecords: names are lower-case letters, digits and dashes (the part before .home.arpa).";
      }
    ];

    systemd.tmpfiles.rules = [ "d /run/resilience-hotspot 0700 root root -" ];

    environment.etc."NetworkManager/dnsmasq-shared.d/resilience.conf".text = ''
      # Resilience hotspot (ADR 0025 §7): names in home.arpa only, never
      # forwarded (no Internet through this computer, not even DNS).
      no-resolv
      local=/home.arpa/
      address=/resilience.home.arpa/${address}
      ${records}'';

    networking.firewall.extraCommands = ''
      iptables -N resilience-hotspot 2>/dev/null || true
      iptables -F resilience-hotspot
      iptables -D nixos-fw -j resilience-hotspot 2>/dev/null || true
      iptables -I nixos-fw -j resilience-hotspot
      # Never flushed here: its DROP rules must survive a firewall reload
      # without a gap (the hotspot unit fills and empties it).
      ip46tables -N resilience-hotspot-fwd 2>/dev/null || true
      iptables -C FORWARD -j resilience-hotspot-fwd 2>/dev/null \
        || iptables -I FORWARD -j resilience-hotspot-fwd
      ip6tables -C FORWARD -j resilience-hotspot-fwd 2>/dev/null \
        || ip6tables -I FORWARD -j resilience-hotspot-fwd
    '';
    networking.firewall.extraStopCommands = ''
      iptables -D nixos-fw -j resilience-hotspot 2>/dev/null || true
      iptables -F resilience-hotspot 2>/dev/null || true
      # The forward chain is left alone: a firewall reload runs these
      # commands too, and its DROP rules must hold throughout. Stopping
      # the firewall stops the hotspot (PartOf), which empties it.
    '';

    systemd.services.resilience-hotspot = {
      description = "Hotspot of Resilience (this computer as a Wi-Fi network, ADR 0025)";
      after = [
        "NetworkManager.service"
        "firewall.service"
      ];
      requires = [ "NetworkManager.service" ];
      partOf = [ "firewall.service" ];
      unitConfig.ReloadPropagatedFrom = [ "firewall.service" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = start;
        ExecReload = reload;
        # On stop, and after a start that failed half way.
        ExecStopPost = stop;
        TimeoutSec = 90;
      };
    };
  };
}
