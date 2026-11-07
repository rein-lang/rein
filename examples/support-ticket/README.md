# support-ticket

A customer support agent flow. It decides how to answer a customer ticket.

The flow routes VIP customers to a human first. Other tickets follow their category. Billing tickets can end in a refund. How-to tickets get a help article when one matches. The agent answers whether the article solved the problem, then the flow closes the ticket or escalates it.

## Files

| File | Function | Job |
| --- | --- | --- |
| `main.rein` | `main` | Entry point. Routes by VIP status and category. |
| `billing/refund.rein` | `billing/refund` | Refunds within the agent limit. Escalates larger refunds. |
| `answer/faq.rein` | `answer/faq` | Answers how-to questions from the help center. |
| `escalate.rein` | `escalate` | Hands the ticket to a human engineer. |
| `templates/tell_customer.rein` | `templates/tell_customer` | A template: one sentence for the customer. `billing/refund` calls it. |

## Run it

```sh
cargo run -p rein -- check examples/support-ticket
cargo run -p rein -- run --root examples/support-ticket
```
