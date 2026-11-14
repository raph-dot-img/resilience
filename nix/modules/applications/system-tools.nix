{
  config,
  lib,
  pkgs,
  ...
}:

lib.mkIf config.resilience.applications.systemTools.enable {
  resilience.apps = {
    htop = {
      name = "Process monitor";
      category = "system";
      description = "Live view of running programs, processor and memory use. Press q to quit.";
      package = pkgs.htop;
      command = [ (lib.getExe pkgs.htop) ];
      # Read-only: no killing or re-prioritising processes.
      restrictedCommand = [
        (lib.getExe pkgs.htop)
        "--readonly"
      ];
      keywords = [
        "processes"
        "cpu"
        "memory"
        "top"
      ];
    };
    ncdu = {
      name = "Disk usage explorer";
      category = "system";
      description = "Shows which folders use the most space in your home directory. Press q to quit.";
      package = pkgs.ncdu;
      command = [
        (lib.getExe pkgs.ncdu)
        "-x"
      ];
      # -rr: read-only, and no spawning of a shell.
      restrictedCommand = [
        (lib.getExe pkgs.ncdu)
        "-x"
        "-rr"
      ];
      keywords = [
        "disk"
        "space"
        "storage"
      ];
    };
  };
}
