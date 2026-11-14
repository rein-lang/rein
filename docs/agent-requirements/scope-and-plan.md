# Scope, build order and open questions

Status: step 1 is built. The rest is a proposal. [Agent requirements](../agent-requirements.md).

## Not in scope

- Rein does not start models, call model APIs, or count cost.
- Rein does not check the agent's answer against the real session.
- The std function is the ask, and the program's handlers are the only
  custom code.

## Build order

Each step is one small pull request.

1. Built: the std struct `Setup` and the std template `agentSetup`, with
   `await agentSetup(...)` as a built-in await. Tests in `rein-core`.
2. Built: completion for `agentSetup` in the language server (tested). Hover
   on it is not tested yet.
3. The std templates `subagent`, `localSession` and `remoteSession`.

## Open questions

- The name. `agentSetup`, or another?
- The tier names. `cheap`, `standard`, `frontier` avoid the word `medium`,
  which effort uses.
- Should `Setup` also carry the raw values, so a handler can name them in its
  text?
