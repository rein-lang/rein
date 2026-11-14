---
description: "Commit the staged changes and push them."
---

<!--@include: ../../../examples/rein/commit-push/README.md-->

## Graph

The decision flow, from `rein graph commit-push`:

```mermaid
flowchart TD
    subgraph s0 ["main.rein"]
        f0(["main"])
        f0_0("Commit the staged changes with a short message, then push.")
    end
    subgraph s1 ["mismatch.rein"]
        f1[["mismatch(model, effort, start)"]]
        f1_0{"Can you start a subagent with a chosen model and effort?<br/>can_spawn: boolean"}
        f1_1("Start a subagent on ${model} with effort ${effort}.<br/>Give it this task: ${start}")
        f1_2("askUser(question=ˋOpen a new session on ${model} with effort ${effort}.<br/>Then run: ${start}ˋ)")
    end
    f0 --> f0_0
    f1 --> f1_0
    f1_0 -->|"yes"| f1_1
    f1_0 -->|"no"| f1_2
```

## Source

### `package.json`

<<< ../../../examples/rein/commit-push/package.json{json}

### `rein.json`

<<< ../../../examples/rein/commit-push/rein.json{json}

### `main.rein`

<<< ../../../examples/rein/commit-push/main.rein{rein}

### `mismatch.rein`

<<< ../../../examples/rein/commit-push/mismatch.rein{rein}
