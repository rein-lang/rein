# Handle the answers

Status: proposal. Not built. [Agent requirements](../agent-requirements.md).

The answers are values. The program handles them with the normal Rein rules:
`if` guards and `switch`, with no `else` and no `default`.

## Required

A required setup is an `if` that returns another path when the answer is not
good enough. Rein has no `<` on unions, so the program lists the members that
are too low:

```ts
  /** The tier of your model. */
  const tier: ModelTier = await agentModel();

  if (tier !== "frontier") {
    return askUser(question=`This review needs the strongest model. Switch?`);
  }
```

## Recommended

A recommended setup is an `if` that adds a hint, then goes on. A commit
program runs on any model, but a strong one costs more:

```ts
  /** The tier of your model. */
  const tier: ModelTier = await agentModel();

  if (tier === "frontier") {
    return @./commit(hint=`A cheaper model can run this program next time.`);
  }

  return @./commit(hint=`-`);
```

Or no check at all: a program that does not ask has no requirement.

## Per harness

A `switch` on `Harness` gives each harness its own path, such as the right
way to start a subagent there:

```ts
  /** The harness you run in. */
  const harness: Harness = await agentHarness();

  switch (harness) {
    case "claude-code":
      return `Use the Agent tool with model haiku and effort medium.`;
    case "codex":
      return `Run codex exec -m <cheap model> -c model_reasoning_effort=medium.`;
    // ... a case for every other member, as for any union.
  }
```

## Std templates for a bad setup

Std adds three templates, next to `delegate` and `askUser`. Each one gives the
agent one instruction:

| Template | Text it gives the agent |
| --- | --- |
| `subagent({ model, effort, task })` | Start a subagent on `model` with effort `effort`, and give it `task`. |
| `localSession({ harness, model, effort, task })` | Open a new local `harness` session on `model` with effort `effort`, and run `task` in it. |
| `remoteSession({ harness, model, effort, task })` | Open a remote `harness` session on `model` with effort `effort`, and run `task` in it. |

- `task` usually starts the same program again, such as
  `` `Run the commit-push program.` ``. The new agent answers the checks
  again, and this time they pass.
- How each harness acts on the text: in Claude Code the Agent tool takes
  `model` (`haiku`, `sonnet`, `opus`, `fable`) and `effort` (`low` to `max`).
  Codex takes `codex exec -m <model> -c model_reasoning_effort=<effort>`.

## Hand-offs

A program can hand off to another program (`return @program/function`). The
target asks its own checks, if it has any, when the run reaches them. The
answers are per call, so the target's agent answers for itself. When the
caller already sent a subagent, that subagent answers.
