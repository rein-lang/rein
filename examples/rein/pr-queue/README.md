# pr-queue

A merge queue agent program. It merges the pull requests in a queue, one at a time. It shows a callback: a function that the caller passes, so the callee goes on to it when its work is done.

`main` picks a pull request and hands it to `merge` with `then=@./main`. `merge` does not know who called it. After the merge, it returns `then`, so the flow goes back to `main` for the next pull request.

## Files

| File | Function | Job |
| --- | --- | --- |
| `main.rein` | `main` | Entry point. Stops when the queue is empty. Picks the next pull request. |
| `merge.rein` | `merge` | Merges one pull request, then returns its callback `then`. |

## Run it

```sh
rein check examples/rein/pr-queue
rein step --root examples/rein/pr-queue --json
```
