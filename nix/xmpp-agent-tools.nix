# xmpp-agent-tools.nix: the Rust workspace's toolchain, build, checks and dev-shell tools.
#
# Everything specific to this repo's targets lives here; flake.nix only wires it into flake-parts.
{
  inputs,
  pkgs,
}:
let
  # The toolchain is pinned here and by flake.lock; keep `rust-version` in Cargo.toml at or below it.
  rustToolchain = pkgs.rust-bin.stable."1.95.0".default.override {
    extensions = [
      "clippy"
      "rustfmt"
    ];
  };
  craneLib = (inputs.crane.mkLib pkgs).overrideToolchain rustToolchain;

  commonArgs = {
    src = craneLib.cleanCargoSource ./..;
    pname = "xmpp-agent-tools";
    version = "0.1.0";
    strictDeps = true;
  };
  # The dependencies compile once and every check below reuses them.
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
  withDeps = commonArgs // {
    inherit cargoArtifacts;
  };
in
{
  inherit rustToolchain craneLib;

  # One named output per CI check: `nix build -L .#checks.<system>.<name>`.
  checks = {
    clippy = craneLib.cargoClippy (
      withDeps // { cargoClippyExtraArgs = "--all-targets -- --deny warnings"; }
    );
    test = craneLib.cargoTest withDeps;
  };

  # The workspace build. It holds the libraries only; the binaries land here when they exist.
  package = craneLib.buildPackage withDeps;

  devShellPackages = [
    pkgs.cargo-mutants
    pkgs.actionlint
    pkgs.zizmor
    pkgs.python3
  ];
}
