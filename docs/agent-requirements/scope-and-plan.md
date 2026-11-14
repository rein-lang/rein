# Scope, build order and open questions

Status: proposal. Not built. [Agent requirements](../agent-requirements.md).

## Not in scope

- Rein does not start models, call model APIs, or count cost.
- Rein does not check the agent's answer against the real session.
- The std function is the ask and the compare, and the program's handlers are the only
  custom code.

## Build order

Each step is one small pull request.

1. `rein-core`: the `Setup` struct and `agentSetup` as a built-in await. The
   request asks for the agent's matrix; the evaluator compares each row and
   gives the `Setup`. Checker rules and tests.
2. The language server: hover and completion for the new std names.
3. The std templates `subagent`, `localSession` and `remoteSession`.
4. `examples/rein/commit-push` moves from `request` to `agentSetup`.

## Open questions

- The name. `agentSetup`, or another?
- The tier names. `cheap`, `standard`, `frontier` avoid the word `medium`,
  which effort uses.
- Should `Setup` also carry the raw values, so a handler can name them in its
  text?
