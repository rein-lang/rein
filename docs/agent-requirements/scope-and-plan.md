# Scope, build order and open questions

Status: proposal. Not built. Index: [Agent requirements](../agent-requirements.md).

- Rein does not start models, call model APIs, or count cost.
- Rein ships no model ids. Only aliases that a harness keeps stable.
- A model or effort per function. The program is the unit. `delegate(...)`
  stays the way to send one task to another model.

## Build order

Each step is one small pull request.

1. `rein-core`: parse and check the `@requires`, `@recommends` and
   `@mismatch` tags of the entry function.
2. `rein show` and `rein list --json` print the requirements.
3. The resolver: the harness from MCP `initialize` or `--harness`, the user
   tier map, and the `claude-code` default.
4. `rein_start`, `rein run` and `rein step`: the `model` and `effort`
   arguments, the `agent` object, and the fallback run.
5. The agent context: the delegation rule.
6. Hand-offs: the `agent` object at a hand-off, and the floor that stays.

## Open questions

- The tier names. `cheap`, `standard`, `frontier` avoid the word `medium`,
  which effort uses.
- Is `@recommends harness` worth keeping, if Rein never acts on it?
- Should a `delegate(...)` call take a tier, and let Rein resolve it?
