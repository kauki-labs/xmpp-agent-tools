# How a release is cut

Modelled on the sandy repo. Releases are prepared but **off**: every job in `release.yaml` is gated on the repository variable
`ENABLE_RELEASES`. While it is unset, nothing is opened, tagged or released.

## What a release is

One version for the whole workspace, one tag `v<version>`, one GitHub release with generated notes and one archive per platform:

| Platform | Runner |
| -------- | ------ |
| x86_64 Linux | `ubuntu-24.04` |
| aarch64 Linux | `ubuntu-24.04-arm` |
| aarch64 macOS | `macos-15` |

Each archive is built natively through the flake (`nix build .#default`) and comes with a `.sha256` file. There is no cross build.
x86_64 macOS is not built. crates.io publishing is off (`publish = false`).

The workspace builds libraries only today, so a release has a tag, notes and a changelog but no archives yet: the binaries job
prints a notice and attaches nothing until a crate with a binary exists.

## Flow

1. Commits land on `main` with conventional titles (`feat:`, `fix:`, `docs:`).
2. On every push to `main`, `release.yaml` runs `release-plz release-pr`. It opens or updates a PR that bumps the version and writes
   `CHANGELOG.md` from the commit titles.
3. Run a check on that PR: it was opened with `GITHUB_TOKEN`, which does not start `pr.yaml`. Push an empty commit or use "Update
   branch", so that `Merge gate` runs. Then review and merge it.
4. The merge is a push to `main`. `release-plz release` sees a version with no tag, creates the tag and the GitHub release, and the
   `binaries` jobs build the archives and upload them to that release. They run in the same workflow because a tag or release made
   with `GITHUB_TOKEN` does not start another workflow.

## Turn it on

1. Set the repository variable `ENABLE_RELEASES=true`.
2. Allow GitHub Actions to create pull requests (Settings, Actions, General, Workflow permissions). Without it `release-pr` fails.
3. Add every new crate to the `version_group` list in `release-plz.toml`, so the workspace keeps one version and one tag.
4. Optional: add the Nix binary cache step in `release.yaml` and `pr.yaml` (commented out in both).

## Roll back

- A bad release: delete the GitHub release and the tag (`gh release delete v<version> --cleanup-tag`), then merge a fix. The next
  release PR proposes the next version; a version is never reused.
- A bad archive for one platform: re-run that `binaries` job; it uploads with `--clobber`.
- Stop all releases: unset `ENABLE_RELEASES`.

## Weekly audit

`audit-schedule.yaml` runs `nix run .#audit` on Mondays: `cargo audit` of `Cargo.lock` against a fresh clone of the RustSec database.
A new advisory shows up as a red run in the Actions tab; nothing is blocked. The hermetic check `nix flake check` does not run it,
because a pinned database would hide new advisories.

## Not verified

The release workflow has not run: it needs `ENABLE_RELEASES` and a merged release PR. In particular, these parts are written from the
tools' documentation and sandy's setup, not exercised:

- `release-plz.toml` (release-plz does not build on the macOS machine where this was written, so the config was not parsed by it);
- the JSON that `release-plz release -o json` prints, which the workflow reads to find the tag;
- the `ubuntu-24.04-arm` and `macos-15` runners building through the flake;
- the archive upload.

Run the first release with the variable set and watch it; fix the workflow in a normal PR if a step differs from what is written here.
