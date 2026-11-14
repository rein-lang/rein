# Scope, build order and open questions

Status: proposal. Not built. [Agent requirements](../agent-requirements.md).

## Not in scope

- Rein does not start models, call model APIs, or count cost.
- Rein does not check the agent's answer against the real session.
- The std function is the check, and the program's handlers are the only
  custom code.

## Build order

Each step is one small pull request.

1. `rein-core`: the `Check` type and the six functions as built-in awaits.
   The request asks for the agent's value; the evaluator compares it and
   gives the `Check`. Checker rules and tests.
2. The language server: hover and completion for the new std names.
3. The std templates `subagent`, `localSession` and `remoteSession`.
4. `examples/rein/commit-push` moves from `request` to `requireEffort`.

## Open questions

- The names. `requireEffort` and `recommendModel`, or one function with a
  `kind` argument?
- The tier names. `cheap`, `standard`, `frontier` avoid the word `medium`,
  which effort uses.
- When a program calls two checks on the model, does the agent answer twice,
  or does one answer serve the run?
