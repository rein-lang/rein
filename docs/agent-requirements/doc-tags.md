# The doc tags

Status: proposal. Not built. [Agent requirements](../agent-requirements.md).

The requirements are tags in the doc comment of the entry function, in
`main.rein`. They are not a manifest field and not a parameter. The doc
comment already describes the program, so the agent and the reader see the
requirements next to the description.

```ts
/**
 * Commit the staged changes and push them.
 *
 * @recommends model cheap
 * @requires effort medium
 */
export default function main() {
  return `Commit the staged changes, then push.`;
}
```

## The tags

| Tag | Meaning |
| --- | --- |
| `@requires <key> <value>` | The program needs this. A mismatch runs the fallback. |
| `@recommends <key> <value>` | The value Rein sends to the agent. No check. |
| `@mismatch <reference>` | The function that runs on a mismatch. See [The fallback](fallback.md). |

- `<key>` is `harness`, `model` or `effort`.
- A key with no tag means "any". A program with no tags runs as today.
- Each key has at most one `@requires` and one `@recommends`.
- When a key has only `@requires`, Rein sends that value.

## `harness`

```ts
 * @requires harness "claude-code" | "codex"
 * @recommends harness "claude-code"
```

- `@requires` takes a union of harness ids, as a TypeScript union of string
  literal types. The program starts only in one of them.
- `@recommends` takes one harness id. It is a hint for a person who picks the
  agent. Rein does not act on it.
- Harness ids are the polyhook 0.2 caller names: `claude-code`, `codex`,
  `gemini-cli`, `cursor`, `windsurf`, `cline`, `amp`, `hermes` and `pi`.

## `model`

```ts
 * @requires model standard
 * @recommends model frontier
```

- A value is a tier: `cheap`, `standard` or `frontier`. The order is
  `cheap < standard < frontier`.
- `@requires` is a floor: the model is that tier or a higher one.
- `@recommends` is the tier Rein resolves to a model, so the agent gets one
  model name.
- Tiers, not model ids. A model id ages, and each harness has its own ids.
  A tier says what the program needs, and it stays true.

## `effort`

```ts
 * @requires effort medium
 * @recommends effort high
```

- A value is `low`, `medium`, `high`, `xhigh` or `max`, in that order.
- `@requires` is a floor. `@recommends` is the effort Rein sends.
- A harness with fewer levels gets the nearest level that is not lower. If no
  level is high enough, it gets its highest level. For example, `max` on Codex
  is `xhigh`.

## Checker rules

- The tags go only in the doc comment of the entry function. On another
  function they are an error: `` `@requires` goes on the entry function ``.
- An unknown key, tier or effort is an error that lists the accepted values.
- A second `@requires` or `@recommends` for one key is an error.
- `@recommends` lower than `@requires` is an error:
  `` `@recommends model cheap` is below `@requires model standard` ``.
- `@recommends harness` that is not in `@requires harness` is an error.
- An unknown harness id is a warning, not an error. New harnesses appear.
- `@mismatch` takes a function reference. The reference rules apply.

## Today

The parser already reads tags. Rein today gives a warning for each one:
``unknown tag `@requires`; Rein supports no tags yet``. `rein check` passes
and `rein step` runs. So a program can add the tags now, before the feature
ships.
