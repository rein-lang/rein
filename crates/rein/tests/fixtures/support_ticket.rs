//! Inline `@` fixtures shared by the LSP and migrate tests. They are the programs as they were before named imports.
#![allow(dead_code)]

/// The `support-ticket` program with its `@` references, as it was before named imports.
pub const SUPPORT_TICKET: &[(&str, &str)] = &[
    (
        "rein.json",
        r#"{
  "entry": "main.rein"
}
"#,
    ),
    (
        "package.json",
        r#"{
  "name": "support-ticket",
  "version": "0.1.0",
  "description": "Decide how the support agent handles a customer ticket.",
  "license": "MIT",
  "private": true,
  "files": ["**/*.rein", "README.md", "rein.json"],
  "engines": {
    "rein": "^0.1.0"
  }
}
"#,
    ),
    (
        "main.rein",
        r#"interface Ticket {
  /** What category best fits the ticket? */
  category: "billing" | "account" | "how_to";
  /** Is the customer on the VIP plan? */
  vip: boolean;
}

/**
 * Decide how the support agent handles a customer ticket.
 */
export default function main({ category, vip }: Ticket) {
  if (vip) {
    return @./escalate;
  }

  switch (category) {
    case "billing":
      return @./refund;
    case "account":
      return @./escalate;
    case "how_to":
      return @./faq;
  }
}
"#,
    ),
    (
        "escalate.rein",
        r#"/**
 * Hand the ticket to a human support engineer.
 */
export default function escalate() {
  return `Assign the ticket to the tier 2 queue. Tell the customer a person
  will reply within one business day. Do not promise a refund or a fix.`;
}
"#,
    ),
    (
        "faq.rein",
        r#"/**
 * Answer a how-to question from the help center.
 */
export default async function answerFaq(
  /** Did you find a help article that answers the question? */
  article_found: boolean,
) {
  if (!article_found) {
    return @./escalate;
  }

  /** Did the customer confirm the article solved the problem? */
  const solved: boolean = await request(
    `Reply with the help article link and ask the customer if it solved the
    problem.`,
  );

  if (solved) {
    return @./faq/close_ticket(
      opening=`Glad the help article solved the problem.`,
      next_step=`Reply to this ticket if it comes back.`,
    );
  }

  return @./escalate;
}
"#,
    ),
    (
        "faq/close_ticket.rein",
        r#"/**
 * Send the customer a last message, then close the ticket.
 */
export default function closeTicket({ opening, next_step }: @../message) {
  return `Tell the customer: ${opening} ${next_step} Then close the ticket as
  solved.`;
}
"#,
    ),
    (
        "message.struct.rein",
        r#"/**
 * One sentence for the customer: what happened, then what comes next.
 */
export default interface Message {
  /** The first part of the sentence, such as the action taken. */
  opening: string;
  /** The next step the customer can expect. */
  next_step: string;
}
"#,
    ),
    (
        "refund.rein",
        r#"/**
 * Process a refund request from a billing ticket.
 */
export default function refund(
  /** Is the requested refund above the agent's limit? */
  refund_over_limit: boolean,
) {
  if (refund_over_limit) {
    return @./escalate;
  }

  return @./refund/tell_customer(
    opening=`Refund the amount the customer asked for and confirm it in the
    ticket.`,
    next_step=`Then tell the customer the refund takes 5 business days.`,
  );
}
"#,
    ),
    (
        "refund/tell_customer.rein",
        r#"/**
 * Tell the customer what happens next, in one sentence.
 */
export default function tellCustomer({ opening, next_step }: @../message) {
  return `${opening} ${next_step}`;
}
"#,
    ),
];
