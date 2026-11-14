# The agent field

Status: proposal. Not built. Index: [Agent requirements](../agent-requirements.md).

The field goes in `rein.json`, next to `entry`. It is a Rein-specific field,
so it does not go in `package.json` (see [Install a program](../install-programs.md)).

```json
{
  "agent": {
    "model": { "recommended": "cheap" },
    "effort": { "required": "medium" }
  }
}
```

- `agent` has three optional keys: `harness`, `model` and `effort`.
- Each key takes an object with `required`, `recommended`, or both.
- `onMismatch` is optional too. It names the function that runs when the
  agent does not match. See [The fallback](fallback.md).
- A key that is not given means "any". A program with no `agent` field runs
  as today.

## `harness`

```json
"harness": { "required": ["claude-code", "codex"], "recommended": "claude-code" }
```

- `required` is a list of harness ids. The program starts only in one of them.
- `recommended` is one harness id. It is a hint for a person who picks the
  agent. Rein does not act on it.
- Harness ids are the polyhook caller names, such as `claude-code`, `codex`,
  `gemini-cli` and `opencode`.

## `model`

```json
"model": { "required": "standard", "recommended": "frontier" }
```

- A value is a tier: `cheap`, `standard` or `frontier`. The order is
  `cheap < standard < frontier`.
- `required` is a floor: the model is that tier or a higher one.
- `recommended` is the tier Rein resolves to a model, so the agent gets one
  model name. When it is not given, Rein uses `required`.
- Tiers, not model ids. A model id ages, and each harness has its own ids.
  A tier says what the program needs, and it stays true.

## `effort`

```json
"effort": { "required": "medium", "recommended": "high" }
```

- A value is `low`, `medium`, `high`, `xhigh` or `max`, in that order.
- `required` is a floor. `recommended` is the effort Rein sends, else
  `required`.
- A harness with fewer levels gets the nearest level that is not lower. If no
  level is high enough, it gets its highest level. For example, `max` on Codex
  is `xhigh`.

## Checker rules

- An unknown tier or effort is an error that lists the accepted values.
- `recommended` lower than `required` is an error:
  `` `agent.model.recommended` is `cheap`, below `required` `standard` ``.
- An empty `harness.required` list is an error.
- `harness.recommended` that is not in `harness.required` is an error.
- An unknown harness id is a warning, not an error. New harnesses appear.

