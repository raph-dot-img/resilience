{
  config,
  lib,
  pkgs,
  ...
}:

lib.mkIf config.resilience.applications.entertainment.enable {
  resilience.apps = {
    nudoku = {
      name = "Sudoku";
      category = "entertainment";
      description = "Classic number puzzle. Arrow keys move, digits fill, q quits.";
      package = pkgs.nudoku;
      command = [ (lib.getExe pkgs.nudoku) ];
      keywords = [
        "puzzle"
        "game"
      ];
    };
    solitaire = {
      name = "Solitaire";
      category = "entertainment";
      description = "Klondike patience card game. Press q to quit.";
      package = pkgs.tty-solitaire;
      command = [ (lib.getExe pkgs.tty-solitaire) ];
      keywords = [
        "cards"
        "game"
        "klondike"
      ];
    };
    ninvaders = {
      name = "Space invaders";
      category = "entertainment";
      description = "Arcade shooter. Arrow keys move, space fires, q quits.";
      package = pkgs.ninvaders;
      command = [ (lib.getExe pkgs.ninvaders) ];
      keywords = [
        "arcade"
        "game"
      ];
    };
    moon-buggy = {
      name = "Moon buggy";
      category = "entertainment";
      description = "Drive across the moon and jump over craters with space. q quits.";
      package = pkgs.moon-buggy;
      command = [ (lib.getExe pkgs.moon-buggy) ];
      keywords = [
        "arcade"
        "game"
      ];
    };
    nethack = {
      name = "NetHack";
      category = "entertainment";
      description = "Deep dungeon-exploration roguelike. Press ? in the game for help; S saves and quits.";
      package = pkgs.nethack;
      command = [ (lib.getExe pkgs.nethack) ];
      keywords = [
        "roguelike"
        "dungeon"
        "game"
      ];
    };
  };
}
