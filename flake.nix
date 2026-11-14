{
  description = "Resilience OS — an offline-first, terminal-only operating system based on NixOS";

  inputs = {
    # Production systems track the NixOS stable release branch only.
    # Bump deliberately with `nix flake update` and record it in CHANGELOG.md.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  };

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      lib = nixpkgs.lib;
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ self.overlays.default ];
      };

      mkSystem =
        modules:
        lib.nixosSystem {
          inherit system;
          modules = [
            self.nixosModules.default
            ./nix/systems/common.nix
          ]
          ++ modules;
        };

      vmTest = file: pkgs.testers.runNixOSTest (import file { inherit self; });
    in
    {
      overlays.default = import ./nix/overlays/default.nix;

      nixosModules.default = ./nix/modules;

      nixosConfigurations = {
        live-standard = mkSystem [
          ./nix/profiles/standard.nix
          ./nix/systems/live-iso.nix
        ];
        live-minimal = mkSystem [
          ./nix/profiles/minimal.nix
          ./nix/systems/live-iso.nix
        ];
        dev-vm = mkSystem [
          ./nix/profiles/standard.nix
          ./nix/systems/dev-vm.nix
        ];
      };

      packages.${system} = {
        inherit (pkgs) resilience resilience-session resilience-handbook;
        default = pkgs.resilience;
        iso = self.nixosConfigurations.live-standard.config.system.build.isoImage;
        iso-minimal = self.nixosConfigurations.live-minimal.config.system.build.isoImage;
        vm = self.nixosConfigurations.dev-vm.config.system.build.vm;
      };

      apps.${system}.vm = {
        type = "app";
        program = "${self.packages.${system}.vm}/bin/run-resilience-dev-vm";
        meta.description = "Run the Resilience development VM (2 vCPU, 2 GiB RAM, serial console)";
      };

      checks.${system} = {
        # Builds the Rust workspace and runs `cargo test`.
        resilience = pkgs.resilience;
        vm-boot-offline = vmTest ./tests/vm/boot-offline.nix;
        vm-modes = vmTest ./tests/vm/modes.nix;
        vm-admin = vmTest ./tests/vm/admin.nix;
        vm-backup = vmTest ./tests/vm/backup.nix;
        vm-wifi = vmTest ./tests/vm/wifi.nix;
        vm-generations = vmTest ./tests/vm/generations.nix;
        vm-packs = vmTest ./tests/vm/packs.nix;
        security-invariants = pkgs.callPackage ./tests/eval/security-invariants.nix {
          configurations = {
            inherit (self.nixosConfigurations) live-standard live-minimal dev-vm;
          };
        };
        # Evaluates the ISO configurations without building the images.
        # (Context is discarded on purpose: this check must not build them.)
        eval-live-iso = pkgs.writeText "eval-live-iso" (
          builtins.unsafeDiscardStringContext (
            builtins.toJSON {
              standard = self.nixosConfigurations.live-standard.config.system.build.toplevel.drvPath;
              minimal = self.nixosConfigurations.live-minimal.config.system.build.toplevel.drvPath;
              dev-vm = self.nixosConfigurations.dev-vm.config.system.build.vm.drvPath;
            }
          )
        );
      };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          cargo
          rustc
          clippy
          rustfmt
          rust-analyzer
          cargo-deny
          nixfmt
          qemu_kvm
        ];
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };

      formatter.${system} = pkgs.nixfmt-tree;
    };
}
