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
        f0_0[/"request: ˋSay if your effort is medium or higher: ok, mismatch, or unknown.ˋ"/]
        f0_1{"Is your effort medium or higher?<br/>effort: #quot;ok#quot; | #quot;mismatch#quot; | #quot;unknown#quot;"}
        f0_2("Commit the staged changes with a short message, then push.")
        f0_3("Start a subagent on the cheapest model with effort medium.<br/>Give it this task: run the commit-push program.")
        f0_4("askUser(question=ˋIs your effort medium or higher?ˋ)")
    end
    f0 --> f0_0
    f0_0 --> f0_1
    f0_1 -->|"ok"| f0_2
    f0_1 -->|"mismatch"| f0_3
    f0_1 -->|"unknown"| f0_4
```

## Source

### `package.json`

<<< ../../../examples/rein/commit-push/package.json{json}

### `rein.json`

<<< ../../../examples/rein/commit-push/rein.json{json}

### `main.rein`

<<< ../../../examples/rein/commit-push/main.rein{rein}
