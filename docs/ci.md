# How the pull-request workflow works

One workflow, [`.github/workflows/pr.yaml`](../.github/workflows/pr.yaml), runs on every pull request into `main` and on the merge
queue (`merge_group`). **`Merge gate` is the only required check.** A ruleset requires that exact job name, so do not rename it.

| Job | Runs when | What it does |
| --- | --------- | ------------ |
| `changes` | always | Lists the changed paths and classifies them with `ci/changes.py`. |
| `tests` | code changed | Unit tests with branch coverage and a 90% floor. |
| `mutation` | code changed | `scripts/mutation-check.py`: every one-line mutation must fail the tests. |
| `scrub` | always | `ci/scrub.py`: no internal hostnames, private addresses, home paths or personal names in this public repo. |
| `workflows` | `.github/` changed | `actionlint` and `zizmor` on the workflow files. |
| `Merge gate` | always, after the others | Green only if every job above is `success` or `skipped`. |

## Which jobs a change runs

`ci/changes.py` reads the changed paths and answers two questions:

- **Docs:** a `.md` file or `LICENSE`. A change that touches only docs skips `tests` and `mutation`.
- **Workflows:** anything under `.github/`. Only these changes run `workflows`.
- **Everything else is code**, including unknown file types and scripts under `docs/`. An empty or unknown diff runs everything.

A skipped job does not block the merge: the gate treats `skipped` as green. It treats `failure`, `cancelled`, any unknown result, a
malformed entry and an empty list of jobs as red (`ci/merge_gate.py`, fail closed).

## Add a job

1. Add the job to `pr.yaml`. Give it `permissions: contents: read` (or less), pin every action by commit SHA with the version in a
   comment, and pass `${{ }}` values through `env:`, never inside a `run:` block.
2. List it in the `needs` of `Merge gate`.
3. If it should be skipped for some changes, add an `if:` on a `changes` output and extend `ci/changes.py` with a test first.
4. Run `actionlint` and `zizmor` on the workflow before pushing.

## Run the checks locally

```sh
just test      # unit tests
just coverage  # coverage, floor 90% (needs `pip install coverage`)
just mutate    # mutation check
just scrub     # scrub check
```

## Known limits

- The gate runs code from the pull request itself (`ci/merge_gate.py` and the workflow file), as any required check does. A pull
  request could weaken it. `.github/CODEOWNERS` names the owner of `.github/` and `ci/`, but it only enforces review when the ruleset
  has "Require review from Code Owners" turned on.
- Pinned tool versions (`coverage`, `zizmor`, `actionlint`) and action SHAs need updating by hand until a Renovate config exists.
