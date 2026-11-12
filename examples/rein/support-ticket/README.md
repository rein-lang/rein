# support-ticket

A customer support agent flow. It decides how to answer a customer ticket.

The flow routes VIP customers to a human first. Other tickets follow their category. Billing tickets can end in a refund. How-to tickets get a help article when one matches. The agent answers whether the article solved the problem, then the flow closes the ticket or escalates it.

## Files

| File | Function | Job |
| --- | --- | --- |
| `main.rein` | `main` | Entry point. Routes by VIP status and category. |
| `refund.rein` | `refund` | Refunds within the agent limit. Escalates larger refunds. |
| `faq.rein` | `faq` | Answers how-to questions from the help center. |
| `escalate.rein` | `escalate` | Hands the ticket to a human engineer. |
| `refund/tell_customer.rein` | `refund/tell_customer` | A template: one sentence for the customer. `refund` calls it. |
| `message.struct.rein` | `Message` | A shared struct. It is the input of both templates. |
| `faq/close_ticket.rein` | `faq/close_ticket` | A template: a last message, then close the ticket. `faq` calls it. |

## Run it

```sh
cargo run -p rein -- check examples/rein/support-ticket
cargo run -p rein -- run --root examples/rein/support-ticket
```
