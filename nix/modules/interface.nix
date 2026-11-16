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
  layout = config.resilience.keyboard.layout;
in
{
  options.resilience.keyboard.layout = lib.mkOption {
    type = lib.types.strMatching "[A-Za-z0-9_-]+";
    default = "fr";
    example = "us";
    description = ''
      Console keyboard layout (a kbd keymap name: "fr" for French AZERTY,
      "us", "de", "be-latin1", "fr_CH", "fr-bepo", ...). Applies to every
      console, the login prompts and the interface. Change it in the
      system configuration and rebuild; the build fails if the keymap does
      not exist.
    '';
  };

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

    # services.getty.autologinUser would autologin on EVERY getty (tty1-6
    # and serial consoles), removing the recovery login prompts. Instead,
    # tty1 alone gets a drop-in that adds --autologin; the arguments mirror
    # nixos/modules/services/ttys/getty.nix.
    systemd.services."getty@tty1" = lib.mkIf (cfg.autologinUser != null) {
      overrideStrategy = "asDropin";
      serviceConfig.ExecStart = [
        ""
        (lib.concatStringsSep " " [
          (lib.getExe' pkgs.util-linux "agetty")
          "--login-program ${config.services.getty.loginProgram}"
          "--issue-file /etc/issue:/etc/issue.d:/run/issue:/run/issue.d"
          "--autologin ${lib.escapeShellArg cfg.autologinUser}"
          "--noclear --keep-baud tty1 115200,38400,9600 $TERM"
        ])
      ];
    };

    services.getty = {
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
      keyMap = layout;
    };

    # A misspelt keymap would only fail at boot (systemd-vconsole-setup),
    # leaving the console in US layout: refuse it at build time instead.
    system.checks = [
      (pkgs.runCommand "resilience-keymap-${layout}" { } ''
        if [ -z "$(find ${pkgs.kbd}/share/keymaps -name '${layout}.map.gz' -print -quit)" ]; then
          echo "resilience.keyboard.layout: no console keymap named '${layout}' in kbd" >&2
          exit 1
        fi
        touch $out
      '')
    ];

    # No graphical stack, ever.
    services.xserver.enable = false;
    fonts.fontconfig.enable = lib.mkDefault false;
    xdg.icons.enable = false;
    xdg.mime.enable = false;
    xdg.sounds.enable = false;
  };
}
