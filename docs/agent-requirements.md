# Agent requirements (proposal)

Status: proposal. Nothing on this page is built.

A program author states which harness, model and effort a program needs.
Rein checks the statement, resolves it for the agent that runs the program,
and tells the agent. Rein does not launch models. The agent acts on the step,
as it acts on every other step.

Example: a program that only commits and pushes runs well on the cheapest
model of any harness, with medium effort or more. A review program needs the
strongest model and high effort.

## Why

- Most steps of a simple program need no strong model. A strong model costs
  more and runs slower.
- Some programs fail on a weak model or a low effort. The author knows this.
  The agent that starts the program does not.
- Today the author can only write the model as text in `delegate(...)`. Rein
  cannot check that text, and the text names one harness's models.

## Topics

- [The agent field](agent-requirements/agent-field.md): `harness`, `model`,
  `effort` in `rein.json`, and the checker rules.
- [Resolution and the step](agent-requirements/resolution.md): the tier map,
  the `agent` object in the first step, and how the agent matches.
- [The fallback](agent-requirements/fallback.md): what the agent gets on a
  mismatch, set by the author or the user, as a Rein function.
- [Hand-offs and worked examples](agent-requirements/handoffs-and-examples.md).
- [Scope, build order and open questions](agent-requirements/scope-and-plan.md).
