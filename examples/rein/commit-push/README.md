# commit-push

A program that commits the staged changes and pushes them. It runs on any
model, but it needs medium effort or more. It asks the agent for its effort
first, and sends a subagent when the effort is too low.

It shows the proposed agent requirement checks: see
`docs/agent-requirements.md` in the repository. The std function
`requireEffort(min="medium")` is not built yet, so the program asks with
`request` and compares in its own code, which works today.

## Files

| File | Function | Job |
| --- | --- | --- |
| `main.rein` | `main` | Asks for the effort. Too low or unknown: send a subagent. Else: commit and push. |

## Run it

```sh
cargo run -p rein -- check examples/rein/commit-push
cargo run -p rein -- run --root examples/rein/commit-push
```
