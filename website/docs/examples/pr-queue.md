---
description: "Merge the pull requests in a queue, one at a time. Shows a callback."
---

<!--@include: ../../../examples/rein/pr-queue/README.md-->

## Graph

The decision flow, from `rein graph pr-queue`:

```mermaid
flowchart TD
    subgraph s0 ["main.rein"]
        f0(["main"])
        f0_0[/"request: #96;Open the merge queue. Look for a pull request that waits.#96;"/]
        f0_1{"Is a pull request waiting in the merge queue?<br/>waiting: boolean"}
        f0_2("Stop. The merge queue is empty.")
        f0_3[/"request: #96;Pick the oldest pull request.#96;<br/>pick: Pick"/]
    end
    subgraph s1 ["merge.rein"]
        f1[["merge(pr, then)"]]
        f1_0[/"request: #96;Merge ${pr}. Then delete its branch.#96;"/]
        f1_1{"Did the pull request merge?<br/>merged: boolean"}
        f1_2("askUser(question=#96;The merge of ${pr} failed. What do I do?#96;)")
        f1_3[["then"]]:::program
    end
    f0 --> f0_0
    f0_0 --> f0_1
    f0_1 -->|"no"| f0_2
    f0_1 -->|"yes"| f0_3
    f0_3 -->|"pr=pick.pr<br/>then=@./main"| f1
    f1 --> f1_0
    f1_0 --> f1_1
    f1_1 -->|"no"| f1_2
    f1_1 -->|"yes"| f1_3
    classDef program stroke-dasharray:4 2
```

## Source

### `package.json`

<<< ../../../examples/rein/pr-queue/package.json{json}

### `rein.json`

<<< ../../../examples/rein/pr-queue/rein.json{json}

### `main.rein`

<<< ../../../examples/rein/pr-queue/main.rein{rein}

### `merge.rein`

<<< ../../../examples/rein/pr-queue/merge.rein{rein}
