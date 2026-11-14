# Scope, build order and open questions

Status: proposal. Not built. [Agent requirements](../agent-requirements.md).

## Not in scope

- Rein does not start models, call model APIs, or count cost.
- Rein does not check the agent's answer against the real session.
- No manifest field, doc tag or tier map. The program's code is the one home
  for the rule.

## Build order

Each step is one small pull request.

1. `rein-core`: the std types `Harness`, `ModelTier` and `Effort`, and the
   three functions as built-in awaits with fixed request text. Checker rules
   and tests.
2. The language server: hover and completion for the new std names.
3. The std templates `subagent`, `localSession` and `remoteSession`.
4. The `examples/rein/commit-push` program moves from `request` to
   `agentEffort()`.

## Open questions

- The names. `agentHarness`, `agentModel`, `agentEffort`, or shorter ones?
- The tier names. `cheap`, `standard`, `frontier` avoid the word `medium`,
  which effort uses.
- Should Rein add `<` and `>=` for these ordered unions, so a floor is one
  test instead of a list of members?
