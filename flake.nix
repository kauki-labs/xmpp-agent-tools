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

          xat = import ./nix/xmpp-agent-tools.nix {
            inherit inputs pkgs;
          };
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
                package = xat.rustToolchain;
                edition = "2024";
              };
            };
          };
          formatter = config.treefmt.build.wrapper;

          # One named output per CI check: `nix build -L .#checks.<system>.<name>`.
          checks = xat.checks // {
            fmt = config.treefmt.build.check inputs.self;
          };

          # The workspace build: the libraries and the binaries (`xmpp-agent-rooms`).
          packages.default = xat.package;

          # `nix run .#audit`: cargo audit against a fresh clone of the RustSec database (the weekly workflow runs it).
          apps.audit = {
            type = "app";
            program = pkgs.lib.getExe (
              pkgs.writeShellApplication {
                name = "audit";
                runtimeInputs = [ pkgs.cargo-audit ];
                text = "cargo-audit audit";
              }
            );
          };

          devShells.default = xat.craneLib.devShell {
            checks = config.checks;
            packages = [
              config.treefmt.build.wrapper
            ]
            ++ xat.devShellPackages
            # nixpkgs marks release-plz broken on darwin; the release job runs on Linux.
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.release-plz ];
          };
        };
    };
}
