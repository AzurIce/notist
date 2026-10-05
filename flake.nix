{
  description = "notist";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      crane,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachSystem
      [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ]
      (
        system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs { inherit system overlays; };
          craneLib = (crane.mkLib pkgs).overrideToolchain (
            p:
            p.rust-bin.nightly."2026-08-01".default.override {
              targets = [ "wasm32-unknown-unknown" ];
              extensions = [
                "rust-src"
                "rustfmt"
                "clippy"
              ];
            }
          );
          src = craneLib.cleanCargoSource ./.;
          commonArgs = {
            inherit src;
            pname = "notist";
            inherit (craneLib.crateNameFromCargoToml { cargoToml = ./crates/notist-cli/Cargo.toml; }) version;
            cargoExtraArgs = "-p notist-cli";
            strictDeps = true;
            # Integration tests use documentation and browser fixtures outside
            # cleanCargoSource; run them in the development checkout.
            doCheck = false;
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          notist = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              meta.description = "Notist document toolchain and language server";
              meta.mainProgram = "notist";
            }
          );
          notistApp = {
            type = "app";
            program = "${notist}/bin/notist";
            inherit (notist) meta;
          };
        in
        {
          packages = {
            default = notist;
            inherit notist;
          };
          apps = {
            default = notistApp;
            notist = notistApp;
          };
          devShells.default = craneLib.devShell {
            packages = with pkgs; [
              cargo-edit
              samply
              wasm-bindgen-cli
              miniserve
            ];
          };
        }
      );
}
