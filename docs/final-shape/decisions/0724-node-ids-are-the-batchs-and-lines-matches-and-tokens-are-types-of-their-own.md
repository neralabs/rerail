# 0724. Node ids are the batch's, and lines, matches and tokens are types of their own

Status: accepted (2026-11-27). The user's model and choices: "32 bit ids. 2 bits for kind. One of four schemes";
"embedded documents can be small or large and sometimes numerous so we will need something flexible for those"; "I
don't want things being coerced. If we need to coerce a node to a line number or back, that is a function"; on tokens,
"we already removed tokens from nodes already. We are not going back on that. Have a separate token type, and
functions to convert between tokens and nodes"; on the names, "make the function names longer, more consistent, and
match souffle where there is a precedent there". The page size, the reserve and shared projections (§2) were left to
me ("sure, your call"). The function names in §4 are mine, written to that direction.

It replaces the plan of 2026-11-26 for chunk node numbers (a document base plus a local id in one 32-bit space per
chunk, roles as regions of ids) and its run-wide numbering. Steps 1 and 1b of that plan (897ba03ccd, 1a97a41368,
1a5a0bd3ae) stand: the ids' encodings defined once, and the check that refuses a shift by 32 elsewhere. It amends
0431 (embedded code takes room in its batch by its size) and keeps 0677 (no node id crosses into the cross part).

## Intro

Node columns are to be 4 bytes. Since 0677 a node id never leaves its file's part, so it needs meaning only inside its
batch. A node id is then its document's slot in the batch and its tape preorder number, read straight from the tape.
Lines, matches and the tokens the tape leaves out are not tape nodes. Today they share the node's id space as spans,
and core's `.type loc = node | span` lets one column hold either. A survey of the rule tree (RR201, §5) finds 95 head
arguments that put a line, a token or a match into a column that holds nodes elsewhere. This decision gives each its
own type, and makes every crossing between types a function a rule calls.

## 1. The id

- 32 bits: the slot (the document in the batch), the preorder number, and 2 kind bits.
- The split is one of four schemes, a constant of the batch passed to every launch and to the CPU executor, never
  stored in an id: 2, 6, 10 or 14 slot bits against 28, 24, 20 or 16 preorder bits.
- Kind 0 is the node, 1 the gap before it, 2 its end point, 3 a zero-width position (as after a node's last child).
  `end_point` has no ids of its own.
- The batch's directory has one row per slot, in the batch's block: the document, its tape's home, the first preorder
  number, and a page table for a paged slot. A tape read loads that row anyway, so finding the document costs no extra
  load.
- Comments and the tokens the tape stores are tape nodes, kind 0.

## 2. Embedded code and room in a batch

- A program over half a slot takes whole slots, like a file. A smaller one takes pages of 256 preorder numbers in a
  paged slot, at a second load.
- The chunker classes a file by the largest of its nodes, lines, tokens and largest program, and cuts each class by
  nodes and by slots. Files and programs take at most 15/16 of a batch's slots. The rest, at least one slot, is for
  tape values and the programs a rule asks for (Vue's view and expressions), which a cold run can't count before it
  runs. A warm run counts them from the documents table.
- A batch that runs out of slots stops with an error naming it. The corpus phase cuts its chunk in two and runs both
  halves again.
- A projection holds preorder numbers only, never a slot. A batch puts its own slot in through its directory as it
  reads, so a projection shared between batches (0616's renewed batch, 0683's corpus batch) is right in each, with no
  numbering of its own.

Measured on m3: 99.895% of files are under 65,536 nodes (scheme 3). Python's string annotations use 92 scheme-3 slots
per million host nodes at 256-node pages, Markdown's inline text 215, and the Vue pack's programs 293 (1,455 files,
22,468 programs, 87% made when a rule asks), against a reserve of 1,024.

## 3. The types

| Type | What it is | Its 32 bits |
|---|---|---|
| `node` | a tape node, or a position on the tape by its kind | slot, preorder, kind |
| `line` | a line of a document, or its whole source | slot, line number, kind |
| `match` | a row of the batch's match relation (slot, start, end): a derived fact, written once (0353) | a row number |
| `token` | a token the tape leaves out, numbered per document in source order | slot, ordinal |

Tokens stay off the tape. On a 1% sample of the mirrors, left-out tokens are 31% as many as stored nodes, and 15% of
the gaps that hold any hold two or more, so a gap's kind can't name each one.

`loc` goes. Every column has one type, and a column that would get two is a compile error. Nothing converts
implicitly.

## 4. The functions

The pattern follows Soufflé's `to_number` and `to_string`: a conversion is named `<from>_to_<to>`, and takes its
input first and its output last. A conversion is a relation, since it can fail or give several answers. A reader of a
type other than `node` is named `<type>_<what>`. The node's own relations keep their names (`text`, `kind`, `field`,
`child`, `docof`, `within`, `precedes`). Soufflé's `match(re, s)` stays as it is, over a symbol.

| Function | Was | Gives |
|---|---|---|
| `node_to_line(n, l)` | `line(n, l)` (a number) | the line n starts on |
| `node_to_end_line(n, l)` | `end_line(n, l)` | the line n ends on |
| `line_to_node(l, n)` | (none) | each node that starts on l |
| `node_to_first_token(n, t)` | `first_token` | n's first left-out token |
| `node_to_last_token(n, t)` | `last_token` | n's last |
| `node_to_token_before(n, t)` | `token_before` on a node | the token just before n |
| `node_to_token_after(n, t)` | `token_after` on a node | the token just after n |
| `token_to_node(t, n)` | (none) | the innermost node whose span holds t |
| `match_to_node(m, n)` | (none: a match was used as a node) | the node whose span is exactly m; none when no node is |
| `node_to_end_position(n, p)` | `end_point` | n's end point, a `node` of kind 2 |
| `node_to_gap_before(n, p)` | (none) | the gap before n, a `node` of kind 1 |
| `node_start_column(n, c)` | `col(n, c)` | n's 0-based byte column |
| `line_number(l, k)`, `line_text(l, s)`, `line_document(l, d)` | `line`'s number, `src_line`'s text | |
| `document_line(d, l)` | `src_line(d, k, sp)` | each line of d |
| `document_source(d, l)` | `source(d, sp)` | d's whole source, a `line` of its own kind |
| `token_text(t, s)`, `token_kind(t, k)`, `token_document(t, d)` | `text` of a token | |
| `token_previous(t, p)`, `token_next(t, x)` | `token_before`, `token_after` on a token | |
| `match_text(m, s)`, `match_start(m, b)`, `match_end(m, b)`, `match_document(m, d)` | `text` of a span | |
| `node_regex_match(re, n, m)`, `line_regex_match(re, l, m)` | `match_all` over a node or span | each match of re in the subject's text |
| `node_regex_group(re, n, m, name, g)`, `line_regex_group(re, l, m, name, g)` | `match_group` | the named group of match m |

A regex's named group over a symbol (`match(re, s)`) binds a symbol, as in Soufflé. Over a node it is a `match`, taken
through `node_regex_group`.

## 5. Finding and fixing the rules that mix types

`docs/rule-syntax/packs/rr/rr-node-span-columns.rerail` (RR201) reads each program's declarations and rules, through
every rule's head to a fixpoint, and reports a head argument that puts a line, a token, an end point or a match into a
column that holds nodes elsewhere. Over `docs/rule-syntax` it finds 95: 80 matches (python-main's capture groups, and
keyword captures such as `module_var`), 11 tokens (the TypeScript `add_*` rules, Rust's `elem_loc`, `member_head`) and
4 lines (`fn_head`, D412's `doc_at`, `return_head`, `no_unexpected_multiline`). Each is rewritten to the functions of
§4, with a `fix` in the policy wherever the rewrite is exact. Once they are, the lowering's type check refuses a mixed
column, and RR201 stays as the lint that names the site.

## 6. Leaving the batch

A node leaves as its document's key and its preorder and kind, `key << 32 | preorder-and-kind`, in findings, outputs
and the place output. A tape value's node leaves as its tape's key and preorder, placed through its string (0431 §3).
Kept per-file facts are kept in that form, and a batch that reads them puts its own slot in as it reads.

## 7. Order

Built straight through, with no compatibility kept: the types and functions (§3, §4), the 95 rewrites (§5), lines,
matches and tokens out of the node space, the batch ids (§1), the chunker (§2), leaving (§6), the 4-byte node columns,
and the bench (300 files on the M1 Max, CPU: 463 violations and the same relation rows, then CPU, wall and peak).
