# Resource packs: large offline content chosen by the user (ADR 0013).
#
# * The catalog (default: nix/packs/kiwix.nix) pins every file's size and
#   SHA-256. It is generated into the root-owned /etc/resilience/packs.toml,
#   which is the only thing the store trusts.
# * Installed packs live in `dir`, owned by the unprivileged system account
#   `resilience-packs`. Only `resilience-pack-store` writes there, started by
#   `pkexec --user resilience-packs` after an administrator's password
#   (polkit action org.resilience.packs.change, auth_admin, never _keep).
#   It never runs as root and never parses the content.
# * Reading: kiwix-serve on 127.0.0.1 for one file at a time, and lynx with
#   every restriction on (no shell escape, no saving, no other address).
#   Map packs (.mbtiles, category maps, ADR 0016) are never given to
#   kiwix-serve; they are opened from Maps & navigation.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  inherit (lib) mkOption types;
  cfg = config.resilience.packs;
  toml = pkgs.formats.toml { };
  store = "${config.resilience.package}/bin/resilience-pack-store";
  library = "${config.resilience.package}/bin/resilience-library";

  packModule = {
    options = {
      id = mkOption {
        type = types.strMatching "[a-z0-9][a-z0-9-]{0,63}";
        description = "Stable identifier across versions.";
      };
      title = mkOption { type = types.str; };
      description = mkOption { type = types.str; };
      category = mkOption {
        type = types.enum [
          "encyclopedia"
          "dictionary"
          "health"
          "practical"
          "travel"
          "maps"
          "computing"
          "documentation"
        ];
      };
      language = mkOption {
        type = types.strMatching "[a-z]{2,3}";
        description = "ISO 639 code of the content's language.";
      };
      file = mkOption {
        type = types.strMatching "[A-Za-z0-9][A-Za-z0-9._-]*\\.(zim|mbtiles)";
        description = ''
          File name, as published (e.g. on download.kiwix.org). A `.zim` file is
          read in the text browser; an `.mbtiles` file is a map pack (ADR 0016,
          `map-osm-REGION_YYYY-MM.mbtiles`) and belongs in the `maps` category.
        '';
      };
      size = mkOption {
        type = types.ints.positive;
        description = "Exact size in bytes.";
      };
      sha256 = mkOption {
        type = types.strMatching "[0-9a-f]{64}";
        description = "SHA-256 of the file. Only a file with this hash is installed.";
      };
      urls = mkOption {
        type = types.listOf (types.strMatching "https://[^[:space:]]+");
        default = [ ];
        description = "Download addresses, tried in order. Empty: USB sticks only.";
      };
      license = mkOption { type = types.str; };
      source = mkOption { type = types.str; };
      notice = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "Shown before the content opens (e.g. medical packs).";
      };
      data_date = mkOption {
        type = types.nullOr (types.strMatching "[0-9]{4}-[0-9]{2}(-[0-9]{2})?");
        default = null;
        description = "Date of the data (YYYY-MM-DD or YYYY-MM). Required for map packs.";
      };
      coverage = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = ''
          What the pack covers and what it leaves out, in plain words; shown in
          full in the pack details. Required for map packs, which must never
          look complete when the data is partial. At most 1000 characters.
        '';
      };
      attribution = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = ''
          The credit the license requires, e.g. "© OpenStreetMap contributors".
          Required for map packs. At most 300 characters.
        '';
      };
    };
  };

  # Lynx settings beyond its defaults; restrictions are on the command line.
  lynxCfg = pkgs.writeText "resilience-lynx.cfg" ''
    INCLUDE:${pkgs.lynx}/etc/lynx.cfg
    CHARACTER_SET:utf-8
    SET_COOKIES:FALSE
    ACCEPT_ALL_COOKIES:FALSE
    NO_REFERER_HEADER:TRUE
    MAKE_LINKS_FOR_ALL_IMAGES:FALSE
    SHOW_CURSOR:TRUE
  '';

  lynxRestrictions = [
    "bookmark"
    "bookmark_exec"
    "change_exec_perms"
    "chdir"
    "dired_support"
    "disk_save"
    "dotfiles"
    "download"
    "editor"
    "exec"
    "exec_frozen"
    "file_url"
    "goto"
    "jump"
    "lynxcfg_xinfo"
    "lynxcgi"
    "mail"
    "multibook"
    "news_post"
    "option_save"
    "print"
    "shell"
    "suspend"
    "useragent"
    "inside_ftp"
    "outside_ftp"
    "inside_news"
    "outside_news"
    "inside_rlogin"
    "outside_rlogin"
    "inside_telnet"
    "outside_telnet"
    "telnet_port"
  ];

  packsConfigUnchecked = toml.generate "resilience-packs.toml" {
    dir = cfg.dir;
    store_user = "resilience-packs";
    inherit store;
    pkexec = "${config.security.wrapperDir}/pkexec";
    pkttyagent = "${lib.getBin config.security.polkit.package}/bin/pkttyagent";
    curl = "${lib.getBin pkgs.curl}/bin/curl";
    sha256sum = "${pkgs.coreutils}/bin/sha256sum";
    kiwix_serve = "${pkgs.kiwix-tools}/bin/kiwix-serve";
    browser = [
      (lib.getExe pkgs.lynx)
      "-cfg=${lynxCfg}"
      # Every restriction except those that only concern old protocols;
      # not `all`, which also refuses the server's redirects.
      "-restrictions=${lib.concatStringsSep "," lynxRestrictions}"
      "-localhost"
      "-noreferer"
      "-display_charset=utf-8"
    ];
    session_shell = "/run/current-system/sw${config.resilience.sessionPackage.shellPath}";
    pack = map (p: lib.filterAttrs (_: v: v != null) p) cfg.catalog;
    set = cfg.sets;
    builtin =
      lib.optional cfg.builtinWorldMap {
        title = "World overview map (built in)";
        description = "The whole world at overview detail: coasts, borders, large lakes and rivers, about 7,300 towns. No streets. Natural Earth ${worldMap.version}.";
        category = "maps";
        language = "en";
        path = "${worldMap}/${worldMap.path}";
        license = "Public domain";
        source = "Natural Earth ${worldMap.version} (naturalearthdata.com)";
      }
      ++ lib.optional cfg.builtinDocs {
        title = "System documentation (built in)";
        description = "The Resilience handbook, the NixOS manual and the NixOS options of this NixOS release.";
        category = "documentation";
        language = "en";
        path = "${docsPack}/share/resilience/packs/resilience-system-docs.zim";
        license = "MIT";
        source = "Resilience handbook; NixOS manual and options of the pinned nixpkgs (nixos.org)";
      };
  };

  worldMap = pkgs.callPackage ../packages/resilience-world-map {
    resilience = config.resilience.package;
  };

  docsPack = pkgs.callPackage ../packages/resilience-docs {
    handbook = config.resilience.handbookPackage;
  };

  # The same parser as the store checks the file at build time.
  packsConfig = pkgs.runCommand "resilience-packs-checked.toml" { } ''
    ${library} check-config ${packsConfigUnchecked}
    cp ${packsConfigUnchecked} $out
  '';

  policy = pkgs.writeTextDir "share/polkit-1/actions/org.resilience.packs.policy" ''
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE policyconfig PUBLIC "-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN"
      "http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd">
    <policyconfig>
      <vendor>Resilience OS</vendor>
      <action id="org.resilience.packs.change">
        <description>Install or remove resource packs</description>
        <message>Installing or removing resource packs requires an administrator password. Request: $(command_line)</message>
        <defaults>
          <allow_any>auth_admin</allow_any>
          <allow_inactive>auth_admin</allow_inactive>
          <allow_active>auth_admin</allow_active>
        </defaults>
        <annotate key="org.freedesktop.policykit.exec.path">${store}</annotate>
      </action>
    </policyconfig>
  '';
in
{
  options.resilience.packs = {
    enable = mkOption {
      type = types.bool;
      default = true;
      description = "Offer resource packs (offline library) in the interface.";
    };
    dir = mkOption {
      type = types.strMatching "/[^[:space:]]+";
      default = "/var/lib/resilience/packs";
      description = "Where installed packs are kept, shared by every account.";
    };
    sets = mkOption {
      type = types.listOf (
        types.submodule {
          options = {
            id = mkOption { type = types.strMatching "[a-z0-9][a-z0-9-]{0,63}"; };
            title = mkOption { type = types.str; };
            description = mkOption { type = types.str; };
            packs = mkOption {
              type = types.listOf types.str;
              description = "Catalog pack ids.";
            };
          };
        }
      );
      # Only packs present in the catalog (it may be replaced); empty sets dropped.
      default =
        let
          ids = map (p: p.id) cfg.catalog;
        in
        lib.filter (s: s.packs != [ ]) (
          map (s: s // { packs = lib.filter (id: lib.elem id ids) s.packs; }) (
            import ../packs/sets.nix cfg.catalog
          )
        );
      defaultText = lib.literalExpression "import ./nix/packs/sets.nix config.resilience.packs.catalog";
      description = "Suggested selections offered on the Resource packs screen.";
    };
    builtinDocs = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Ship a small documentation pack (Resilience handbook, NixOS manual and
        options, about 4 MiB) in the system, readable in the Offline library.
      '';
    };
    builtinWorldMap = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Ship the world overview map (Natural Earth, zooms 0 to 7, about
        12 MiB) in the system: maps then work without any map pack
        (ADR 0016).
      '';
    };
    configFile = mkOption {
      type = types.package;
      readOnly = true;
      internal = true;
      description = "The generated and checked /etc/resilience/packs.toml.";
    };
    catalog = mkOption {
      type = types.listOf (types.submodule packModule);
      default = import ../packs/kiwix.nix;
      defaultText = lib.literalExpression "import ./nix/packs/kiwix.nix";
      description = ''
        Packs offered for download or installation from a USB stick. Only a
        file with exactly this size and SHA-256 is installed.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    resilience.packs.configFile = packsConfig;

    assertions = [
      {
        assertion = lib.allUnique (map (p: p.id) cfg.catalog);
        message = "resilience.packs.catalog: pack ids must be unique";
      }
      {
        assertion = lib.allUnique (map (p: p.file) cfg.catalog);
        message = "resilience.packs.catalog: pack files must be unique";
      }
      {
        assertion = lib.all (s: lib.all (id: lib.any (p: p.id == id) cfg.catalog) s.packs) cfg.sets;
        message = "resilience.packs.sets: every pack id must be in the catalog";
      }
    ];

    users.users.resilience-packs = {
      isSystemUser = true;
      group = "resilience-packs";
      description = "Resilience resource packs";
      home = "/var/empty";
    };
    users.groups.resilience-packs = { };

    systemd.tmpfiles.rules = [
      "d ${cfg.dir} 0755 resilience-packs resilience-packs -"
      "d ${cfg.dir}/.staging 0755 resilience-packs resilience-packs -"
    ];

    environment.etc."resilience/packs.toml".source = packsConfig;
    environment.systemPackages = [ policy ];
  };
}
