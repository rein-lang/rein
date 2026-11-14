# The std functions

Status: proposal. Not built. [Agent requirements](../agent-requirements.md).

Std adds three functions and three types. Like `request`, each function is
built in and needs no import. Each one asks the agent one question about
itself, and the answer is a typed `const`.

| Function | Answer type | The agent answers |
| --- | --- | --- |
| `agentHarness()` | `Harness` | The harness it runs in. |
| `agentModel()` | `ModelTier` | The tier of its model in that harness. |
| `agentEffort()` | `Effort` | The effort it runs with. |

## The types

| Type | Members |
| --- | --- |
| `Harness` | `"claude-code"`, `"codex"`, `"gemini-cli"`, `"cursor"`, `"windsurf"`, `"cline"`, `"amp"`, `"hermes"`, `"pi"`, `"other"` |
| `ModelTier` | `"cheap"`, `"standard"`, `"frontier"`, `"unknown"` |
| `Effort` | `"low"`, `"medium"`, `"high"`, `"xhigh"`, `"max"`, `"unknown"` |

- Each type is a union of string literal types. A `switch` on it needs a case
  for every member, as for any union.
- The `Harness` members are the polyhook 0.2 caller names, plus `"other"`.
- `ModelTier` is a tier, not a model id. A model id ages, and each harness has
  its own ids. `"cheap"` is the smallest and fastest model of the harness,
  `"frontier"` the strongest, and `"standard"` the one between.
- `"unknown"` is for an agent that cannot see its model or effort. The program
  must handle it, so no setup is guessed.
- The type names are std names. A file cannot declare a struct with the same
  name.

## How the agent answers

A call is an async request with fixed text. The step is a `request`, as in
[Stepping](../spec/stepping.md), so the protocol does not change:

```json
{"kind": "request", "key": "main.effort", "function": "main",
 "instruction": "Say which effort you run with.",
 "response": {"name": "effort", "type": "union",
  "options": ["low", "medium", "high", "xhigh", "max", "unknown"],
  "description": "The effort you run with."}}
```

- The instruction text comes from std. The `const` doc comment is the
  `description`, as for every answer.
- Evaluation is lazy ([Evaluation](../spec/evaluation.md)). A program that only reads `effort` asks only
  for the effort.
- Rein does not check the answer against the real session. MCP does not send
  the model or the effort. The agent knows them from its own context.

## Checker rules

- A call is `const NAME: TYPE = await agentEffort();`, with a doc comment, in
  an `async` function. The rules of [Async requests](../spec/async-requests.md)
  apply.
- `TYPE` must be the function's own type. Another type is an error:
  `` `agentEffort()` answers `Effort`; write `const x: Effort` ``.
- The functions take no arguments: `` `agentEffort()` takes no arguments ``.
- A call can be in any function, not only the entry. A program usually calls
  them first, so a bad setup ends the run before any work.

## Today

Rein today does not know these names. `rein check` gives an error:
``only `request` or a reference like `@program` can be awaited``.
So a program cannot use them before the feature ships. Until then, the same
check is a plain `request` with an inline union, which works today:

```ts
/** The effort you run with. */
const effort: "low" | "medium" | "high" | "xhigh" | "max" | "unknown" =
  await request(`Say which effort you run with.`);
```
