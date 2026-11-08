# 0081: `match_all` over nodes of a kind, and over another pattern's captures

Status: built (2026-11-08; the scan is per distinct text, not chunked yet, O122)

## Intro

Decision 0054 made `match_all("re", s)` produce spans when `s` is a document's `source`, one of its lines
(`src_line`), or a comment. Everywhere else the lowering refused it, because it could not tell which spans to scan.
Four Ruff ports stopped there:

```
// fast003: the {name} parameters of a route path, a string.
fast_param(d, k) :- route_path(d, s), match_all("\\{(?<k>[A-Za-z_]\\w*)(?::[^}]*)?\\}", s), text(k, x), ...
// ruf001-003: a confusable character in a string, a docstring or a comment.
conf(c) :- str_or_comment(s), match_all("(?<c>[ᴀ-ᴢ…])", s), ...
```

Here `route_path` and `str_or_comment` are derived relations. The nodes they hold are strings, or strings and
comments, but only the rules that derive them say so. This record makes the lowering follow those rules, and has
the batch scan exactly those nodes.

## Decision

### 1. A scanned column names what it holds

`front::SpanIn`, the description of the spans a pattern scans, gets two cases besides `Source`, `Line` and
`Comment`:

- `Kinds({k, …})`: nodes of these kinds.
- `Capture(pattern, group)`: the spans group `group` of `pattern` captured.

`rr::lower` infers them for every column of a derived relation, to a fixpoint (at most 64 rounds):

- `kind(y, "comment")` gives `Comment`.
- `kind(y, K)` gives `Kinds{K}`.
- `rx_span("p", _, _, "g", y)` gives `Capture(p, g)`.
- A derived atom passes on its column's description.

A column with several rules takes the union of their kinds; a comment among other kinds becomes the kind
`"comment"`. A column with no description is still refused, and the message now says to bind it with
`kind($v, "…")`. `program_facts` stores the two new cases (decision 0074).

### 2. A context that feeds a scan keeps its derived atoms

A helper that binds an aggregate's shared variables (`count : { match_all("re", w) }`) is built from the rule's
other literals. Its usual first choice is inputs alone, so in

```
ndig(t, n) :- wd(w), text(w, t), n = count : { match_all("(?<d>[0-9])", w) }.
```

the context was `rr_ctx(w) :- text(w, t)`. That holds every node, so the scan had no description. When the
context's variables are a `match_all`'s text (`Lowerer::scanned`), the inputs-only choice is skipped and the derived
atom `wd(w)` stays. For `count` and `sum`, the groups a `match_all` binds are also columns of the body's helper,
so each match is its own row: AB12 counts 2, not 1.

### 3. The batch scans each distinct text once

`CoreRows::matches` reads the nodes of every wanted kind off the tree in one pass. It orders the patterns so that a
pattern over captures runs after the pattern that captures them, and it keeps each pattern's captured spans by
group. Containers with equal text (`"/x/{id}"` twice) share one scan, and each gets its own spans. The bytes
scanned are reported as profile work (`Unit::Byte`).

Other built-ins inside an aggregate are now lowered there: `match_all` was printed as `match_all_`, and `unset` as
a variable.

## Measured

Each check is one `run-rr` over Ruff's own fixture, compared by line with Ruff's snapshot:

| rule | found | Ruff | missing | extra |
|---|---|---|---|---|
| RUF001 | 12 | 9 | 0 | 3: `−`, `ρ`, `∗`, which Ruff's test allows by setting |
| RUF002 | 2 | 1 | 0 | 1: the same |
| RUF003 | 5 | 3 | 0 | 2: the same |
| FAST003 | 64 | 31 | 0 | 33: a parameter with no default is not seen (O121) |

`match_all_over_nodes_of_a_kind_and_over_captures_gives_spans` covers strings and comments joined in one
relation, captures scanned again, `count` over captures, and two containers with the same text. The pack's
`ruf001`–`ruf003` and `fast003` now lower; `perf403` is still refused (O122).

## What this does not do yet

- **The scan is per container, not chunked.** A pattern runs once per distinct text of the nodes it scans, not
  in one pass over the file's chunks. For nested kinds, a byte can be scanned more than once. O122.
- **A column bound by any expression** (`store(t, e)` with `e` any node) has no kind to scan, and is refused
  (`perf403`). O122.

## Prior work consulted

- **Lessons ledger 16:** the cost is repeated scans over the same broad relations. Equal texts share one scan, and
  the description limits a scan to the kinds the rules name, not every node.
- **Lessons ledger 18:** use structured facts, not text tests over everything. The kind comes from the tree, and
  the regex runs only on what the rules select.
- **ast-index-spike design index §4b (text and regex predicates):** a selective regex scans the text it is given,
  then confirms by span. The spans here are the node's own bytes.
- **ast-index-spike history index T24 (DH L7673-7687, S250):** one regex or eight over the shared text cost the
  same; the scan is the cost. Hence one scan per distinct text, and the bytes reported.
- **Decisions 0041, 0042 §3, 0054 and 0074:** no per-node text scans, the file scan, `match_all`'s spans, and rules
  stored as facts.
