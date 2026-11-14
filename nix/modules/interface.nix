# Text-only console session: branding on the login prompt, console font,
# optional autologin into the TUI on tty1. Other ttys keep normal login
# prompts so recovery stays possible if the interface fails.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.resilience.interface;
in
{
  options.resilience.interface = {
    autologinUser = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        Account logged in automatically on tty1. Only for kiosk-like or live
        systems; the account must be an Ultra-Stable Resilience user.
      '';
    };
  };

  config = {
    assertions = [
      {
        assertion =
          cfg.autologinUser == null
          || (
            config.resilience.users ? ${cfg.autologinUser}
            && config.resilience.users.${cfg.autologinUser}.mode == "ultra-stable"
          );
        message = "resilience.interface.autologinUser must be an ultra-stable resilience.users account.";
      }
    ];

    services.getty = {
      autologinUser = cfg.autologinUser;
      greetingLine = ''
        \e[1;32m
         █▀█ █▀▀ █▀ █ █   █ █▀▀ █▄ █ █▀▀ █▀▀
         █▀▄ ██▄ ▄█ █ █▄▄ █ ██▄ █ ▀█ █▄▄ ██▄  OS
        \e[0m
         Resilience OS ${config.resilience.package.version} (${config.resilience.edition}) - \l'';
      helpLine = lib.mkForce ''
        Log in to start the Resilience interface. Alt+F1 ... Alt+F6 switch consoles.
      '';
    };

    console = {
      earlySetup = true;
      # ter-i16n: 256 glyphs (keeps 16 colours on the VT), CP437 box/block
      # characters used by the TUI, and most accented Latin letters.
      # (ter-v16n lacks ▀ ▄ and double-line boxes; verified from its PSF table.)
      font = "ter-i16n";
      packages = [ pkgs.terminus_font ];
      keyMap = lib.mkDefault "us";
    };

    # No graphical stack, ever.
    services.xserver.enable = false;
    fonts.fontconfig.enable = lib.mkDefault false;
    xdg.icons.enable = false;
    xdg.mime.enable = false;
    xdg.sounds.enable = false;
  };
}
