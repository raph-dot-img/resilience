# The Resilience userland: the `resilience` TUI binary.
# Unit and TUI tests (`cargo test`) run in the check phase of this build.
{ lib, rustPlatform }:

let
  root = ../../..;
  cargoToml = lib.importTOML (root + "/Cargo.toml");
in
rustPlatform.buildRustPackage {
  pname = "resilience";
  inherit (cargoToml.workspace.package) version;

  # Only the Rust sources: documentation or Nix edits do not trigger a rebuild.
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/Cargo.toml")
      (root + "/Cargo.lock")
      (root + "/crates")
    ];
  };

  cargoLock.lockFile = root + "/Cargo.lock";

  meta = {
    description = "Terminal interface of Resilience OS";
    license = lib.licenses.gpl3Plus;
    mainProgram = "resilience";
    platforms = lib.platforms.linux;
  };
}
