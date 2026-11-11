# pr-handling

A pull request agent program. It decides the next step for one pull request: review it, answer its comments, fix its CI, or merge it. The program takes the pull request link as its input `pr`.

The program stops on a draft. The merge task checks CI, conflicts and the review state, in that order. It hands the agent to the fix, the comments or the review when one of them blocks the merge. A review never merges in the same run.

The program has no reference cycle. A retry ends the run with the text "start this program again".

## Files

| File | Function | Job |
| --- | --- | --- |
| `main.rein` | `main` | Entry point. Stops on a draft. Routes by task. |
| `review.rein` | `review` | Delegates the review to a subagent. Posts one review from the result. |
| `comments/address.rein` | `comments/address` | Asks the agent to answer each open comment. |
| `ci/fix.rein` | `ci/fix` | Routes a failed CI run by its cause. Asks the agent to push a fix. |
| `merge.rein` | `merge` | Merges when CI is green, no conflict exists, and the review approves. |
| `templates/post_review.rein` | `templates/post_review` | A template: one review with a type and a body. `review` calls it. |

Each function gets `pr` from its caller and passes it on. There is no global scope.

## Run it

```sh
rein check examples/rein/pr-handling
rein step --root examples/rein/pr-handling --input pr=https://example.com/pr/1 --json
```

## Sample walk

The merge task on a pull request that is ready. Each call after the first repeats the input and the earlier answers. `...` stands for them. Each output line shows part of the JSON.

```text
$ rein step --root examples/rein/pr-handling --input pr=https://example.com/pr/1 --json
{"kind": "question", "key": "main.draft", "description": "Is the pull request still a draft?", "trace": ["main"]}
# exit 2

$ rein step ... --answer main.draft=no
{"kind": "question", "key": "main.task", "options": ["review", "address comments", "fix ci", "merge"], "description": "Which task does the agent do on the pull request?", "trace": ["main"]}
# exit 2

$ rein step ... --answer main.task=merge
{"kind": "question", "key": "merge.ci", "options": ["green", "red", "pending"], "description": "What is the CI status of the PR?", "trace": ["main", "merge"]}
# exit 2

$ rein step ... --answer merge.ci=green
{"kind": "question", "key": "merge.conflicts", "description": "Does the PR conflict with its base branch?", "trace": ["main", "merge"]}
# exit 2

$ rein step ... --answer merge.conflicts=no
{"kind": "question", "key": "merge.review", "options": ["approved", "changes requested", "no review"], "description": "What is the review state of the PR?", "trace": ["main", "merge"]}
# exit 2

$ rein step ... --answer merge.review=approved
{"kind": "done", "instruction": "Merge the pull request https://example.com/pr/1. Then delete its branch.", "trace": ["main", "merge"]}
# exit 0
```
