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
        f0_0[/"request: ˋGive your setup: model tier and effort.ˋ<br/>setup: Matrix"/]
        f0_1{"The effort you run with.<br/>setup.effort: #quot;low#quot; | #quot;medium#quot; | #quot;high#quot; | #quot;xhigh#quot; | #quot;max#quot; | #quot;unknown#quot;"}
        f0_2("askUser(question=ˋIs your effort medium or higher?ˋ)")
        f0_3{"The effort you run with.<br/>setup.effort: #quot;low#quot; | #quot;medium#quot; | #quot;high#quot; | #quot;xhigh#quot; | #quot;max#quot; | #quot;unknown#quot;"}
        f0_4("Start a subagent on the cheapest model with effort medium.<br/>Give it this task: run the commit-push program.")
        f0_5("Commit the staged changes with a short message, then push.")
    end
    f0 --> f0_0
    f0_0 --> f0_1
    f0_1 -->|"unknown"| f0_2
    f0_1 -->|"low / medium / high / xhigh / max"| f0_3
    f0_3 -->|"low"| f0_4
    f0_3 -->|"medium / high / xhigh / max / unknown"| f0_5
```

## Source

### `package.json`

<<< ../../../examples/rein/commit-push/package.json{json}

### `rein.json`

<<< ../../../examples/rein/commit-push/rein.json{json}

### `main.rein`

<<< ../../../examples/rein/commit-push/main.rein{rein}
