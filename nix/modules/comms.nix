# Local messaging network (ADR 0018): one Reticulum instance per machine,
# off by default.
#
#   resilience-reticulum.service  rnsd as a dynamic unprivileged user;
#       AutoInterface only (IPv6 link-local multicast on the local
#       network), no transport, no remote management.
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
    ${lib.optionalString (
      cfg.interfaces != [ ]
    ) "    devices = ${lib.concatStringsSep "," cfg.interfaces}"}
  '';
  clientConfig = pkgs.writeText "reticulum-client-config" ''
    ${reticulumSection}
    [logging]
      loglevel = 3

    [interfaces]
  '';

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

    systemd.services.resilience-reticulum = {
      description = "Reticulum network instance of Resilience (local messaging)";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      serviceConfig = {
        ExecStartPre = prepare;
        ExecStart = "${cfg.package}/bin/rnsd --config /var/lib/resilience-reticulum";
        Restart = "on-failure";
        RestartSec = 5;
        DynamicUser = true;
        StateDirectory = "resilience-reticulum";
        StateDirectoryMode = "0700";
        RuntimeDirectory = "resilience-reticulum";
        RuntimeDirectoryMode = "0755";
        # Unprivileged, and nothing to hold on to.
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
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_INET"
          "AF_INET6"
          "AF_NETLINK"
        ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [
          "@system-service"
          "~@privileged"
        ];
        MemoryMax = "256M";
      };
    };
  };
}
