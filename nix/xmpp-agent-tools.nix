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

  # The Python side (the CI scripts and the tools not yet ported). Standard library only, plus coverage for the floor.
  pythonSrc = pkgs.lib.cleanSource ../.;
  python = pkgs.python3.withPackages (ps: [ ps.coverage ]);
  # Every Python check copies the source to a writable directory first: the tests write caches, and the scrub and mutation
  # checks need a git repository (they scan tracked files).
  pythonCheck =
    name: nativeBuildInputs: script:
    pkgs.runCommand "xmpp-agent-tools-${name}"
      {
        nativeBuildInputs = [
          python
          pkgs.git
        ]
        ++ nativeBuildInputs;
        PYTHONDONTWRITEBYTECODE = "1";
      }
      ''
        cp -r ${pythonSrc} work
        chmod -R u+w work
        cd work
        git init -q . && git add .
        ${script}
        touch $out
      '';
in
{
  inherit rustToolchain craneLib;

  # One named output per CI check: `nix build -L .#checks.<system>.<name>`.
  checks = {
    clippy = craneLib.cargoClippy (
      withDeps // { cargoClippyExtraArgs = "--all-targets -- --deny warnings"; }
    );
    test = craneLib.cargoTest withDeps;

    # Unit tests with branch coverage and a floor of 90%.
    py-tests = pythonCheck "py-tests" [ ] ''
      PYTHONPATH=src python -m coverage run --branch --source=src -m unittest discover -s tests -t .
      python -m coverage report -m --fail-under=90
    '';
    # Every one-line mutation must fail the tests.
    mutation = pythonCheck "mutation" [ ] "python scripts/mutation-check.py";
    # No internal hostnames, private addresses, home paths or personal names in this public repository.
    scrub = pythonCheck "scrub" [ ] "python ci/scrub.py";
    # Workflow lint. zizmor runs offline: its online audits need a token and the network.
    workflows =
      pythonCheck "workflows"
        [
          pkgs.actionlint
          pkgs.zizmor
        ]
        ''
          actionlint .github/workflows/*.yaml
          zizmor --offline --format plain .github/workflows
        '';
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
