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

          # The workspace build. It holds the libraries only; the binaries land here when they exist.
          packages.default = xat.package;

          devShells.default = xat.craneLib.devShell {
            checks = config.checks;
            packages = [ config.treefmt.build.wrapper ] ++ xat.devShellPackages;
          };
        };
    };
}
