# merge-pr

An agent flow that decides whether a pull request can merge.

## Files

| File | Function | Job |
| --- | --- | --- |
| `main.rein` | `main` | Entry point. Routes the task to a merge or a security review. |
| `merge_pr.rein` | `merge_pr` | Checks docs-only changes and CI status. |
| `review_code.rein` | `review_code` | Code review step after green CI. Delegates the review to a subagent, on a stronger model when the PR is security-sensitive. |
| `security/review.rein` | `security/review` | Checks for secrets and auth changes. Reached from `main` for a security review task. |
| `ci/fix.rein` | `ci/fix` | Fix a red CI run, then return to `merge_pr`. |

## Run it

```sh
cargo run -p rein -- check examples/.rein/merge-pr
cargo run -p rein -- run --root examples/.rein/merge-pr
```

## Intentional cycle

`merge_pr` references `ci/fix`, and `ci/fix` references back to `merge_pr`:

```text
merge_pr -> ci/fix -> merge_pr
```

This loop is on purpose. Rein has no loops yet, so the cycle is the retry.
`rein check` reports it as a reference cycle warning. The warning is expected for this example.
