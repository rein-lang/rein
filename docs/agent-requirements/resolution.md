# Resolution and the step

Status: proposal. Not built. Index: [Agent requirements](../agent-requirements.md).

Rein resolves the field when a run starts. It needs two facts.

1. The harness of the caller. `rein serve --mcp` reads the client name from
   the MCP `initialize` request (`client_info.name`, from `peer_info()` in
   rmcp). A client name is not always a harness id, so Rein keeps a small
   map from client names to ids. An unknown client name is `unknown`.
   Known: Codex 0.154.0 sends `codex-mcp-client` (`codex`). Claude Code
   2.1.325 sends `claude-code`, the same as its harness id. `rein run` and `rein step` take
   `--harness <id>` or `REIN_HARNESS`. With no harness, Rein resolves nothing
   and sends the tiers as text.
2. The model for each tier in that harness. The user tier map holds it, in
   `agents.json` in the Rein config folder:

```json
{
  "claude-code": { "cheap": "haiku", "standard": "sonnet", "frontier": "opus" },
  "codex": { "cheap": "<model id>", "standard": "<model id>", "frontier": "<model id>" }
}
```

- Rein ships a default only for a harness with stable model aliases. Today
  that is `claude-code` (`haiku`, `sonnet`, `opus`). It ships no model ids,
  because ids age.
- A harness with no map entry gets the tier name as text. The agent picks the
  model.
- The user map wins over the default.

## The step tells the agent

The first step of a run has an `agent` object when the entry function has
requirement tags. `rein_start`, the `first_step` of `rein_programs`, and the first
`rein step` response carry it.

```json
"agent": {
  "harness": "claude-code",
  "model": "haiku",
  "tier": "cheap",
  "effort": "medium",
  "required": ["effort"],
  "instruction": "Run this program on model haiku with effort medium or higher."
}
```

- `required` lists the keys that the program requires. The other keys are
  recommendations.
- `instruction` is one sentence for the agent. Agents act on text, and this is
  the Rein way: Rein gives instructions, it does not manage the harness.
- The agent context from the hook adds one rule: when a step has `agent`, and
  you do not match it, start the run in a subagent with that model and effort.

## How the agent matches

Rein cannot see the model of the session. MCP does not send it. So the agent
says it:

- `rein_start` takes two optional arguments, `model` and `effort`. The agent
  gives its own model and effort. `rein run` and `rein step` take `--model`
  and `--effort`.
- With the arguments, Rein checks the floors. A model below a required tier,
  or an effort below a required level, is a mismatch. The program does not
  start. Rein runs [the fallback](fallback.md) instead.
- Without the arguments, Rein cannot check. The run starts, and the step has
  the `agent` instruction.
- A harness that is not in `harness.required` is a mismatch at start, with or
  without the arguments, because Rein knows the harness.

To map a model name to a tier, Rein uses the same tier map, read backwards.
A model that is not in the map is unknown, and Rein treats it as unchecked.

