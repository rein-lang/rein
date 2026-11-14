# Hand-offs and worked examples

Status: proposal. Not built. Index: [Agent requirements](../agent-requirements.md).

A program can hand off to another program (`return @program/function`).
The target can have its own `agent` field.

- The step that enters the target carries the target's `agent` object, when
  it differs from the current one.
- When the target raises a required floor, the agent hands the rest of the run
  to a subagent with the new model and effort. The subagent goes on with
  `rein_answer` on the same run id.
- A target never lowers the floor of the run that called it. The higher floor
  stays until the run ends.

## Worked examples

A program that commits and pushes. Any harness, the cheapest model, medium
effort or more:

```json
{ "agent": { "model": { "recommended": "cheap" }, "effort": { "required": "medium" } } }
```

In Claude Code, the first step says: run on `haiku`, effort `medium` or
higher. The model is a recommendation, so a session on `opus` may run it. The
effort is required, so a session that gives `effort: "low"` gets an error
step.

A review program. Claude Code only, the strongest model, high effort:

```json
{
  "agent": {
    "harness": { "required": ["claude-code"] },
    "model": { "required": "frontier" },
    "effort": { "required": "high" }
  }
}
```

In Codex, `rein_start` gives an error step: the program requires
`claude-code`.

