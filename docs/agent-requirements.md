# Agent requirements (proposal)

Status: proposal. Nothing on this page is built.

A program asks the agent that runs it which harness, model tier and effort it
has. Three std functions ask it. The program calls them at its start, gets
typed values, and handles each value in its own code, with `if` and `switch`.

Example: a program that only commits and pushes runs well on the cheapest
model of any harness, with medium effort or more. A review program needs the
strongest model and high effort.

## Why

- Most steps of a simple program need no strong model. A strong model costs
  more and runs slower.
- Some programs fail on a weak model or a low effort. The author knows this.
  The agent that starts the program does not.
- The author decides what happens on a bad setup: send a subagent, open a new
  local or remote session, ask the user, or go on. That is a rule, so it is
  code, not a manifest field.

## Example

```ts
/**
 * Commit the staged changes and push them.
 */
export default async function main() {
  /** The effort you run with. */
  const effort: Effort = await agentEffort();

  if (effort === "low" || effort === "unknown") {
    return subagent(
      model=`the cheapest model`,
      effort=`medium`,
      task=`Run the commit-push program.`,
    );
  }

  return `Commit the staged changes with a short message, then push.`;
}
```

- `agentEffort()` asks the agent. The agent answers one member of `Effort`.
- The program reads the answer like any other answer. "Required" is an `if`
  that returns another path. "Recommended" is an `if` that adds a hint, or no
  check at all.
- Rein checks nothing about the agent itself. The agent answers, and the
  program decides.

## Topics

- [The std functions](agent-requirements/std-functions.md): `agentHarness`,
  `agentModel`, `agentEffort`, their types, and the checker rules.
- [Handle the answers](agent-requirements/handling.md): required and
  recommended setups, the std templates for a bad setup, and hand-offs.
- [Scope, build order and open questions](agent-requirements/scope-and-plan.md).
