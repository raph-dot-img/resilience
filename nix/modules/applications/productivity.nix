{
  config,
  lib,
  pkgs,
  ...
}:

lib.mkIf config.resilience.applications.productivity.enable {
  resilience.apps = {
    nano = {
      name = "Nano text editor";
      category = "productivity";
      description = "A simple text editor. Keys are listed at the bottom; Ctrl-O saves, Ctrl-X quits.";
      package = pkgs.nano;
      command = [ (lib.getExe pkgs.nano) ];
      minMode = "stable";
      keywords = [
        "edit"
        "write"
        "text"
      ];
    };
    vim = {
      name = "Vim text editor";
      category = "productivity";
      description = "A powerful editor for experienced users. Type :q and Enter to quit.";
      package = pkgs.vim;
      command = [ (lib.getExe pkgs.vim) ];
      minMode = "stable";
      keywords = [
        "edit"
        "vi"
      ];
    };
    mc = {
      name = "Midnight Commander";
      category = "productivity";
      description = "A two-panel file manager: copy, move and view files. F10 quits.";
      package = pkgs.mc;
      command = [ (lib.getExe pkgs.mc) ];
      minMode = "stable";
      keywords = [
        "files"
        "folders"
        "copy"
        "move"
      ];
    };
    calculator = {
      name = "Calculator";
      category = "productivity";
      description = "Type a calculation such as 12.5*4 and press Enter. Type quit to leave.";
      package = pkgs.bc;
      command = [
        (lib.getExe pkgs.bc)
        "--mathlib"
        "--quiet"
      ];
      keywords = [
        "math"
        "calc"
        "bc"
      ];
    };
    units = {
      name = "Unit converter";
      category = "productivity";
      description = "Converts units: answer 'You have' (e.g. 10 miles) and 'You want' (e.g. km). Ctrl-D quits.";
      package = pkgs.units;
      command = [ (lib.getExe pkgs.units) ];
      keywords = [
        "convert"
        "metric"
        "imperial"
      ];
    };
    calendar = {
      name = "Calendar (3 months)";
      category = "productivity";
      description = "Shows the previous, current and next month.";
      command = [
        "${pkgs.util-linux}/bin/cal"
        "-3"
      ];
      pauseAfterExit = true;
      keywords = [
        "date"
        "month"
        "cal"
      ];
    };
    calcurse = {
      name = "Calcurse planner";
      category = "productivity";
      description = "Calendar with appointments and a to-do list. Press q to quit.";
      package = pkgs.calcurse;
      command = [ "${pkgs.calcurse}/bin/calcurse" ];
      keywords = [
        "agenda"
        "todo"
        "appointments"
      ];
    };
  };
}
