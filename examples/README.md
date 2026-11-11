# Examples

This folder acts like a repo. Its `rein/` folder holds the example programs, one folder per program, each with a `rein.json` manifest.

- [merge-pr](rein/merge-pr): checks a pull request and picks the next step. Shows a retry that starts the program again.
- [incident-triage](rein/incident-triage): route a production alert to a page, a runbook, or a ticket.
- [support-ticket](rein/support-ticket): route a customer ticket. Shows a template and an async request.
- [pr-handling](rein/pr-handling): one pull request. Takes the link as the input `pr` and passes it on.

Run `rein list` from here to list them:

```sh
cd examples
rein list
rein run support-ticket
```
