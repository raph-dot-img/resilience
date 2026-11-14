# Settings shared by every Resilience system configuration.
{ lib, ... }:

{
  system.stateVersion = "26.05";
  time.timeZone = lib.mkDefault "UTC";
  i18n.defaultLocale = lib.mkDefault "en_US.UTF-8";
}
