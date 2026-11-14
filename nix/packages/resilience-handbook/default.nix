# Offline handbook pages shown by the TUI (plain text / light Markdown).
{ lib, runCommand }:

let
  src = lib.fileset.toSource {
    root = ../../../handbook;
    fileset = lib.fileset.fileFilter (f: f.hasExt "md" || f.hasExt "txt") ../../../handbook;
  };
in
runCommand "resilience-handbook" { meta.description = "Resilience OS offline handbook"; } ''
  mkdir -p $out/share/resilience/handbook
  cp ${src}/* $out/share/resilience/handbook/
''
