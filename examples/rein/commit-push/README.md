# commit-push

A program that commits the staged changes and pushes them. It runs best on
the cheapest model, and it needs medium effort or more. It asks the agent for
its setup first with the std function `agentSetup`, and sends a subagent when
the effort does not match.

## Files

| File | Function | Job |
| --- | --- | --- |
| `main.rein` | `main` | Asks the setup. Effort ok: commit and push. Mismatch: send a subagent. Unknown: ask the user. |

## Run it

```sh
cargo run -p rein -- check examples/rein/commit-push
cargo run -p rein -- run --root examples/rein/commit-push
```
