# T30: named-only trees in memory (RER-131 option B)

## Intro

A v12 tape stores a syntax tree's named nodes and leaves out the tokens
the grammar implies: keywords, punctuation and operators (t27). Today
restore rebuilds those tokens and holds the full tree in memory. So the
store got smaller, but memory did not change.

Option B keeps the tree named-only in memory too. Everything sized per
node then shrinks to about 0.57–0.68× (t27 §9):

- the restored records;
- the facts columns;
- the label and text bitsets;
- the prefilter rows;
- the device's slabs.

Restore also gets cheaper, because there is no unfold. This note fixes
the design and the order of work.

## 1. The design

There is one node space, and it is named-only. Every preorder index,
in the host's records, the transducer and the device, counts named
nodes plus the few tokens the fold kept. Tokens are reached through
their parent.

- **The node API serves tokens as virtual nodes.**
  - `TapeNode` (ast-grep's `Node` over a tape) is either a record or a
    recognized token.
  - A token carries its own kind, field, span and slot among its
    parent's children.
  - `children`, `dfs`, `next`, `prev`, `next_all`, `prev_all`, `field`
    and `parent` answer exactly as the full tree does.
  - A token's `node_id` has bit 62 set, above every record index.
  - So ast-grep patterns, `.rr` `~` patterns, the native analyses and
    the tree API (`code_rule_dsl::tree`) read the same answers with no
    change.
- **Tokens are recognized per parent, on demand** (`TokenGrammar::sequence`).
  - It is the same function the decoder uses, so the view cannot
    disagree with what the fold proved.
  - Nothing is cached. A token costs memory only while a reader holds
    it.
- **Preorder readers get a token API on the record.**
  - `transducer_parts(node)` gives the children in source order with
    tokens.
  - `transducer_tokens(node)` gives the tokens alone.
  - Readers that need tokens ask for them. All others walk named
    records and are faster for it.

## 2. What reads tokens, and how each moves

These lists come from the two surveys in this session.

### Host readers through the node API: no change

- ast-grep pattern matching;
- the native analyses: scope, value, calls, matcher, literal;
- the `tree` API;
- `matcher/layout`, `relation`, `count`;
- Python's async detection through the node API.

### Facts, which `facts::core::from_tape` builds from every record

- **T shape:** `op`, `op_t`, `op_at`.
- **core.rr:**
  - `op`, `op_at`;
  - `text` of token spans;
  - `first_token`, `last_token`, `token_before`, `token_after`;
  - `tree_digest`, `def_digest`.
- **Elsewhere:**
  - the f-string `=`;
  - Python `async` in `call_batch` and `families`.

They move to `transducer_parts`. The `op` rows are built per node from
its tokens.

### Preorder state that is renumbered, with the version to bump

| State | Version |
| -- | -- |
| Prefilter rows | `PREFILTER_VERSION` |
| Search kind stream | `POSITIONAL_INDEX_VERSION` |
| Core facts derivation | `rerail.core/v2` |
| Python rows | `py` `VERSION` |
| facts-a | `SCHEMA_VERSION` |
| Wave memo | `SCHEMA` |
| Corpus memo | `SCHEMA` |
| Persisted node references | `NODE_REFERENCE_FORMAT` |

### Rules rooted at an anonymous kind

The kind postings and the prefilter list only records, so a rule
rooted at a token kind (`kind: "+"`) needs postings by token kind.
These are built when the prefilter rows are built, from
`transducer_tokens`.

### Node ids taken as preorder indexes

`origin::of_node` and guest extents, `matcher/memo`,
`relation::has_remembered`, `value/mod.rs:173`, `call_batch` and
`sql_backend`'s fact targets.

A token's id maps to its parent's record plus a slot. These sites map
through `TOKEN_ID_BIT`, or refuse a token id where it can never occur.

### The transducer and the device

The transducer and the device read mode-0 walks (`CHILD`, `NEXT`,
`FOLLOWING`, `FIRST`, `LAST`, `DESCEND` with `named=false`) as "any
child, tokens included". Every backend does:

- the packed CPU interpreter;
- the Cranelift JIT;
- the Metal interpreter;
- Metal JIT codegen;
- about a dozen hand-written kernels.

Most mode-0 walks do not care about tokens. Their target test (a named
kind, a field, a capture) can never match a token. So the change goes
in the compiler, not in every backend:

- **A mode-0 walk whose target cannot match a token** becomes the named
  walk. It is the same answer.
- **Adjacency, where a token in between matters**, gets a guard. This
  covers `NEXT 0` (an immediate sibling), the first `CHILD` of a
  pattern, `FIRST`/`LAST 0`, and `stop: neighbor`. The guard is a new
  op, `NO_TOKEN_BETWEEN`.
- **A real token test**, mode 0 then `KIND <anonymous id>`, becomes a
  new op, `TOKEN K`. It tests the token column at a sibling position.
- **Pattern candidate lists** (`PackedPattern::children`, Metal
  `label_patterns`) and `exact()` read the merged parts.

The two new ops read a per-document **token column**: per record, an
offset into a list of (kind, field, start, end) entries, at about 12 B
per token. It is built when a document is admitted, and only for
programs that use the ops. The device gets the same column in its slab.

A token costs about 12 B in that column, against about 48 B in a full
record plus slab, facts and bitset rows today.

## 3. Order of work

Each step is committed on its own and checked with named tests and S7
(519 keys, four backends).

1. **Node API with virtual tokens, and a parity check.**
   - `TapeDoc::restore_named`.
   - A test checks, node by node, that a named-only document answers as
     the full document does: kind, span, flags, children, next, prev,
     next_all and prev_all counts, and every field. It runs on
     snippets and on the corpora.
2. **Facts from parts.** `from_tape` builds the token relations from
   `transducer_parts` and takes no tokens from records.
3. **Compiler lowering.** Mode 0 becomes named, `NO_TOKEN_BETWEEN` or
   `TOKEN K`. The pattern candidate list and `exact()` move onto
   parts. Then the token column is built for the CPU backends, for the
   interpreter and the JIT.
4. **The device.** The token column goes into slabs, and the two ops
   into the Metal interpreter, codegen and kernels.
5. **Indexes.** Token-kind postings, then the version bumps above.
6. **Cut over.** Restore defaults to named-only, and the full restore
   becomes test-only. Then:
   - measure process memory and CPU on S7 and the data-layer repository;
   - remove `named_only`'s host-image detour on the device, since
     slabs can read named-only bytes in place.

## 4. Status

All six steps are done. Steps 1 and 2 are in §5 and §6, step 3 in §7–§9,
and the cut-over (steps 5 and 6) in §10. Step 4's fact rows and root
programs on Metal are open items (O-metal-fact-rows, O-metal-root-programs).

## 5. Step 1 result

The node-by-node parity check (`tape::tokens_tests::the_token_view_answers_as_tree_sitter_does*`)
compares a named-only document against the full one. Every file
answered alike:

| Language | Files (sample) |
| -- | -- |
| Python | 655 (1 in 20) |
| TypeScript | 380 (1 in 2) |
| TSX | 34 |
| JavaScript | 260 |
| Rust | 304 (1 in 4) |
| PHP | 105 |
| Vue | 9 |
| SQL | 169 (files under 60 KB, 1 in 25) |

Notes on the check:

- It compares fields and the next_all/prev_all counts on every
  seventh node. Everything else it compares on every node.
- The baseline is the full document, not tree-sitter's own tree. The
  two differ in one way on purpose: tree-sitter also finds a hidden
  child's fields on the parent (`type_query` answers `object`).
- The view is not tuned yet. Sibling steps rebuild the parent's parts,
  so `next_all` over a long list is quadratic. That is why the SQL run
  is sampled. It is a cost, not a correctness issue, and gets fixed
  with the readers that need it.

## 6. Step 2 result: facts

Facts share the document's node space. A named-only document's facts
hold its records, plus a token column: per record, the tokens the
records leave out, each with its slot among the record's children.

Every reader of tokens asks for them through one of these, the same on
both shapes:

- `Facts::tokens(id)`: the anonymous children in order, whether in the
  records or not;
- `Facts::parts(id)`: all children in order, tokens among them;
- `Facts::is_leaf(id)`.

These readers moved:

- T's `op_t`, `op_at` and `async`;
- core.rr's `op` and `op_at`;
- the token spans behind `first_token`, `last_token`, `token_before`
  and `token_after`;
- `tree_digest` and `def_digest`. A token outside the records hashes
  as a token record does.
- Python's f-string `=`, and the async context in `families` and
  `call_batch`.

Rules never saw token nodes. `Projected` numbers named nodes only, and
gives token spans ids past them. So rule-visible ids do not change.

Parity test (`facts::vtab::syntax::tests::named_only_documents_project_as_full_ones`):

- Inputs: the CPython corpus, plus TypeScript, JavaScript, Rust, PHP
  and SQL snippets.
- Facts from named-only documents give the same rows as facts from
  full ones, in every relation of both shapes (core.rr and T).
- Two relations are compared differently, by design:
  - `tape_of` names the facts by their digest, so it is left out;
  - `core_node`'s record index is compared by the record's span.

The facts module's own tests show no new failures. S7 is unchanged at
519 keys; production restores full documents until step 6.

### 6.1 A whole run on named-only documents

Test builds read `RERAIL_TEST_NAMED_IN_MEMORY=1`. With it set, every
named-only image is restored named-only, so a whole run shows what
still reads tokens from the records. It is a probe, not a mode.

S7 with the probe set:

| | Keys |
|---|---|
| Full documents | 519 |
| Named-only documents (interpreter and JIT) | 381 |

The missing keys belong to the packs' forbid-ast clauses: RET505–508,
E722, B006, B404, SIM118 and others. These clauses run on the
transducer, whose mode-0 walks still expect token records. Step 3
fixes that.

The device refuses the named-only document ("noncanonical tape image
header"). Step 4 fixes that.

## 7. Step 3a: the CPU walk reaches tokens

### Why tokens are walk nodes, not two new ops

§2 planned two ops, `NO_TOKEN_BETWEEN` and `TOKEN K`, for the walks
that care about tokens. The pack's forbid-ast clauses ask for more than
that:

- E722's root rule matches the `except` keyword itself, and its finding
  is anchored there.
- SIM118 steps from a node to the `.` before it (`follows` +
  `regex "^\\.$"`).
- E722 also steps to the `:` after the keyword (`precedes`).

So a token must be a node a path can stand on. TEXT and KIND test it,
PARENT leaves it, and NEXT reaches it.

### The design

A named-only document gets a **token column** (`tape::walk`). Per
record, it holds an offset into rows of (start, end, slot, kind,
field), 16 B a row. The rows are ordered by parent, in the records'
preorder, so the tokens below a record are one range.

`TapeDoc::walking_tokens()` returns the document as a walk sees it, a
clone that shares the records, the source and the column. Its
`transducer_*` accessors number token `t` as node `records + t`:

- kind, field, flags and span read the row;
- parent finds the record by its offsets;
- children merge the records with the record's own tokens, by slot;
- descendants are two ranges: records
  (`transducer_descendants`), then tokens
  (`transducer_descendant_tokens`);
- `TapeDoc::node` maps a token id to the node API's token, so host
  fact labels can test it.

Every other reader of `transducer_*` ids gets a plain document and
reads records only. That includes facts, matchers, prefilters and stats.

### What changed in the CPU backends

- The waves and the catalog hand the transducer walking documents.
- The interpreter, micro walks and sets add the token range to
  DESCEND.
- The term index's text rows cover the records. A walking document
  refuses them, so text is scanned.
- Candidates are a stopgap until token postings (step 5). On a walking
  document, each token whose kind a clause's root program starts from
  is a candidate too. That is a superset, and the program tests each one.

### Result

| S7 | Interpreter / JIT keys | CPU |
|---|---|---|
| Full documents | 519 | 31 s |
| Named-only documents (probe) | 519, the same set | 69 s |

The full shape is unchanged on all four backends. Metal refuses
named-only documents until step 4.

The doubled CPU is the cost of these stopgaps:

- token candidates offered to every clause;
- sibling steps that rebuild the parent's child list;
- DESCEND lists that are collected, not walked lazily;
- text scans in place of index rows;
- the column built for every document.

Step 3b removes most of them. Mode-0 walks whose target cannot match a
token become named walks, and only clauses whose root can start at a
token get token candidates.

## 8. Step 3b, first part: what a walk over tokens costs

Each change below was measured on S7 with the named-only probe. The
interpreter found 519 keys, the full set, after each one.

| Change | Interpreter CPU |
|---|---|
| Step 3a | 69 s |
| Text bits: the index row for the records, scanning only the tokens | 48 s |
| `Domain::Records`, see below | 46 s |
| Children merged lazily by slot; NEXT by slot | 41 s |
| Full documents, same run window | 31 s |

**`Domain::Records`.** A kind-less rule that cannot match a token gets
this domain. `MatchTree::may_match_token` decides it, conservatively:

- a token has no children, so a `has` conjunct rules tokens out;
- a supertype rules them out too;
- `regex` and `not` may match a token.

Label passes with this domain never enter a token. Wherever a domain
only proposes starts, it is read as every node, which is a sound
superset.

**Slots.** The column holds each record's slot among its parent's
children, 4 B a record. So a walk merges a record's children with no
list, and steps to a record's next sibling with one binary search
among the parent's tokens.

**Tried and dropped.** Testing each token's text on its own, in place
of the batched scan, took 137 s. Text tests are built for the batch.

**What is left, about 9 s:**

- text bits for tokens, about 2 s: the batched scan still reads the
  whole source. Token postings (step 5) replace it.
- formulas and packed walks, about 0.6 s.
- the layer, about 0.8 s.
- token candidates, about 0.3 s.
- label passes over complements, which cover tokens.

## 9. Step 3c: rules range over named nodes; the walk column is gone

§7 and §8 let the transducer walk tokens through a column. That column
had to be built for every named-only document a wave reads. Step 3c
removes the need for it: rules no longer walk tokens at all.

- **Roots and relations reach named nodes only**, on the host matcher
  and on the transducer alike.
- **Tokens are tested by text** through a new builtin, `tokens`
  (`matcher::tokens`). Its places are `has`, `below`, `before`,
  `after`, `earlier`, `later`, `beforeCode`, `afterCode` and a field;
  `as` binds the token and `where` tests it.
- **The compiler keeps old rules exact.** A relation to a bare regex
  reached tokens too, so it now reads as both:

  ```yaml
  # written:            has: { regex: "^\\($" }
  # compiled, in effect: any: [ has: { regex: "^\\($" },        # a named child
  #                             tokens: { has: "^\\($" } ]      # or a token
  ```

- **Patterns** compare a named-only node's tokens by kind, recognized
  from the source as the pattern meets them
  (`TapeDoc::pattern_children`).

### How a `tokens` test runs without a stored column

The facts stage evaluates every `tokens` test of a tape before the label
passes. It builds a **per-tape token index** once (`TapeTokens`):

- one `parts()` call per named node, so each node's tokens are recognized
  once;
- token rows in the preorder of their owners, so a subtree's tokens are
  one contiguous range;
- each node's neighbours and its place among its parent's tokens.

A regex is matched once per distinct token text, and each test is then a
prefix-sum lookup per node. The index is dropped when the stage ends;
nothing is kept on the tape. A test with `as`, `where`, a count or a
field place still asks the matcher at the nodes the index cannot decide.

A `tokens` test also starts only at the kinds its enclosing `all`
requires, while the lowering stays on the same node.

### Removed

`tape::walk` (`TokenColumn`, `walking_tokens`, the merged children), the
token range in DESCEND (interpreter, micro walks, sets), the stopgap
token candidates, `Domain::Records` and `may_match_token`.

The facts layer keeps its own token column (§6). The SQL syntax tables
read it.

### Result

S7, interpreter, process CPU (user + sys). Both shapes ran back to back
in one window, at load average 27–149:

| Shape | Keys | CPU | Footprint |
|---|---|---|---|
| Full documents | 519 | 43.7 s | 208 MB |
| Named-only (probe) | 519 | 44.1 s | 192 MB |

The JIT agrees on both shapes. Before the token index, the probe took
266 s: a `below` test walked each subtree with `dfs()`, recognizing
tokens again at every node, for each of 154 kind-less facts. Narrowing
the tests to their `all`'s kinds left 113 kind-less facts.

Still open: Metal fails on named-only documents with `BackendFailure`,
and on the wave with `BackendLaunchLimit` (step 4).

## 10. Steps 5 and 6: one node space

There is one shape now. A tree-sitter tape is stored named-only and
restored as stored. The node API recognizes a parent's tokens when a
reader asks for them.

### What went

- **The full restore** (`restore_tree_as`, `Shape`, `restore_named`,
  the `named_only` field and its environment switches). Tests that need
  every node as a record build one for themselves
  (`TapeDoc::every_node_for_test`); they no longer pin the old restore.
- **The unfold on the read path.** `tokens::unfold` stays, as a test that
  the fold loses nothing.
- **The parts cache.** §1 says nothing is cached. `next_all` and
  `prev_all` now hold their parent's parts while they walk, and find
  their anchor by bisection over the parts' ends. A single `next` or
  `prev` recognizes its parent once.

### Stored formats

Every format that numbers nodes was bumped (§2): prefilter 7, positional
index 4, `rerail.core/v3`, Python rows 2, facts-a 2, slice memo 2, corpus
memo 6, `rerail.node-reference/v2`. No token-kind postings were added,
because no rule roots at a token any more (§9).

### Old rules kept exact

Named-only walks changed what two kinds of old rule meant. The compiler
now keeps them exact:

- **A neighbour is the sibling right beside the node.** A `precedes` or
  `follows` neighbour that walks named nodes stepped over tokens. In
  `a = 'x'; b = 'x'`, the `;` used to be `a = 'x'`'s neighbour, so
  `precedes: {kind: expression_statement}` failed; the named walk found
  `b = 'x'`. A named neighbour now also needs no token beside the node
  (`TokenTest::none`). The token index decides it, and the SQL catalog
  reads it as a gate (`SemanticFactSource::Tokens`).

  ```yaml
  # written:            precedes: { kind: expression_statement }
  # compiled, in effect: all: [ precedes: { kind: expression_statement },
  #                             tokens: { none: after } ]   # nothing beside
  ```

- **A regex tested with more reaches tokens too.** `precedes: {regex:
  "^==$", precedes: …}` (Ruff's E712) found no token, since the regex
  was not bare (§9). The tokens it reaches by the regex are now tested by
  the rest of the target, at the token (`TokenTest::rule`).

The converter writes the same `.rr` for the data-layer pack as before:
the token alternatives are read on names, as it always read a kindless
regex.

### Other fixes

- Structural keys and named leaves count recognized tokens, as
  tree-sitter's tree has them. Structure reads are bounded by the nodes
  the node API serves, not by the records.
- `op_at`, `prev_token` and `token` read a node's parts. They had read
  the records alone, so they had lost the recognized tokens.
- The anchor indexes walk records and skip anonymous leaves.
- Required text: a token test inside the node needs its regex's text.


### Measured

Python pack, 30 files, interpreter, process CPU: 500 keys on every
change above. Without the parts cache it took 36.1 s (load 24–78). After
the neighbour fix it took 34.4 s (load 67–94). The full tree took 32.5 s
in an earlier window.

The data-layer repository at `2b1504ad`, through the corpus runner
(`measure_inside_three_ways`, way `baseline`), cold, at load 70–96:

| | Named-only (now) | Full tree (t10 §1, load ~25) |
|---|---|---|
| Files | 9,024 | 9,010 |
| Nodes held | 18.68M | 28.2M |
| Parse (worker) CPU | 88.4 s | 22.9 s |
| Batch (worker) CPU | 15.5 s | 8.8 s |
| Total CPU | 115.9 s | 43.8 s |
| Peak RSS | 857 MB | 316 MB |

The node count is 0.66× the full tree's, as t27 §9 predicted. The CPU and
peak RSS are not yet explained. The load was three to four times t10's,
and t10's run is from another build with another program, so this is no
same-window comparison. The next step is a full-shape baseline of the
same build and program in the same window, then a profile of the parse
phase (the fold, and the token recognition that facts ask for). S7 was
not rerun after these fixes.
