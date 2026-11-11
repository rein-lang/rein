# incident-triage

An on-call agent flow. It decides how to handle a new production alert.

The flow asks three questions: how severe the alert is, whether customers feel it, and whether a runbook exists. Then it picks one of three outcomes: page a human, follow the runbook, or open a ticket.

## Files

| File | Function | Job |
| --- | --- | --- |
| `main.rein` | `main` | Entry point. Routes the alert. |
| `page_human.rein` | `page_human` | Pages the on-call engineer. |
| `runbook/follow.rein` | `runbook/follow` | Asks the agent to run the known runbook and report whether the alert cleared. Pages a human if it did not. |
| `ticket/open.rein` | `ticket/open` | Opens or updates a ticket for non-urgent alerts. |

## Run it

```sh
cargo run -p rein -- check examples/rein/incident-triage
cargo run -p rein -- run --root examples/rein/incident-triage
```
