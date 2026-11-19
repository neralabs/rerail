# Tool calls as an input of `.rerail`

Status: proposal, in use by the engine; needs the review of the engine's author.

A JSON tool-call rule ([Writing command-line rules](../tool-call-rules.md)) matches a coding agent's tool call, not
a file: `match` on `input.command`, `sample: { "source": "tool:*", "boundary": "tool-call" }`. Until this change the
rule language had no input for a tool call, so `rerail-engine translate-json` made a policy with no rule and
`run-policies` refused it (`no .violation declarations`). This page is the language's side of it: two core
relations, one scope, a finding at a call, and a test form.

## The input: two core relations

`core.rerail` declares what a run is given of each call, the fields a JSON tool-call rule reads (`rules.rs`,
`Field`):

| Relation | Rows | The JSON rule's field |
|---|---|---|
| `tool_call(call: symbol, tool: symbol)` | one per call: its id and the tool's name (`Bash`) | `sample.source` `tool:<name>`, `tool.name` |
| `tool_input(call: symbol, key: symbol, value: symbol)` | each string argument, a nested one by its dotted path (`command`, `options.branch`) | `input.<path>` |

As in the product, a missing or non-string argument has no row, no array is walked, and a key with a dot in it has
no row (no path names it). The rows are what the run adds to the programs that declare the relations (decision
0249, `corpus::added`), as `env` is the run's settings: never text, and part of every key the run makes, so a run
over other calls is another run.

## The scope: `.scope tool_calls`

A policy that reads tool calls says so:

```
.scope tool_calls
```

- Its run reads no file: its documents are none (`doc_scopes` with no entry), whatever the root holds.
- Its `.violation`s are at a call: `at: symbol`, the call's id. A finding has path `""`, line 0, and `at` the id. Its
  message may name the id (`{at}`), and no property of it: a call has no line or name.
- Lowering refuses a policy that reads `tool_call` or `tool_input` without it (a JSON rule that reads `input` must
  sample `tool:*` or `tool:<name>` at `tool-call`), an `at: symbol` without it, a node or span `at` with it, and
  another `.scope` beside it.

A JSON rule's regex searches (`Regex::is_match`) and `match` is whole-string, so a search for `R` is
`match("(?s:.*)(?:R)(?s:.*)", s)`: R's anchors still read the whole argument, and a newline is any character
around it.

```
.policy "no-force-push" revision 1
.mode steer
.scope tool_calls
.violation no_force_push(at: symbol) code "no-force-push" "Do not force-push: it rewrites published history."
no_force_push(c) :-
    tool_call(c, "Bash"),
    tool_input(c, "command", s),
    match("(?s:.*)(?:\\bgit\\s+push\\b[^\\n]*\\s--force(?:\\s|=|$))(?s:.*)", s).
.test caught
    .tool_call "Bash" `{"command": "git push --force origin main"}`
    .expect "no-force-push"
.end
.test passes
    .tool_call "Bash" `{"command": "git push origin main"}`
    .expect no "no-force-push"
.end
```

## Giving a run its calls

- `rerail-engine run-policies --tool-calls <calls.json> <root> <out> <policy.rerail>...`: `calls.json` is a JSON
  array of `{"id": "…", "tool": "Bash", "input": {"command": "…"}}`, a call as the product samples it (the tool's
  name and its arguments, `host-runtime.ts`). Each finding is `{"policy", "clause", "code", "path": "", "line": 0,
  "at": "<id>", "message", …}`. File policies in the same run read their files as before.
- The `toolCalls` option is the same array, for a caller that runs programs itself (`tools::run_program`). An option
  of another shape is an error where a run is first given its options (`corpus::run_with_options`, `run_read`), before
  any run reads it.
- In a test, `.tool_call "<tool>" <fenced JSON object>`: the test's calls, ids `"1"`, `"2"`, … in order; a fixture
  set may hold them too. `.expect CODE` and `.expect no CODE` check the findings; one at a call prints as
  `CODE at call "2"`. `rerail rules test <file>.rerail` runs them.

## From JSON

`translate-json` turns a JSON rule whose `sample.boundary` is `tool-call` into such a policy: its `match` the rule's
body over `tool_call(c, _)` (`tool_call(c, "<name>")` for `tool:<name>`), its `action.content` the message, its
`id` the code. It translates the forms the shipped tool-call rules use: `field: "input.<path>"` with `regex`, and
`all`, `any`, `not`; any other form is an error that names it.

## Not decided here

- How the product hands a session's calls to `.rerail` policies (it matches JSON tool-call rules natively today,
  `rules.rs`); this gives it the input, the finding and the CLI form.
- `tool.name` and the whole `input` as JSON, and `eq`/`contains`: no shipped rule uses them in a translation; the
  relations already hold the tool's name, and `=` and `contains` are the language's own tests.
