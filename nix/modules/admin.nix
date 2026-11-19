# The privileged helper `resilience-admin` (ADR 0007).
#
# * One typed operation per polkit action; no generic command runner.
#   `set-mode`, `set-time` (ADR 0009) and `set-default-generation` (ADR 0011)
#   require an administrator's password every time (auth_admin, never
#   auth_admin_keep), whoever asks. The exceptions are `set-keymap`
#   (ADR 0017: one of the listed console keyboard layouts), `set-mesh`
#   (ADR 0021: the Wi-Fi mesh on or off) and `record-licence` (ADR 0024),
#   allowed without a password to the active local session only.
#   `set-share` and `set-hotspot` (ADR 0025) are auth_admin, on and off.
# * Configuration the helper trusts is generated here and root-owned
#   (/etc/resilience/admin.toml); it never reads the caller's environment.
# * Mode changes persist in /var/lib/resilience/modes.toml and are
#   re-applied after every system activation (boot or switch), right after
#   NixOS has reset group membership to the declared configuration.
# * Every privileged operation is appended to /var/log/resilience/audit.log
#   (root:wheel 0640) and sent to the journal (authpriv).
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.resilience;
  toml = pkgs.formats.toml { };
  helper = "${cfg.package}/bin/resilience-admin";

  adminConfig = toml.generate "resilience-admin.toml" (
    {
      state_file = "/var/lib/resilience/modes.toml";
      audit_log = "/var/log/resilience/audit.log";
      lock_file = "/run/resilience-admin.lock";
      gpasswd = "${pkgs.shadow}/bin/gpasswd";
      loginctl = "${config.systemd.package}/bin/loginctl";
      timedatectl = "${config.systemd.package}/bin/timedatectl";
      nix_env = "${config.nix.package}/bin/nix-env";
      inherit helper;
      pkexec = "${config.security.wrapperDir}/pkexec";
      pkttyagent = "${lib.getBin config.security.polkit.package}/bin/pkttyagent";
      session_shell = "/run/current-system/sw${cfg.sessionPackage.shellPath}";
      # ADR 0012: no Nix sandbox, so no account may be raised at runtime.
      ultra_stable_only = !cfg.security.userNamespaces;
      # ADR 0017: the only layouts `set-keymap` loads.
      loadkeys = "${pkgs.kbd}/bin/loadkeys";
      keymaps = lib.unique ([ cfg.keyboard.layout ] ++ map (l: l.id) cfg.keyboard.layouts);
      # ADR 0021: `set-mesh` starts and stops this unit only.
      systemctl = "${config.systemd.package}/bin/systemctl";
      users = lib.mapAttrs (_: u: {
        declared_mode = u.mode;
        inherit (u) description;
      }) cfg.users;
    }
    // lib.optionalAttrs cfg.localAccounts {
      local_accounts_file = "/var/lib/resilience/accounts.toml";
    }
    // lib.optionalAttrs cfg.mesh.enable {
      mesh_unit = "resilience-mesh.service";
    }
    // lib.optionalAttrs cfg.share.enable {
      # ADR 0025: `set-share` starts and stops these template units only.
      share = {
        unit = "resilience-share";
        services = [
          "packs"
          "library"
          "files"
          "pages"
        ];
        state_dir = "/run/resilience-share";
      };
    }
    // lib.optionalAttrs cfg.share.hotspot.enable {
      # ADR 0025 §6: `set-hotspot` starts and stops this unit only.
      hotspot = {
        unit = "resilience-hotspot.service";
        state_dir = "/run/resilience-hotspot";
        inherit (cfg.share.hotspot) ssid;
        mesh_devices = "/run/resilience-mesh/devices";
      };
    }
    // lib.optionalAttrs (cfg.updates.settings != null) {
      # ADR 0023: `apply-update`.
      updates = cfg.updates.settings;
    }
    // {
      # ADR 0024: `record-licence`.
      licence = cfg.licence.settings;
    }
    // lib.optionalAttrs (cfg.interface.autologinUser != null) {
      autologin_user = cfg.interface.autologinUser;
    }
  );

  # pkexec picks the action whose exec.path (canonicalised) and exec.argv1
  # match the command; anything else falls back to the generic pkexec action.
  policy = pkgs.writeTextDir "share/polkit-1/actions/org.resilience.admin.policy" ''
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE policyconfig PUBLIC "-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN"
      "http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd">
    <policyconfig>
      <vendor>Resilience OS</vendor>
      <action id="org.resilience.admin.set-mode">
        <description>Change the security mode of an account</description>
        <message>Changing a security mode requires an administrator password</message>
        <defaults>
          <allow_any>auth_admin</allow_any>
          <allow_inactive>auth_admin</allow_inactive>
          <allow_active>auth_admin</allow_active>
        </defaults>
        <annotate key="org.freedesktop.policykit.exec.path">${helper}</annotate>
        <annotate key="org.freedesktop.policykit.exec.argv1">set-mode</annotate>
      </action>
      <action id="org.resilience.admin.set-time">
        <description>Set the system clock</description>
        <message>Setting the clock requires an administrator password</message>
        <defaults>
          <allow_any>auth_admin</allow_any>
          <allow_inactive>auth_admin</allow_inactive>
          <allow_active>auth_admin</allow_active>
        </defaults>
        <annotate key="org.freedesktop.policykit.exec.path">${helper}</annotate>
        <annotate key="org.freedesktop.policykit.exec.argv1">set-time</annotate>
      </action>
      <action id="org.resilience.admin.set-default-generation">
        <description>Choose the system generation started by default</description>
        <message>Changing the default system generation requires an administrator password</message>
        <defaults>
          <allow_any>auth_admin</allow_any>
          <allow_inactive>auth_admin</allow_inactive>
          <allow_active>auth_admin</allow_active>
        </defaults>
        <annotate key="org.freedesktop.policykit.exec.path">${helper}</annotate>
        <annotate key="org.freedesktop.policykit.exec.argv1">set-default-generation</annotate>
      </action>
      ${lib.optionalString cfg.share.enable ''
        <action id="org.resilience.admin.set-share">
          <description>Share offline resources with the computers on a network, or stop</description>
          <message>Sharing with a network, or stopping it, requires an administrator password. Everything shared can be read by anyone on that network. Request: $(command_line)</message>
          <defaults>
            <allow_any>auth_admin</allow_any>
            <allow_inactive>auth_admin</allow_inactive>
            <allow_active>auth_admin</allow_active>
          </defaults>
          <annotate key="org.freedesktop.policykit.exec.path">${helper}</annotate>
          <annotate key="org.freedesktop.policykit.exec.argv1">set-share</annotate>
        </action>
      ''}
      ${lib.optionalString cfg.share.hotspot.enable ''
        <action id="org.resilience.admin.set-hotspot">
          <description>Turn this computer into a Wi-Fi network for others, or stop</description>
          <message>Turning the hotspot on or off requires an administrator password</message>
          <defaults>
            <allow_any>auth_admin</allow_any>
            <allow_inactive>auth_admin</allow_inactive>
            <allow_active>auth_admin</allow_active>
          </defaults>
          <annotate key="org.freedesktop.policykit.exec.path">${helper}</annotate>
          <annotate key="org.freedesktop.policykit.exec.argv1">set-hotspot</annotate>
        </action>
      ''}
      ${lib.optionalString (cfg.updates.settings != null) ''
        <action id="org.resilience.admin.apply-update">
          <description>Install a Resilience update from removable media</description>
          <message>Installing an update requires an administrator password</message>
          <defaults>
            <allow_any>auth_admin</allow_any>
            <allow_inactive>auth_admin</allow_inactive>
            <allow_active>auth_admin</allow_active>
          </defaults>
          <annotate key="org.freedesktop.policykit.exec.path">${helper}</annotate>
          <annotate key="org.freedesktop.policykit.exec.argv1">apply-update</annotate>
        </action>
      ''}
      <action id="org.resilience.admin.record-licence">
        <description>Record that you read the licence of Resilience</description>
        <message>Recording that you read the licence is allowed to the person at the console</message>
        <defaults>
          <allow_any>no</allow_any>
          <allow_inactive>no</allow_inactive>
          <allow_active>yes</allow_active>
        </defaults>
        <annotate key="org.freedesktop.policykit.exec.path">${helper}</annotate>
        <annotate key="org.freedesktop.policykit.exec.argv1">record-licence</annotate>
      </action>
      <action id="org.resilience.admin.set-keymap">
        <description>Load a console keyboard layout of this system</description>
        <message>Loading a keyboard layout is allowed to the person at the console</message>
        <defaults>
          <allow_any>no</allow_any>
          <allow_inactive>no</allow_inactive>
          <allow_active>yes</allow_active>
        </defaults>
        <annotate key="org.freedesktop.policykit.exec.path">${helper}</annotate>
        <annotate key="org.freedesktop.policykit.exec.argv1">set-keymap</annotate>
      </action>
      <action id="org.resilience.admin.set-mesh">
        <description>Turn the Wi-Fi mesh network on or off</description>
        <message>Turning the mesh network on or off is allowed to the person at the console</message>
        <defaults>
          <allow_any>no</allow_any>
          <allow_inactive>no</allow_inactive>
          <allow_active>yes</allow_active>
        </defaults>
        <annotate key="org.freedesktop.policykit.exec.path">${helper}</annotate>
        <annotate key="org.freedesktop.policykit.exec.argv1">set-mesh</annotate>
      </action>
    </policyconfig>
  '';
in
{
  options.resilience.localAccounts = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = ''
      Installed systems (ADR 0022): accounts created on the machine and
      listed in /var/lib/resilience/accounts.toml are managed like declared
      ones, with Ultra-Stable as their base mode. Needs mutable users.
    '';
  };

  config = {
    assertions = [
      {
        assertion = !cfg.localAccounts || config.users.mutableUsers;
        message = "resilience.localAccounts needs users.mutableUsers = true.";
      }
    ];
    security.polkit.enable = true;
    # Administrators for polkit's auth_admin are exactly the Advanced accounts.
    security.polkit.adminIdentities = [ "unix-group:wheel" ];

    environment.etc."resilience/admin.toml".source = adminConfig;
    environment.systemPackages = [ policy ];

    systemd.tmpfiles.rules = [
      "d /var/lib/resilience 0755 root root -"
      "d /var/log/resilience 0750 root wheel -"
      "f /var/log/resilience/audit.log 0640 root wheel -"
    ];

    # Re-apply recorded mode changes after NixOS has reset group membership
    # (`users`) and installed the new /etc (`etc`). If this fails, the
    # declared modes stay in effect; the failure is reported on the console,
    # in the journal and in the audit log.
    system.activationScripts.resilienceModes = {
      deps = [
        "users"
        "groups"
        "etc"
      ];
      text = ''
        install -d -m 0755 -o root -g root /var/lib/resilience
        install -d -m 0750 -o root -g wheel /var/log/resilience
        [ -e /var/log/resilience/audit.log ] || install -m 0640 -o root -g wheel /dev/null /var/log/resilience/audit.log
        ${helper} apply-state || echo "resilience-admin apply-state failed; declared modes are in effect" >&2
      '';
    };
  };
}
