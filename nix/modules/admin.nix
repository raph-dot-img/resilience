# The privileged helper `resilience-admin` (ADR 0007).
#
# * One typed operation per polkit action; no generic command runner.
#   `set-mode` and `set-time` (ADR 0009) require an administrator's password
#   every time (auth_admin, never auth_admin_keep), whoever asks.
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
      inherit helper;
      pkexec = "${config.security.wrapperDir}/pkexec";
      pkttyagent = "${lib.getBin config.security.polkit.package}/bin/pkttyagent";
      session_shell = "/run/current-system/sw${cfg.sessionPackage.shellPath}";
      users = lib.mapAttrs (_: u: {
        declared_mode = u.mode;
        inherit (u) description;
      }) cfg.users;
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
    </policyconfig>
  '';
in
{
  config = {
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
