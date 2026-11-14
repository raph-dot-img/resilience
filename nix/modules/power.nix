# Power management defaults. Profiles (Maximum Efficiency / Balanced /
# Performance) are on the roadmap; until then the efficient defaults apply.
{ lib, ... }:

{
  powerManagement.enable = true;
  powerManagement.cpuFreqGovernor = lib.mkDefault "schedutil";
  services.logind.settings.Login = {
    HandleLidSwitch = lib.mkDefault "suspend";
    HandlePowerKey = lib.mkDefault "poweroff";
  };
}
