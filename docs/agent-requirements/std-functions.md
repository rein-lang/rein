# The std functions

Status: proposal. Not built. [Agent requirements](../agent-requirements.md).

Std adds six functions. Like `request`, each one is built in and needs no
import. Each one asks the agent one question about itself, compares the
answer, and gives the program a `Check`.

| Function | Argument | `"ok"` when the agent's value is |
| --- | --- | --- |
| `requireHarness(any_of)` | harness ids, joined with `\|` | one of them |
| `requireModel(min)` | a tier | that tier or a stronger one |
| `requireEffort(min)` | an effort | that effort or a higher one |
| `recommendHarness(id)` | one harness id | that harness |
| `recommendModel(tier)` | a tier | exactly that tier |
| `recommendEffort(level)` | an effort | exactly that effort |

- A `require` function is a floor: the program cannot run below it.
- A `recommend` function is a target: the program runs anywhere, but this
  setup fits it best. A stronger model than recommended is a mismatch too,
  because it costs more.

## The values

| Kind | Values, lowest first |
| --- | --- |
| Harness id | `claude-code`, `codex`, `gemini-cli`, `cursor`, `windsurf`, `cline`, `amp`, `hermes`, `pi`, `other` |
| Model tier | `cheap`, `standard`, `frontier` |
| Effort | `low`, `medium`, `high`, `xhigh`, `max` |

- The harness ids are the polyhook 0.2 caller names, plus `other`.
- A tier, not a model id. A model id ages, and each harness has its own ids.
  `cheap` is the smallest and fastest model of the harness, `frontier` the
  strongest, `standard` the one between.
- An argument is a tag in quotes, such as `min="medium"`. A value outside the
  list is an error that lists the values.

## The result

`Check` is a std union: `"ok" | "mismatch" | "unknown"`.

- `"ok"`: the setup matches.
- `"mismatch"`: the agent gave a value, and it does not match.
- `"unknown"`: the agent cannot see that value, such as its effort.

## How the agent answers

A call stops the run with a `request`, as in [Stepping](../spec/stepping.md).
The agent answers its own value. Std compares it, and the `const` gets the
`Check`.

```json
{"kind": "request", "key": "main.effort", "function": "main",
 "instruction": "Say which effort you run with.",
 "response": {"name": "effort", "type": "union",
  "options": ["low", "medium", "high", "xhigh", "max", "unknown"],
  "description": "Is your effort medium or higher?"}}
```

- The agent answers a value, such as `high`. The program sees `"ok"`.
- The step protocol does not change. Only the evaluator learns the compare.
- Evaluation is lazy ([Evaluation](../spec/evaluation.md)). A check runs only
  when the path reaches it.
- Rein does not check the answer against the real session. MCP does not send
  the model or the effort. The agent knows them from its own context.

## Checker rules

- A call is `const NAME: Check = await requireEffort(min="medium");`, with a
  doc comment, in an `async` function. The rules of
  [Async requests](../spec/async-requests.md) apply.
- The type must be `Check`: `` `requireEffort` gives a `Check`; write
  `const x: Check` ``.
- Each function takes its one named argument.
- `Check` is a std name. A file cannot declare a struct with that name.

## Today

Rein today does not know these names. `rein check` gives an error:
``only `request` or a reference like `@program` can be awaited``. Until the
feature ships, a program asks with `request` and compares in its own code.
`examples/rein/commit-push` does this.
