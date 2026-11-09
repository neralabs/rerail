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

Steps 1 and 2 are done. Their results are in §5 and §6.

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
