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
        f0_0[/"request: ˋSay which effort you run with.ˋ"/]
        f0_1{"effort === #quot;low#quot; || effort === #quot;unknown#quot;"}
        f0_2("Start a subagent on the cheapest model with effort medium.<br/>Give it this task: run the commit-push program.")
        f0_3("Commit the staged changes with a short message, then push.")
    end
    f0 --> f0_0
    f0_0 --> f0_1
    f0_1 -->|"yes"| f0_2
    f0_1 -->|"no"| f0_3
```

## Source

### `package.json`

<<< ../../../examples/rein/commit-push/package.json{json}

### `rein.json`

<<< ../../../examples/rein/commit-push/rein.json{json}

### `main.rein`

<<< ../../../examples/rein/commit-push/main.rein{rein}
