{
  description = "xmpp-agent-tools: a Rust workspace, built and checked with Nix";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    treefmt-nix.url = "github:numtide/treefmt-nix";

    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      imports = [ inputs.treefmt-nix.flakeModule ];

      perSystem =
        {
          config,
          system,
          ...
        }:
        let
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ (import inputs.rust-overlay) ];
          };

          # The toolchain is pinned here and by flake.lock; keep `rust-version` in Cargo.toml at or below it.
          rustToolchain = pkgs.rust-bin.stable."1.95.0".default.override {
            extensions = [
              "clippy"
              "rustfmt"
            ];
          };
          craneLib = (inputs.crane.mkLib pkgs).overrideToolchain rustToolchain;

          src = craneLib.cleanCargoSource ./.;
          commonArgs = {
            inherit src;
            pname = "xmpp-agent-tools";
            version = "0.1.0";
            strictDeps = true;
          };
          # The dependencies compile once and every check below reuses them.
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        {
          # Rust formatting and Nix formatting. `nix fmt` formats; `checks.fmt` only checks.
          treefmt = {
            projectRootFile = "flake.nix";
            flakeCheck = false;
            programs = {
              nixfmt.enable = true;
              rustfmt = {
                enable = true;
                package = rustToolchain;
                edition = "2024";
              };
            };
          };
          formatter = config.treefmt.build.wrapper;

          # One named output per CI check: `nix build -L .#checks.<system>.<name>`.
          checks = {
            fmt = config.treefmt.build.check inputs.self;
            clippy = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoClippyExtraArgs = "--all-targets -- --deny warnings";
              }
            );
            test = craneLib.cargoTest (commonArgs // { inherit cargoArtifacts; });
          };

          # The workspace build. It holds the libraries only; the binaries land here when they exist.
          packages.default = craneLib.buildPackage (commonArgs // { inherit cargoArtifacts; });

          devShells.default = craneLib.devShell {
            checks = config.checks;
            packages = [
              config.treefmt.build.wrapper
              pkgs.cargo-mutants
              pkgs.actionlint
              pkgs.zizmor
              pkgs.python3
            ];
          };
        };
    };
}
