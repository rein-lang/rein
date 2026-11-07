# merge-pr

An agent flow that decides whether a pull request can merge.

## Files

| File | Function | Job |
| --- | --- | --- |
| `merge_pr.rein` | `merge_pr` | Entry point. Checks docs-only changes and CI status. |
| `review_code.rein` | `review_code` | Code review step after green CI. |
| `security/review.rein` | `security/review` | Extra review for security-sensitive changes. |
| `ci/fix.rein` | `ci/fix` | Fix a red CI run, then return to `merge_pr`. |

## Run it

```sh
cargo run -p rein -- check examples/merge-pr
cargo run -p rein -- run merge_pr --root examples/merge-pr
```

## Intentional cycle

`merge_pr` refers to `ci/fix`, and `ci/fix` refers back to `merge_pr`:

```text
merge_pr -> ci/fix -> merge_pr
```

This loop is on purpose. Rein has no loops yet, so the cycle is the retry.
`rein check` reports it as a reference cycle warning. The warning is expected for this example.
