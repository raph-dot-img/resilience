# The Resilience userland: the `resilience` TUI and its helpers
# `resilience-admin` (privileged, ADR 0007) and `resilience-backup` (ADR 0008).
# Unit and TUI tests (`cargo test`) run in the check phase of this build.
{
  lib,
  rustPlatform,
  age,
  zstd,
}:

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

  # The backup round-trip test runs the real tools (tar, sha256sum: stdenv).
  nativeCheckInputs = [
    age
    zstd
  ];

  meta = {
    description = "Terminal interface of Resilience OS";
    license = lib.licenses.bsd0;
    mainProgram = "resilience";
    platforms = lib.platforms.linux;
  };
}
