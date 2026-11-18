# The licence notice (ADR 0024). Resilience's own code is under the
# Hippocratic License 2.1 (LICENSE). Before its first use of the interface,
# each account is shown a summary and the original text, and certifies it
# read it; `resilience-admin record-licence` (polkit
# org.resilience.admin.record-licence, active local session, no password)
# appends a timestamped line to a root-owned record. On the live ISO the
# record is in memory, so the notice is shown at every start.
{
  config,
  lib,
  ...
}:

let
  licenceFile = ../../LICENSE;
in
{
  options.resilience.licence = {
    prompt = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Show the licence notice before an account's first use of the
        interface. Only automated tests turn it off; an evaluation
        invariant requires it on every real system.
      '';
    };
    settings = lib.mkOption {
      type = lib.types.attrs;
      internal = true;
      readOnly = true;
      default = {
        name = "Hippocratic License";
        version = "2.1";
        file = "${licenceFile}";
        sha256 = builtins.hashFile "sha256" licenceFile;
        record = "/var/lib/resilience/licence-acceptances.log";
        inherit (config.resilience.licence) prompt;
      };
      description = "The [licence] section of system.toml and admin.toml.";
    };
  };
}
