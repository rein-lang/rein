# merge-pr

An agent flow that decides whether a pull request can merge.

## Files

| File | Function | Job |
| --- | --- | --- |
| `main.rein` | `main` | Entry point. Routes the task to a merge or a security review. |
| `merge_pr.rein` | `merge_pr` | Checks docs-only changes and CI status. |
| `review_code.rein` | `review_code` | Code review step after green CI. Delegates the review to a subagent, on a stronger model when the PR is security-sensitive. |
| `security/review.rein` | `security/review` | Checks for secrets and auth changes. Reached from `main` for a security review task. |
| `ci/fix.rein` | `ci/fix` | Fix a red CI run. Then the agent starts this program again. |

## Run it

```sh
cargo run -p rein -- check examples/rein/merge-pr
cargo run -p rein -- run --root examples/rein/merge-pr
```

## Retry without a cycle

`merge_pr` references `ci/fix`. `ci/fix` does not reference back. Rein has no loops yet, so the retry ends the run.
After a fix is pushed, `ci/fix` returns the text "start this program again with the task merge".
The agent then starts the program again, and `merge_pr` checks the pull request again.
