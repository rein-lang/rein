# commit-push

A program that commits the staged changes and pushes them. It shows the
proposed agent requirement tags: see
[Agent requirements](../../../docs/agent-requirements.md).

## Files

| File | Function | Job |
| --- | --- | --- |
| `main.rein` | `main` | Entry point. Asks for the cheapest model and medium effort or more. |
| `mismatch.rein` | `mismatch` | The fallback. Sends a subagent, or asks the user to open a new session. |

## Run it

```sh
cargo run -p rein -- check examples/rein/commit-push
cargo run -p rein -- run --root examples/rein/commit-push
```

Rein does not read the tags yet. `rein check` warns about each one, and the
run ignores them. `mismatch` is not reachable until the tags are built, so
`rein check` also warns that it is unused.
