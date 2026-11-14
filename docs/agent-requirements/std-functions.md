# The std function

Status: proposal. Not built. [Agent requirements](../agent-requirements.md).

Std adds one built-in function, `agentSetup`, and one std struct, `Setup`.
Like `request`, it needs no import. Its name is open (see
[open questions](scope-and-plan.md)).

```ts
/** Your setup. */
const setup: Setup = await agentSetup(harness="claude-code | codex",
  model="cheap", effort="medium+");
```

## The targets

Each argument is optional. A missing argument means "any".

| Argument | Value | `"ok"` when the agent's value is |
| --- | --- | --- |
| `harness` | ids joined with ` \| `, such as `"claude-code \| codex"` | one of them |
| `model` | a tier, such as `"cheap"` | exactly that tier |
| `model` | a tier with `+`, such as `"standard+"` | that tier or a stronger one |
| `effort` | an effort, such as `"high"` | exactly that effort |
| `effort` | an effort with `+`, such as `"medium+"` | that effort or a higher one |

| Row | Values |
| --- | --- |
| harness (no order) | `claude-code`, `codex`, `gemini-cli`, `cursor`, `windsurf`, `cline`, `amp`, `hermes`, `pi`, `other` |
| model, lowest first | `cheap`, `standard`, `frontier` |
| effort, lowest first | `low`, `medium`, `high`, `xhigh`, `max` |

- The harness ids are the polyhook 0.2 caller names, plus `other`.
- A tier, not a model id. A model id ages, and each harness has its own ids.
  `cheap` is the smallest and fastest model of the harness, `frontier` the
  strongest, `standard` the one between.
- An exact target also fails on a stronger model, because a stronger model
  costs more. A `+` target accepts it.
- A value outside the lists is an error that lists the values.

## The result

`Setup` is a std struct with one row per argument:

| Field | Type |
| --- | --- |
| `harness` | `"ok" \| "mismatch" \| "unknown"` |
| `model` | `"ok" \| "mismatch" \| "unknown"` |
| `effort` | `"ok" \| "mismatch" \| "unknown"` |

- `"ok"`: the value matches the target, or the argument is missing.
- `"mismatch"`: the agent gave a value, and it does not match.
- `"unknown"`: the agent cannot see that value, such as its effort, and the
  argument is given.

## How the agent answers

A call stops the run with one `request`, as in
[Stepping](../spec/stepping.md). The agent gives the matrix as a struct:

```json
{"kind": "request", "key": "main.setup", "function": "main",
 "instruction": "Give your setup: harness, model tier and effort.",
 "response": {"name": "setup", "type": "struct", "fields": [
  {"name": "harness", "type": "union", "options": ["claude-code", "codex",
   "gemini-cli", "cursor", "windsurf", "cline", "amp", "hermes", "pi",
   "other", "unknown"]},
  {"name": "model", "type": "union",
   "options": ["cheap", "standard", "frontier", "unknown"]},
  {"name": "effort", "type": "union",
   "options": ["low", "medium", "high", "xhigh", "max", "unknown"]}]}}
```

- The `instruction` and the fields come from std. The agent answers its
  values, such as `{"harness": "codex", "model": "cheap", "effort": "high"}`.
- Std compares, and the `const` gets the `Setup` of checks.
- The step protocol does not change. Only the evaluator learns the compare.
- Rein does not check the answer against the real session. MCP does not send
  the model or the effort. The agent knows them from its own context.

## Checker rules

- A call is `const NAME: Setup = await agentSetup(...);`, with a doc comment,
  in an `async` function. The rules of
  [Async requests](../spec/async-requests.md) apply, with one change: `await`
  takes `agentSetup`, next to `request` and a reference. That page and
  [Std and templates](../spec/std-and-templates.md) list it once it is built.
- The type must be `Setup`: `` `agentSetup` gives a `Setup`; write
  `const x: Setup` ``.
- `Setup` is a std name. A file cannot declare a struct with that name.

## Today

Rein today does not know `agentSetup`. Until it ships, a program asks with
`request` for a struct of the raw values and compares them in its own code.
`examples/rein/commit-push` does this.
