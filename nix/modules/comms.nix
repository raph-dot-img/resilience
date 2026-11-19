# Local messaging network (ADR 0018): one Reticulum instance per machine,
# off by default.
#
#   resilience-reticulum.service  rnsd as a dynamic unprivileged user;
#       AutoInterface only (IPv6 link-local multicast on the local
#       network), no transport, no remote management.
#   resilience-messages (systemd user service)  each account's messaging
#       program, with its identity and messages in its own home.
#   resilience-messages-node.service  the machine's message store (ADR
#       0019): an LXMF propagation node that keeps messages, encrypted for
#       their recipients, for people who are away; unprivileged, bounded,
#       peering only with the stores of the local network.
#   /run/resilience-reticulum/client/config  what programs of users need
#       to attach to the instance (instance name and RPC key). Readable by
#       every local account on purpose (ADR 0018 §4); never in the store.
#
# Reticulum and LXMF are under the Reticulum License, which is not a free
# software licence: see nix/packages/resilience-reticulum.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  inherit (lib) mkOption types;
  cfg = config.resilience.comms;

  instance = "resilience";
  discoveryPort = 29716;
  dataPort = 42671;

  # Everything but the RPC key, which the service adds at start.
  reticulumSection = ''
    [reticulum]
      share_instance = Yes
      instance_name = ${instance}
      enable_transport = No
      enable_remote_management = No
      respond_to_probes = No
      panic_on_interface_error = No
  '';
  serverConfig = pkgs.writeText "reticulum-server-config" ''
    ${reticulumSection}
    [logging]
      loglevel = 3

    [interfaces]
      [[Local network]]
        type = AutoInterface
        enabled = Yes
        group_id = ${cfg.group}
        discovery_port = ${toString discoveryPort}
        data_port = ${toString dataPort}
    ${lib.optionalString (cfg.interfaces != [ ])
      "    devices = ${lib.concatStringsSep "," (cfg.interfaces ++ meshInterface)}"
    }
  '';
  clientConfig = pkgs.writeText "reticulum-client-config" ''
    ${reticulumSection}
    [logging]
      loglevel = 3

    [interfaces]
  '';

  storeAddress = "/run/resilience-messages-node/address";

  # The mesh (ADR 0021) adds bat0 while it is on.
  meshInterface = lib.optional config.resilience.mesh.enable "bat0";

  # Attaching before rnsd's local socket exists fails (exit 75, ADR 0019):
  # wait for it, so that a boot does not go through several restarts.
  waitForInstance = pkgs.writeShellScript "resilience-wait-for-reticulum" ''
    for _ in $(${pkgs.coreutils}/bin/seq 60); do
      ${pkgs.gnugrep}/bin/grep -q '@rns/${instance}$' /proc/net/unix && exit 0
      ${pkgs.coreutils}/bin/sleep 0.5
    done
    exit 0
  '';

  hardening = {
    NoNewPrivileges = true;
    CapabilityBoundingSet = "";
    ProtectSystem = "strict";
    ProtectHome = true;
    PrivateTmp = true;
    PrivateDevices = true;
    ProtectKernelTunables = true;
    ProtectKernelModules = true;
    ProtectKernelLogs = true;
    ProtectControlGroups = true;
    ProtectClock = true;
    ProtectHostname = true;
    RestrictNamespaces = true;
    RestrictRealtime = true;
    RestrictSUIDSGID = true;
    LockPersonality = true;
    SystemCallArchitectures = "native";
    SystemCallFilter = [
      "@system-service"
      "~@privileged"
    ];
  };

  prepare = pkgs.writeShellScript "resilience-reticulum-prepare" ''
    set -eu
    umask 077
    state="$STATE_DIRECTORY"
    run="$RUNTIME_DIRECTORY"
    if ! ${pkgs.gnugrep}/bin/grep -Eqx '[0-9a-f]{64}' "$state/rpc-key" 2>/dev/null; then
      ${pkgs.coreutils}/bin/head -c 32 /dev/urandom | ${pkgs.coreutils}/bin/od -An -v -tx1 \
        | ${pkgs.coreutils}/bin/tr -d ' \n' > "$state/rpc-key.new"
      ${pkgs.coreutils}/bin/mv "$state/rpc-key.new" "$state/rpc-key"
    fi
    key=$(${pkgs.coreutils}/bin/cat "$state/rpc-key")
    with_key() {
      ${pkgs.gnused}/bin/sed "s/^  instance_name = .*/&\n  rpc_key = $key/" "$1"
    }
    with_key ${serverConfig} > "$state/config"
    umask 022
    ${pkgs.coreutils}/bin/mkdir -p "$run/client"
    with_key ${clientConfig} > "$run/client/config.new"
    ${pkgs.coreutils}/bin/mv "$run/client/config.new" "$run/client/config"
  '';
in
{
  options.resilience.comms = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Local messaging between nearby machines without the Internet
        (ADR 0018), using Reticulum and LXMF. The machine then announces
        itself on its local network (IPv6 multicast, UDP ports 29716 and
        42671, opened in the firewall).

        Reticulum and LXMF are under the Reticulum License, which is not a
        free software licence: they may not be used in systems able to
        purposefully harm people, nor to create training data for
        artificial intelligence. Enabling this option accepts those terms.
      '';
    };
    package = mkOption {
      type = types.package;
      default = pkgs.callPackage ../packages/resilience-reticulum { };
      defaultText = lib.literalExpression "pkgs.callPackage ./nix/packages/resilience-reticulum { }";
      description = "Python environment with Reticulum (rns) and LXMF.";
    };
    messagesPackage = mkOption {
      type = types.package;
      default = pkgs.callPackage ../packages/resilience-messages { reticulum = cfg.package; };
      defaultText = lib.literalExpression "pkgs.callPackage ./nix/packages/resilience-messages { reticulum = config.resilience.comms.package; }";
      description = "The per-user messaging program `resilience-messages`.";
    };
    group = mkOption {
      type = types.strMatching "[a-z0-9]{1,32}";
      default = "reticulum";
      description = ''
        Reticulum's network name on the local network: machines find each
        other only with the same name. The default is Reticulum's own, so
        that other Reticulum programs on the network are reachable.
      '';
    };
    interfaces = mkOption {
      type = types.listOf (types.strMatching "[A-Za-z0-9._-]{1,15}");
      default = [ ];
      example = [ "wlan0" ];
      description = "Network interfaces to use. Empty: every local network interface.";
    };
    store = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Run a message store on this machine (ADR 0019): an LXMF
          propagation node that keeps messages for people whose computer
          is off or who are logged out, and exchanges them with the stores
          of other machines on the local network. Messages in it are
          encrypted for their recipients; the store cannot read them. It
          uses about 40 MB of memory and at most `limitMB` of disk.
        '';
      };
      limitMB = mkOption {
        type = types.ints.between 1 100000;
        default = 32;
        description = "Disk space for the messages the store keeps for others, in megabytes (1 MB holds hundreds of short messages).";
      };
    };
    clientConfig = mkOption {
      type = types.path;
      readOnly = true;
      default = "/run/resilience-reticulum/client/config";
      description = "The configuration with which programs of users attach to the instance.";
    };
  };

  config = lib.mkIf cfg.enable {
    networking.firewall.allowedUDPPorts = [
      discoveryPort
      dataPort
    ];

    # Each account's own messaging program, while it is logged in: its
    # identity and messages stay in its home (ADR 0018 §5).
    systemd.user.services.resilience-messages = {
      description = "Local messaging of this account (Resilience)";
      wantedBy = [ "default.target" ];
      unitConfig.ConditionUser = "!root";
      serviceConfig = {
        ExecStart = "${cfg.messagesPackage}/bin/resilience-messages run --client-config ${cfg.clientConfig} --local-store ${storeAddress}";
        Restart = "on-failure";
        RestartSec = 10;
        NoNewPrivileges = true;
        MemoryMax = "256M";
      };
    };

    # Reticulum lists the network cards only when it starts: a cable
    # plugged in or a Wi-Fi network joined afterwards would never be used.
    # When NetworkManager brings a connection up, restart the instance
    # (the messaging programs and the store reattach and announce again).
    networking.networkmanager.dispatcherScripts = lib.mkIf config.networking.networkmanager.enable [
      {
        type = "basic";
        source = pkgs.writeShellScript "resilience-reticulum-dispatcher" ''
          case "''${2:-}" in
            up)
              case "''${1:-}" in
                ""|lo|bat0) ;;
                *) ${config.systemd.package}/bin/systemctl --no-block try-restart resilience-reticulum.service ;;
              esac
              ;;
          esac
          exit 0
        '';
      }
    ];

    systemd.services.resilience-messages-node = lib.mkIf cfg.store.enable {
      description = "Message store of Resilience (local messaging, store-and-forward)";
      # Started, stopped and restarted with the instance (and if the
      # instance crashes, Reticulum reattaches the store by itself).
      wantedBy = [
        "multi-user.target"
        "resilience-reticulum.service"
      ];
      partOf = [ "resilience-reticulum.service" ];
      after = [ "resilience-reticulum.service" ];
      serviceConfig = hardening // {
        ExecStartPre = waitForInstance;
        ExecStart = lib.concatStringsSep " " [
          "${cfg.messagesPackage}/bin/resilience-messages node"
          "--dir /var/lib/resilience-messages-node"
          "--client-config ${cfg.clientConfig}"
          "--address-file ${storeAddress}"
          "--limit-mb ${toString cfg.store.limitMB}"
        ];
        Restart = "on-failure";
        RestartSec = 5;
        DynamicUser = true;
        StateDirectory = "resilience-messages-node";
        StateDirectoryMode = "0700";
        RuntimeDirectory = "resilience-messages-node";
        RuntimeDirectoryMode = "0755";
        UMask = "0077";
        # It only talks to the machine's instance, over its local socket.
        RestrictAddressFamilies = [ "AF_UNIX" ];
        MemoryMax = "192M";
      };
    };

    systemd.services.resilience-reticulum = {
      description = "Reticulum network instance of Resilience (local messaging)";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      # Unprivileged, and nothing to hold on to.
      serviceConfig = hardening // {
        ExecStartPre = prepare;
        ExecStart = "${cfg.package}/bin/rnsd --config /var/lib/resilience-reticulum";
        Restart = "on-failure";
        RestartSec = 5;
        DynamicUser = true;
        StateDirectory = "resilience-reticulum";
        StateDirectoryMode = "0700";
        RuntimeDirectory = "resilience-reticulum";
        RuntimeDirectoryMode = "0755";
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_INET"
          "AF_INET6"
          "AF_NETLINK"
        ];
        MemoryMax = "256M";
      };
    };
  };
}
