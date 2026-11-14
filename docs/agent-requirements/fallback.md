# The fallback

Status: proposal. Not built. Index: [Agent requirements](../agent-requirements.md).

The fallback decides what the agent gets on a mismatch. The program author
sets it, and the user can replace it. Examples: send a subagent, open a new
local session, open a remote session, or ask the user.

The fallback is a Rein function, not a text field. So it can ask questions
and branch, like any other function, and the checker checks it.

```ts
/**
 * Review the pull request.
 *
 * @requires effort high
 * @mismatch @./mismatch
 */
```

- `@mismatch` takes a reference: `@./path` in the program, or
  `@program/function` in another installed program.
- The function's struct may have these `string` fields. Rein gives each one
  as an input, by name. Another `string` field is a checker error. A `boolean`
  or union field is a question, as in any function.
  - `program`: the program name.
  - `harness`: the harness of the caller, or `unknown`.
  - `model` and `effort`: what to run on, resolved as in [Resolution](resolution.md).
  - `gap`: one sentence that says what did not match.
  - `start`: the exact call that starts the program again, with its inputs,
    such as `rein_start name=pr-handling model=opus effort=high`.
- The fallback runs as a run of its own. It can ask questions and requests.
  When it ends, the agent does its instruction. The program itself did not
  start, so it has no run id to resume.

A fallback that asks the agent what it can do:

```ts
interface Input {
  /** Can you start a subagent with a chosen model and effort? */
  can_spawn: boolean;
  /** The harness of the caller. */
  harness: string;
  /** The model to run on. */
  model: string;
  /** The effort to run with. */
  effort: string;
  /** The call that starts the program again. */
  start: string;
}

/**
 * Tell the agent how to run the program on the right model.
 */
export default function mismatch(
  { can_spawn, harness, model, effort, start }: Input,
) {
  if (can_spawn) {
    return subagent(model=model, effort=effort, task=start);
  }

  return remoteSession(
    harness=harness, model=model, effort=effort, task=start,
  );
}
```

## Std templates for the fallback

Std adds three templates, so a fallback is often one line:

| Template | Text it gives the agent |
| --- | --- |
| `subagent({ model, effort, task })` | Start a subagent on `model` with effort `effort`, and give it `task`. |
| `localSession({ harness, model, effort, task })` | Open a new local `harness` session on `model` with effort `effort`, and run `task` in it. |
| `remoteSession({ harness, model, effort, task })` | Open a remote `harness` session on `model` with effort `effort`, and run `task` in it. |

`askUser(...)` from std also fits: the user decides.

## Who sets the fallback

The first one found wins:

1. The user: `mismatch` in `agents.json`. The user knows the setup, such as
   whether remote sessions exist. It is one reference for all programs.
2. The program: the `@mismatch` tag of its entry function.
3. Rein: the default, `subagent(model=model, effort=effort, task=start)`.

## Delegation in each harness

The std templates give text. Each harness acts on it its own way:

- Claude Code: the Agent tool takes `model` and `effort`. A remote session is
  a cloud session.
- Codex: `codex exec -m <model> -c model_reasoning_effort=<effort>`.
- Other harnesses: the instruction text only.

