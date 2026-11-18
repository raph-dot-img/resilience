# The manifest of an offline update bundle (ADR 0023): which new system each
# installed variant of an edition takes. A store path, signed with the
# release key like every path of the bundle. The toplevels are listed without
# string context: the manifest does not reference them, so importing it
# imports nothing else (the helper then imports only this computer's system).
{ lib, writeText }:
{
  edition,
  version,
  systems,
}:
writeText "resilience-update-${edition}-${version}" (
  builtins.unsafeDiscardStringContext ''
    format = 1
    edition = "${edition}"
    version = "${version}"

    [systems]
    ${lib.concatStrings (lib.mapAttrsToList (n: t: "${n} = \"${t}\"\n") systems)}''
)
