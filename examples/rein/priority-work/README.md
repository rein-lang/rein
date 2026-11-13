# priority-work

A program that needs the result of another function as a value. It finds the high priority projects of a workplace, then works on their pull requests.

`main` calls `find_projects` with `await`. The agent walks `find_projects` as it walks a hand-off: it answers its question and gets its final instruction. Then the agent does that instruction and answers `main.projects` with the result. `main` goes on with the answer.

## Files

| File | Function | Job |
| --- | --- | --- |
| `main.rein` | `main` | Entry point. Awaits `find_projects`, then routes on its result. |
| `find_projects.rein` | `find_projects` | Tells the agent how to find the priority repositories. |
| `open_prs.rein` | `open_prs` | Works on the open pull requests of the repositories. |

## Run it

```sh
cargo run -p rein -- check examples/rein/priority-work
cargo run -p rein -- step --root examples/rein/priority-work --input workplace=~/work
```
