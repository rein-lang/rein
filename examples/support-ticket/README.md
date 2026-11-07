# support-ticket

A customer support agent flow. It decides how to answer a customer ticket.

The flow routes VIP customers to a human first. Other tickets follow their category. Billing tickets can end in a refund. How-to tickets get a help article when one matches.

## Files

| File | Function | Job |
| --- | --- | --- |
| `handle_ticket.rein` | `handle_ticket` | Entry point. Routes by VIP status and category. |
| `billing/refund.rein` | `billing/refund` | Refunds within the agent limit. Escalates larger refunds. |
| `answer/faq.rein` | `answer/faq` | Answers how-to questions from the help center. |
| `escalate.rein` | `escalate` | Hands the ticket to a human engineer. |

## Run it

```sh
cargo run -p rein -- check examples/support-ticket
cargo run -p rein -- run handle_ticket --root examples/support-ticket
```
