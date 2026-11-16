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
  kb = config.resilience.keyboard;
  layout = kb.layout;
  # Every name offered or used, checked against kbd at build time.
  keymapNames = lib.unique ([ layout ] ++ map (l: l.id) kb.layouts);
  keymapId = lib.types.strMatching "[A-Za-z0-9][A-Za-z0-9_-]{0,39}";
in
{
  options.resilience.keyboard = {
    layout = lib.mkOption {
      type = keymapId;
      default = "us";
      example = "fr";
      description = ''
        The system's console keyboard layout (a kbd keymap name: "us",
        "fr" for French AZERTY, "de-latin1", ...). Used at boot, on every
        login prompt (it is loaded again before each one) and by accounts
        that have not chosen another layout. The build fails if the keymap
        does not exist.
      '';
    };
    layouts = lib.mkOption {
      type = lib.types.listOf (
        lib.types.submodule {
          options = {
            id = lib.mkOption {
              type = keymapId;
              description = "kbd keymap name.";
            };
            label = lib.mkOption {
              type = lib.types.strMatching "[ -~]{1,60}";
              description = "Name shown in Settings (printable ASCII).";
            };
          };
        }
      );
      default = [
        {
          id = "us";
          label = "English (US, QWERTY)";
        }
        {
          id = "uk";
          label = "English (UK, QWERTY)";
        }
        {
          id = "fr";
          label = "French (AZERTY)";
        }
        {
          id = "fr-bepo";
          label = "French (BEPO)";
        }
        {
          id = "be-latin1";
          label = "Belgian (AZERTY)";
        }
        {
          id = "fr_CH";
          label = "Swiss French (QWERTZ)";
        }
        {
          id = "de-latin1";
          label = "German (QWERTZ)";
        }
        {
          id = "de_CH-latin1";
          label = "Swiss German (QWERTZ)";
        }
        {
          id = "es";
          label = "Spanish (QWERTY)";
        }
        {
          id = "it";
          label = "Italian (QWERTY)";
        }
        {
          id = "pt-latin1";
          label = "Portuguese (QWERTY)";
        }
        {
          id = "nl";
          label = "Dutch (QWERTY)";
        }
        {
          id = "dvorak";
          label = "English (Dvorak)";
        }
      ];
      description = ''
        Layouts each account may choose in Settings. The choice is loaded
        with loadkeys (as the user, on their own console) at every login
        and when changed; the console has one keymap, so it applies to
        every console until someone logs out. The system's layout should
        be in the list.
      '';
    };
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

    # A misspelt keymap would only fail at boot (systemd-vconsole-setup)
    # or in Settings: refuse it at build time instead.
    system.checks = [
      (pkgs.runCommand "resilience-keymaps" { } ''
        for m in ${lib.escapeShellArgs keymapNames}; do
          if [ -z "$(find ${pkgs.kbd}/share/keymaps -name "$m.map.gz" -print -quit)" ]; then
            echo "resilience.keyboard: no console keymap named '$m' in kbd" >&2
            exit 1
          fi
        done
        touch $out
      '')
    ];

    # The console keymap is shared by all consoles and survives a logout.
    # Load the system's layout again before every login prompt, so nobody
    # types a password (and trips faillock) with another account's layout.
    systemd.services."getty@".serviceConfig.ExecStartPre = [
      "-${pkgs.kbd}/bin/loadkeys -q -- ${layout}"
    ];

    # No graphical stack, ever.
    services.xserver.enable = false;
    fonts.fontconfig.enable = lib.mkDefault false;
    xdg.icons.enable = false;
    xdg.mime.enable = false;
    xdg.sounds.enable = false;
  };
}
