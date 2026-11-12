# Proposed: `iter`

Status: proposed, not built. The checker, the evaluator and the CLI do not run this yet.
Read [spec.md](spec.md) for the current language and [interface.md](interface.md) for the current agent interface.

## 1. Purpose

The agent has a list, for example its open pull requests.
The agent sends the whole list once, as JSON.
Rein runs one function for each item, one item at a time, in list order.
This is like Rust `iter()` over a collection.

- The agent does not send the list again.
- Each item has its own questions. The agent answers them.
- The run ends after the last item.

## 2. Language form

### 2.1 The call

```ts
/** Handle each open pull request, one at a time. */
export default function main({ prs }: Input) {
  return iter(items=prs, each=@./handle_pr);
}
```

- `iter` is a std item. It needs no import.
- `iter` is the value of a `return`. It is not a template.
- `items` names a list field of the input. Both arguments are named and both are required.
- `each` is a hand-off, `@./handle_pr`. It takes no arguments.
- `each` is the only argument that is a hand-off. `logic` of `self_improve` (PR #10, not merged) is the same kind of exception.
- Only the entry function can use `iter`. Only the entry function holds a list (section 2.2).

Proposed grammar:

```text
iterReturn := "return" "iter" "(" "items" "=" IDENT "," "each" "=" REF ","? ")" ";"?
listField  := DOC IDENT ":" ("string" | REF) "[]" (";" | ",")?   // a list field of an input struct
```

### 2.2 The list type

- `T[]` is a list type. `T` is `string` or a reference to a shared struct.
- A list is a field of an input struct: `prs: @./pr[];`. A list of text is `urls: string[];`.
- Rein has no imports. The reference names the struct, as in `({ a }: @./message)`.
- A field still cannot be a struct. Only a list names a struct, and only by reference.
- A list field is an input. The agent is never asked for it.
- Only `items=` reads a list. A list is never interpolated, read by a condition, or passed to a call.

### 2.3 The item type

An item is one element of the list.

- A `string` item: `each` takes one `string` parameter. The item text is its input.
- A shared struct item: `each` takes the struct, `({ url, title }: @./pr)`. It names every field.
- Every field of an item struct is a `string` in this proposal. Item values come from JSON, and no question asks for them.
- The item binds to the one parameter of `each`. `each` takes no other parameter. A shared value must be a field of each item.
- The type of the `each` parameter must equal the item type.

The item struct is a shared `.struct.rein` file. The entry input and `each` both name it. An inline struct belongs to one function, so it cannot be the item type.

## 3. Input

The primary form is JSON. The value of a list field is a JSON array.

- Step protocol: the `inputs` object of the request (section 11 of spec.md) carries the array.
- MCP: `rein_start` takes `inputs`, a JSON object. The value of `prs` is a JSON array.
- CLI: `rein run <name> --input 'prs=[...]'`. A list field takes JSON. Other fields take text. `rein run` has no `--input` flag yet. This proposal adds one.

```json
{"version": 1, "entry": "main", "inputs": {"prs": [{"url": "https://example.com/pr/1", "title": "Fix a typo"}]}, "answers": {}}
```

Rein checks the list before the first step. Each error names the fix:

```text
`prs` is a list; give a JSON array, such as prs=[{...}]
`prs[1]` has no field `title`; Pr has fields url, title
`prs[1]` has field `draft`; Pr has fields url, title
`prs[2].url` must be a string; got 3
`prs` takes at most 100 items; got 120
`Pr` has a boolean field `draft`; an item field is a string in this proposal
`each` takes a string, but `prs` is a list of Pr; take `({ url, title }: @./pr)`
`each` takes the item, not arguments; write `each=@./handle_pr`
`items` must name a list field of this function; `x` is not one
`iter` runs in the entry function only; `handle_pr` cannot use it
```

An empty list is valid. The run ends at once with the text `No items. Nothing to do.`

## 4. Run

### 4.1 Order

- Rein starts item run 0. An item run calls `each` with item 0.
- An item run ends with a final text: a string, a template, or a template call.
- A hand-off inside an item run works as it does now.
- When an item run ends, Rein sends an `item_done` step (section 4.4). The agent does its text, then answers `yes`.
- Rein then starts the next item run. After the last item, Rein sends the final `done`.

### 4.2 Answer keys

The form is `iter[<index>].<function path>.<name>`. The index is the position of the item in the list, from 0.

- `iter[0].handle_pr.draft` is the `draft` question of `handle_pr`, for item 0.
- `iter[2].ci/fix.cause` is a question of `ci/fix`, reached in item 2.
- `iter[1].done` ends item 1 (section 4.4).
- A key outside an item keeps its form, such as `main.x`.

### 4.3 Trace and status

- The trace names the item: `iter[0]:handle_pr`, `iter[0]:ci/fix`.
- Each `question`, `request` and `item_done` step has an `item` field: `{"index": 1, "count": 3}`.
- `rein_status` gives the current `item` of a live run.

### 4.4 Step kinds

- `question` and `request` keep their fields. They add `item` when they run inside an item.
- New kind `item_done`. Its fields are `key` (`iter[<index>].done`), `instruction` and `item`. Its exit code is 3, as for a request. The agent does `instruction`, then answers `yes`.
- `done` keeps its fields. Its `instruction` lists the final text of each item, in order.

Protocol version 1 stays for programs without `iter`. A client must know `item_done` before it runs an `iter` program (open question 8).

### 4.5 Item end, skip and stop

- An item has no failure state. Its final text is its outcome. Rein does not read the text for failure.
- The agent cannot skip one item. An item ends only through its own flow.
- The agent stops the whole run with `rein_stop`.

### 4.6 Stateless steps

`rein step` keeps no state. Each call replays from item 0 with the answers it gets.

- An item is finished when its `iter[<index>].done` answer is present.
- The first item without that answer is the current item.
- To resume, send the same list and the same answers.

Each item run has its own limit of 64 functions (section 11 of spec.md). The limit does not count the whole call.

### 4.7 Live runs

- A live run keeps the list and the current index.
- `rein_answer` takes the value for the key the run waits on, as it does now.
- A live run does not accept a new list.

### 4.8 `rein eval`

`rein eval` runs one function. It refuses `iter` with this message: `iter runs with rein step or a live run`.

## 5. Graph

- A new node kind `iter`, labeled `each item of prs`.
- An `each` edge goes from the `iter` node to the entry node of `each`.
- The nodes of `each` sit in one subgraph, named after the function. The subgraph is the loop body.
- A `next` edge goes from the end of `each` back to the `iter` node.
- JSON output: node kind `iter`, edge kinds `each` and `next`.
- `--refs` draws one `iter` node and one `each` edge.

## 6. Limits and open questions

Decided in this proposal:

- At most 100 items per list. Each call replays the list, so the cost grows with the count.
- An empty list is valid.
- Items run in list order. Rein does not sort them.
- Items run one at a time. Parallel iteration is not part of this proposal.
- A nested `iter` is an error. Only the entry function holds a list.

Open questions:

1. Should item fields allow `boolean` and union types? Their values would come from JSON.
2. Should a list pass to another function? Today only the entry function holds a list.
3. Keys use the index. If the list changes between calls, an answer can reach the wrong item. A key field per item would fix this.
4. Should the agent skip one item? One option is an answer `iter[<index>].skip`.
5. Should an item have a failure state for the summary?
6. Is 100 the right limit? Is the 64-function limit right per item run?
7. Should a long list come from a file, such as `--input prs=@prs.json`?
8. Is `item_done` a new kind under protocol version 1, or does it need version 2?

## 7. Worked example (illustrative)

The output below is illustrative. It shows the shape of the steps. It is not real output.

Folder `pr-batch/` holds three files.

`pr.struct.rein` is the item struct:

```ts
/**
 * One open pull request.
 */
export default interface Pr {
  /** The link of the pull request. */
  url: string;
  /** The title of the pull request. */
  title: string;
}
```

`main.rein` is the entry function:

```ts
interface Input {
  /** The open pull requests to handle, in order. */
  prs: @./pr[];
}

/**
 * Handle each open pull request, one at a time.
 */
export default function main({ prs }: Input) {
  return iter(items=prs, each=@./handle_pr);
}
```

`handle_pr.rein` is the function for one item:

```ts
/**
 * Handle one open pull request: stop when it is a draft, otherwise review it and merge it when approved.
 */
export default async function handlePr({ url, title }: @./pr) {
  /** Is the pull request still a draft? */
  const draft: boolean = await request(`Open ${url} and check whether it is a draft.`);
  if (draft) {
    return `Stop. The pull request ${url} is a draft. Leave it for its author.`;
  }
  /** Does the review approve the pull request? */
  const approved: boolean = await request(`Review ${url} and post one review.`);
  if (approved) {
    return `Merge ${url}. Then delete its branch.`;
  }
  return `Ask the author of ${url} to answer the review comments: ${title}.`;
}
```

The input has three pull requests. The agent sends the list once:

```text
rein run pr-batch --input 'prs=[{"url":"https://example.com/pr/1","title":"Fix a typo"},{"url":"https://example.com/pr/2","title":"Add a retry"},{"url":"https://example.com/pr/3","title":"Bump the logger"}]' --json
```

The walk, with one answer per line (`>`):

```text
{"kind":"request","key":"iter[0].handle_pr.draft","item":{"index":0,"count":3},"instruction":"Open https://example.com/pr/1 and check whether it is a draft.","response":{"name":"draft","type":"boolean"},"trace":["main","iter[0]:handle_pr"]}
> no
{"kind":"request","key":"iter[0].handle_pr.approved","item":{"index":0,"count":3},"instruction":"Review https://example.com/pr/1 and post one review.","response":{"name":"approved","type":"boolean"},"trace":["main","iter[0]:handle_pr"]}
> yes
{"kind":"item_done","key":"iter[0].done","item":{"index":0,"count":3},"instruction":"Merge https://example.com/pr/1. Then delete its branch.","trace":["main","iter[0]:handle_pr"]}
> yes
{"kind":"request","key":"iter[1].handle_pr.draft","item":{"index":1,"count":3},"instruction":"Open https://example.com/pr/2 and check whether it is a draft.","response":{"name":"draft","type":"boolean"},"trace":["main","iter[0]:handle_pr","iter[1]:handle_pr"]}
> yes
{"kind":"item_done","key":"iter[1].done","item":{"index":1,"count":3},"instruction":"Stop. The pull request https://example.com/pr/2 is a draft. Leave it for its author.","trace":["main","iter[0]:handle_pr","iter[1]:handle_pr"]}
> yes
{"kind":"request","key":"iter[2].handle_pr.draft","item":{"index":2,"count":3},"instruction":"Open https://example.com/pr/3 and check whether it is a draft.","response":{"name":"draft","type":"boolean"},"trace":["main","iter[0]:handle_pr","iter[1]:handle_pr","iter[2]:handle_pr"]}
> no
{"kind":"request","key":"iter[2].handle_pr.approved","item":{"index":2,"count":3},"instruction":"Review https://example.com/pr/3 and post one review.","response":{"name":"approved","type":"boolean"},"trace":["main","iter[0]:handle_pr","iter[1]:handle_pr","iter[2]:handle_pr"]}
> no
{"kind":"item_done","key":"iter[2].done","item":{"index":2,"count":3},"instruction":"Ask the author of https://example.com/pr/3 to answer the review comments: Bump the logger.","trace":["main","iter[0]:handle_pr","iter[1]:handle_pr","iter[2]:handle_pr"]}
> yes
{"kind":"done","instruction":"Handled 3 items.\n0: Merge https://example.com/pr/1. Then delete its branch.\n1: Stop. The pull request https://example.com/pr/2 is a draft. Leave it for its author.\n2: Ask the author of https://example.com/pr/3 to answer the review comments: Bump the logger.","trace":["main","iter[0]:handle_pr","iter[1]:handle_pr","iter[2]:handle_pr"]}
```

The status of a live run, after item 0 ends and item 1 waits on its draft question:

```json
{"run":"run-1","key":"iter[1].handle_pr.draft","kind":"request","item":{"index":1,"count":3}}
```

Each answer changes one key only. The bad value `maybe` would give an error step, as it does now, and the run would wait on the same key.
