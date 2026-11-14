# Agent requirements (proposal)

Status: proposal. Nothing on this page is built.

A program states which harness, model tier and effort it needs, with std
functions. Std asks the agent for its setup, compares it, and returns a typed
result. The program's own code is only the handlers: what the agent does when
the setup does not match.

Example: a program that only commits and pushes runs well on the cheapest
model of any harness, with medium effort or more. A review program needs the
strongest model and high effort.

## Why

- Most steps of a simple program need no strong model. A strong model costs
  more and runs slower.
- Some programs fail on a weak model or a low effort. The author knows this.
  The agent that starts the program does not.
- The check is the same in every program, so it belongs in std. What to do on
  a mismatch differs per program and per author: send a subagent, open a new
  local or remote session, ask the user, or go on. That is the handler.

## Example

```ts
/**
 * Commit the staged changes and push them.
 */
export default async function main() {
  /** Is your effort medium or higher? */
  const effort: Check = await requireEffort(min="medium");

  switch (effort) {
    case "ok":
      return `Commit the staged changes with a short message, then push.`;
    case "mismatch":
      return subagent(
        model=`the cheapest model`,
        effort=`medium`,
        task=`Run the commit-push program.`,
      );
    case "unknown":
      return askUser(question=`Is your effort medium or higher?`);
  }
}
```

- `requireEffort(min="medium")` is std. It asks the agent for its effort,
  compares it with `medium`, and returns a `Check`.
- The `switch` is the author's code. Each case is a handler. Rein has no
  `default`, so the author must handle `"mismatch"` and `"unknown"`.

## Topics

- [The std functions](agent-requirements/std-functions.md): the `require` and
  `recommend` functions, the `Check` result, and the checker rules.
- [Handlers](agent-requirements/handling.md): what a handler can do, the std
  templates for a bad setup, and hand-offs.
- [Scope, build order and open questions](agent-requirements/scope-and-plan.md).
