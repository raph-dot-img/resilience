# Community network, server side (ADR 0025): share packs, the library, a
# folder and web pages with nearby machines, read-only, on ONE network
# interface, only while an administrator wants it.
#
#   resilience-admin set-share on IF --name NAME SERVICE... | off
#       (polkit auth_admin every time, also to stop) writes
#       /run/resilience-share/{services,name} and starts:
#   resilience-share@IF.service          root oneshot, kept active: checks
#       the request again, opens TCP 80 on IF alone (chain
#       `resilience-share`), writes the nginx includes and the index page,
#       state.toml for the interface. Stopping undoes all of it. Bound to
#       the interface's device; a NetworkManager dispatcher script stops it
#       when the interface's connection goes down or changes.
#   resilience-share-web@IF.service      nginx, DynamicUser, GET/HEAD only,
#       limits per address, no access log.
#   resilience-share-library@IF.service  kiwix-serve on 127.0.0.1:81
#       (a privileged port: no local account can take it first), behind
#       nginx under /library/.
# Not kept across reboots, except with `autoStart`.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  inherit (lib) mkOption types;
  cfg = config.resilience.share;
  packsCfg = config.resilience.packs;
  comms = config.resilience.comms;
  stateDir = "/run/resilience-share";
  sharedDir = "/var/lib/resilience/shared";
  libraryPort = 81;
  nmcli = "${config.networking.networkmanager.package}/bin/nmcli";
  nm = config.networking.networkmanager.enable;
  services = [
    "packs"
    "library"
    "files"
    "pages"
  ];

  # Files of the catalog: the only ones /packs/ lists and serves.
  catalogFiles = pkgs.writeText "resilience-share-catalog-files" (
    lib.concatMapStrings (p: p.file + "\n") (lib.optionals packsCfg.enable packsCfg.catalog)
  );

  # Headers for content other machines' browsers (phones too) render:
  # no scripts, nothing from elsewhere.
  strictHeaders = ''
    add_header X-Content-Type-Options nosniff always;
    add_header Content-Security-Policy "default-src 'none'; img-src 'self'; style-src 'self'; media-src 'self'" always;
    add_header Referrer-Policy no-referrer always;
  '';

  snippets = pkgs.linkFarm "resilience-share-locations" [
    {
      name = "packs.conf";
      # One exact location per catalog file: nothing else in the
      # directory is ever served.
      path = pkgs.writeText "packs.conf" (
        lib.concatMapStrings (p: ''
          location = /packs/${p.file} {
            alias ${packsCfg.dir}/${p.file};
            default_type application/octet-stream;
            ${strictHeaders}
          }
        '') (lib.optionals packsCfg.enable packsCfg.catalog)
      );
    }
    {
      name = "library.conf";
      path = pkgs.writeText "library.conf" ''
        location = /library { return 301 /library/; }
        location /library/ {
          proxy_pass http://127.0.0.1:${toString libraryPort};
          proxy_http_version 1.1;
          proxy_set_header Host $host;
          proxy_set_header Connection "";
          proxy_connect_timeout 5s;
          proxy_read_timeout 30s;
          proxy_send_timeout 30s;
          proxy_buffering on;
          proxy_max_temp_file_size 0;
          add_header X-Content-Type-Options nosniff always;
        }
      '';
    }
    {
      name = "files.conf";
      path = pkgs.writeText "files.conf" ''
        location /files/ {
          alias ${sharedDir}/files/;
          autoindex on;
          autoindex_exact_size on;
          charset utf-8;
          ${strictHeaders}
        }
      '';
    }
    {
      name = "pages.conf";
      path = pkgs.writeText "pages.conf" ''
        location /pages/ {
          alias ${sharedDir}/pages/;
          index index.html;
          charset utf-8;
          ${strictHeaders}
        }
      '';
    }
  ];

  nginxConf = pkgs.writeText "resilience-share-nginx.conf" ''
    daemon off;
    master_process on;
    worker_processes 1;
    pid /run/resilience-share-web/nginx.pid;
    error_log stderr warn;
    events { worker_connections 256; }
    http {
      include ${pkgs.nginx}/conf/mime.types;
      default_type application/octet-stream;
      access_log off;
      server_tokens off;
      # Relative redirects: a client reading through its loopback relay
      # (ADR 0025 §5) stays on the relay.
      absolute_redirect off;
      sendfile on;
      client_body_temp_path /run/resilience-share-web/body;
      proxy_temp_path /run/resilience-share-web/proxy;
      fastcgi_temp_path /run/resilience-share-web/fastcgi;
      uwsgi_temp_path /run/resilience-share-web/uwsgi;
      scgi_temp_path /run/resilience-share-web/scgi;
      client_max_body_size 1k;
      client_body_buffer_size 1k;
      client_header_buffer_size 1k;
      large_client_header_buffers 4 4k;
      client_header_timeout 10s;
      client_body_timeout 10s;
      send_timeout 30s;
      keepalive_timeout 10s;
      keepalive_requests 100;
      reset_timedout_connection on;
      limit_conn_zone $binary_remote_addr zone=peraddr:1m;
      limit_req_zone $binary_remote_addr zone=reqs:1m rate=20r/s;
      limit_conn_status 429;
      limit_req_status 429;
      server {
        listen 80 default_server;
        listen [::]:80 default_server ipv6only=on;
        server_name _;
        limit_conn peraddr 6;
        limit_req zone=reqs burst=40 nodelay;
        disable_symlinks on;
        autoindex off;
        if ($request_method !~ ^(GET|HEAD)$) { return 405; }
        location ~ "/\." { return 404; }
        location = / {
          root ${stateDir}/www;
          try_files /index.html =404;
          ${strictHeaders}
        }
        location / { return 404; }
        include ${stateDir}/conf/*.conf;
      }
    }
  '';

  path = lib.makeBinPath [
    pkgs.coreutils
    pkgs.gnugrep
    pkgs.iptables
    pkgs.util-linux
    config.systemd.package
  ];

  # Open TCP 80 on the interface alone (the chain is empty otherwise).
  openPort = pkgs.writeShellScript "resilience-share-firewall" ''
    set -eu
    export PATH=${path}
    for t in iptables ip6tables; do
      $t -w -F resilience-share
      $t -w -A resilience-share -i "$1" -p tcp --dport 80 -j nixos-fw-accept
    done
  '';

  start = pkgs.writeShellScript "resilience-share-start" ''
    set -eu
    export PATH=${path}
    iface=$1
    d=${stateDir}
    # Checked again here (the helper did): a name, not a path.
    echo "$iface" | grep -Eqx '[A-Za-z][A-Za-z0-9_.]{0,14}'
    ${lib.optionalString (cfg.autoStart != null) ''
      if [ ! -e "$d/services" ] && [ "$iface" = ${lib.escapeShellArg cfg.autoStart.interface} ]; then
        echo ${lib.escapeShellArg (lib.concatStringsSep " " cfg.autoStart.services)} > "$d/services"
        echo ${lib.escapeShellArg cfg.autoStart.name} > "$d/name"
      fi
    ''}
    services=$(cat "$d/services")
    name=$(cat "$d/name")
    echo "$name" | grep -Eqx '[A-Za-z0-9]([A-Za-z0-9 -]{0,30}[A-Za-z0-9])?'
    rm -rf "$d/conf" "$d/www"
    mkdir -m 0755 "$d/conf" "$d/www"
    rm -f "$d/library"
    list=""
    for s in $services; do
      case "$s" in
        packs|library|files|pages) ;;
        *) echo "unknown service $s" >&2; exit 1 ;;
      esac
      echo " ${lib.concatStringsSep " " services} " | grep -q " $s " || exit 1
      cp ${snippets}/"$s".conf "$d/conf/$s.conf"
      [ "$s" != library ] || touch "$d/library"
      list="$list\"$s\", "
    done
    chmod 0644 "$d"/conf/*.conf

    # The index page: what is shared, readable in lynx.
    {
      echo '<!DOCTYPE html><html><head><meta charset="utf-8">'
      echo "<title>$name (Resilience)</title></head><body>"
      echo "<h1>$name</h1>"
      echo '<p>A Resilience computer sharing offline resources on this network.'
      echo 'Anyone on the network can read what is here. Packs can be checked'
      echo 'against the catalog of a Resilience system; other content is as'
      echo 'trustworthy as whoever runs this computer.</p><ul>'
      for s in $services; do
        case "$s" in
          library) echo '<li><a href="/library/">Library</a> (encyclopedias, guides, documentation)</li>' ;;
          files) echo '<li><a href="/files/">Shared folder</a></li>' ;;
          pages) echo '<li><a href="/pages/">Pages</a></li>' ;;
          packs) echo '<li><a href="#packs">Packs to download</a></li>' ;;
        esac
      done
      echo '</ul>'
      case " $services " in
        *" packs "*)
          echo '<h2 id="packs">Packs</h2><ul>'
          for f in ${packsCfg.dir}/*; do
            b=$(basename "$f")
            [ -f "$f" ] && [ ! -L "$f" ] || continue
            grep -Fqx -- "$b" ${catalogFiles} || continue
            size=$(stat -c %s -- "$f")
            echo "<li><a href=\"/packs/$b\">$b</a> ($size bytes)</li>"
          done
          echo '</ul>'
          ;;
      esac
      echo '</body></html>'
    } > "$d/www/index.html"
    chmod 0644 "$d/www/index.html"

    conn=""
    ${lib.optionalString nm ''
      conn=$(${nmcli} -t -g UUID,DEVICE connection show --active 2>/dev/null \
        | grep -E ":$iface\$" | head -n 1 | cut -d: -f1 || true)
    ''}
    echo "$conn" > "$d/connection"
    echo "$iface" > "$d/interface"
    {
      echo "interface = \"$iface\""
      echo "name = \"$name\""
      echo "services = [ $list]"
      echo "connection = \"$conn\""
    } > "$d/state.toml.tmp"
    chmod 0644 "$d/state.toml.tmp"
    mv "$d/state.toml.tmp" "$d/state.toml"

    ${openPort} "$iface"
    echo "sharing $services on $iface as \"$name\""
  '';

  stop = pkgs.writeShellScript "resilience-share-stop" ''
    set -u
    export PATH=${path}
    for t in iptables ip6tables; do
      $t -w -F resilience-share 2>/dev/null
    done
    d=${stateDir}
    rm -rf "$d/conf" "$d/www"
    rm -f "$d/library" "$d/state.toml" "$d/state.toml.tmp" "$d/services" "$d/name" \
      "$d/connection" "$d/interface"
    echo "sharing stopped on $1"
    exit 0
  '';

  # NetworkManager: the shared interface's connection went down, or
  # another connection took the interface: stop sharing there.
  dispatcher = pkgs.writeShellScript "resilience-share-dispatcher" ''
    export PATH=${path}
    iface=''${1:-}
    action=''${2:-}
    d=${stateDir}
    [ -f "$d/interface" ] || exit 0
    [ "$(cat "$d/interface")" = "$iface" ] || exit 0
    stop=no
    case "$action" in
      down|pre-down) stop=yes ;;
      up)
        rec=$(cat "$d/connection" 2>/dev/null || true)
        if [ -n "$rec" ] && [ "$rec" != "''${CONNECTION_UUID:-}" ]; then stop=yes; fi
        ;;
    esac
    if [ "$stop" = yes ]; then
      logger -t resilience-share "network on $iface changed ($action): sharing stopped"
      systemctl --no-block stop "resilience-share@$iface.service"
    fi
    exit 0
  '';

  libraryStart = pkgs.writeShellScript "resilience-share-library" ''
    set -eu
    export PATH=${path}
    set --
    for f in ${packsCfg.dir}/*.zim; do
      b=$(basename "$f")
      [ -f "$f" ] && [ ! -L "$f" ] || continue
      grep -Fqx -- "$b" ${catalogFiles} || continue
      set -- "$@" "$f"
    done
    ${lib.concatMapStrings (f: "set -- \"$@\" ${f}\n") packsCfg.builtinZimFiles}
    if [ $# -eq 0 ]; then
      echo "no pack to serve in the library" >&2
      exit 1
    fi
    exec ${pkgs.kiwix-tools}/bin/kiwix-serve --address 127.0.0.1 \
      --port ${toString libraryPort} --urlRootLocation /library --blockexternal \
      --ipConnectionLimit 16 --threads 2 --skipInvalid "$@"
  '';

  hardening = {
    DynamicUser = true;
    AmbientCapabilities = [ "CAP_NET_BIND_SERVICE" ];
    CapabilityBoundingSet = [ "CAP_NET_BIND_SERVICE" ];
    NoNewPrivileges = true;
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
    ProtectProc = "invisible";
    ProcSubset = "pid";
    RestrictAddressFamilies = [
      "AF_INET"
      "AF_INET6"
      "AF_UNIX"
    ];
    RestrictNamespaces = true;
    RestrictRealtime = true;
    RestrictSUIDSGID = true;
    LockPersonality = true;
    MemoryDenyWriteExecute = true;
    SystemCallArchitectures = "native";
    SystemCallFilter = [
      "@system-service"
      "~@privileged @resources"
    ];
    UMask = "0077";
    LimitNOFILE = 1024;
    TasksMax = 32;
    Restart = "on-failure";
    RestartSec = 2;
  };

  autoStartModule = {
    options = {
      interface = mkOption {
        type = types.strMatching "[A-Za-z][A-Za-z0-9_.]{0,14}";
        description = "The one network interface to share on (e.g. `wlan0`, `bat0`).";
      };
      services = mkOption {
        type = types.nonEmptyListOf (types.enum services);
        description = "What to share.";
      };
      name = mkOption {
        type = types.strMatching "[A-Za-z0-9]([A-Za-z0-9 -]{0,30}[A-Za-z0-9])?";
        description = "The name other machines see.";
      };
    };
  };
in
{
  options.resilience.share = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Offer sharing offline resources with nearby machines (ADR 0025):
        packs, the library, a shared folder and web pages, read-only, over
        HTTP on one network interface. Nothing is shared until an
        administrator turns it on (`resilience-admin set-share`, with a
        password) or `autoStart` is set. Everything shared can be read by
        anyone on that network.
      '';
    };
    autoStart = mkOption {
      type = types.nullOr (types.submodule autoStartModule);
      default = null;
      example = {
        interface = "wlan0";
        services = [
          "library"
          "packs"
        ];
        name = "Shelter library";
      };
      description = ''
        Share at every boot, for a computer left on in a shelter. The
        interface must exist at boot; sharing stops if its network
        changes, as when it is turned on by hand.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.tmpfiles.rules = [
      "d ${stateDir} 0755 root root -"
      "d ${sharedDir} 0755 root root -"
      "d ${sharedDir}/files 2775 root wheel -"
      "d ${sharedDir}/pages 2775 root wheel -"
    ];

    # TCP 80 is opened by the share unit on one interface only; the chain
    # is empty otherwise. A firewall reload flushes it; the share unit's
    # reload (propagated) fills it again.
    networking.firewall.extraCommands = ''
      ip46tables -N resilience-share 2>/dev/null || true
      ip46tables -F resilience-share
      ip46tables -D nixos-fw -j resilience-share 2>/dev/null || true
      ip46tables -I nixos-fw -j resilience-share
    '';
    networking.firewall.extraStopCommands = ''
      ip46tables -D nixos-fw -j resilience-share 2>/dev/null || true
      ip46tables -F resilience-share 2>/dev/null || true
    '';

    networking.networkmanager.dispatcherScripts = lib.mkIf nm [
      {
        source = dispatcher;
        type = "basic";
      }
    ];

    systemd.services."resilience-share@" = {
      description = "Sharing offline resources on %i (Resilience, ADR 0025)";
      after = [
        "network.target"
        "firewall.service"
        "sys-subsystem-net-devices-%i.device"
      ];
      bindsTo = [ "sys-subsystem-net-devices-%i.device" ];
      partOf = [ "firewall.service" ];
      unitConfig.ReloadPropagatedFrom = [ "firewall.service" ];
      wants = [
        "resilience-share-web@%i.service"
        "resilience-share-library@%i.service"
      ]
      ++ lib.optional comms.enable "resilience-share-announce@%i.service";
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${start} %i";
        ExecReload = "${openPort} %i";
        ExecStopPost = "${stop} %i";
        TimeoutSec = 30;
      };
    };

    systemd.services."resilience-share-web@" = {
      description = "Web server for sharing on %i (Resilience, ADR 0025)";
      after = [ "resilience-share@%i.service" ];
      bindsTo = [ "resilience-share@%i.service" ];
      serviceConfig = hardening // {
        ExecStart = "${pkgs.nginx}/bin/nginx -c ${nginxConf} -p /run/resilience-share-web -e stderr";
        ExecReload = "${pkgs.coreutils}/bin/kill -HUP $MAINPID";
        RuntimeDirectory = "resilience-share-web";
        RuntimeDirectoryMode = "0700";
        MemoryMax = "128M";
      };
    };

    systemd.services."resilience-share-library@" = {
      description = "Library for sharing on %i (kiwix-serve, Resilience, ADR 0025)";
      after = [ "resilience-share@%i.service" ];
      before = [ "resilience-share-web@%i.service" ];
      bindsTo = [ "resilience-share@%i.service" ];
      unitConfig.ConditionPathExists = "${stateDir}/library";
      serviceConfig = hardening // {
        ExecStart = libraryStart;
        # kiwix-serve checks its address with getifaddrs (netlink).
        RestrictAddressFamilies = hardening.RestrictAddressFamilies ++ [ "AF_NETLINK" ];
        MemoryMax = "512M";
      };
    };

    # Discovery (ADR 0025 §4): while sharing, announce it on the machine's
    # Reticulum instance. Talks only to the instance's local socket; the
    # IPv4 socket is for asking the kernel the interface's address
    # (SIOCGIFADDR), and IPAddressDeny keeps it off the network.
    systemd.services."resilience-share-announce@" = lib.mkIf comms.enable {
      description = "Announces sharing on %i to nearby machines (Reticulum, ADR 0025)";
      after = [
        "resilience-share@%i.service"
        "resilience-reticulum.service"
      ];
      bindsTo = [ "resilience-share@%i.service" ];
      serviceConfig = {
        ExecStart = lib.concatStringsSep " " [
          "${comms.messagesPackage}/bin/resilience-messages share"
          "--dir /var/lib/resilience-share-announce"
          "--client-config ${comms.clientConfig}"
          "--state ${stateDir}/state.toml"
        ];
        DynamicUser = true;
        StateDirectory = "resilience-share-announce";
        StateDirectoryMode = "0700";
        UMask = "0077";
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
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_INET"
        ];
        IPAddressDeny = "any";
        MemoryMax = "128M";
        Restart = "on-failure";
        RestartSec = 10;
      };
    };

    systemd.services."resilience-share@${
      if cfg.autoStart != null then cfg.autoStart.interface else "none"
    }" =
      lib.mkIf (cfg.autoStart != null) {
        overrideStrategy = "asDropin";
        wantedBy = [ "multi-user.target" ];
      };
  };
}
