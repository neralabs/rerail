# 0106: A `.rr` program's tree patterns run as the transducer's matchers

Status: proposed (2026-11-10)

## Intro

A rule can ask a tree question in two ways:

- **As joins.** `kind(c, "call"), field(c, "function", f), kind(f, "attribute")` is evaluated by the native join
  engine, one index lookup per atom and per binding.
- **As a matcher.** `ast(d, c, {"language": "python", "rule": {…}})` holds an ast-grep-style rule. The slice
  pipeline already compiles such rules into the transducer's programs, the label passes and root programs of
  decision 0011, and runs them over a batch's tapes in one pass.

The `.rr` corpus engine had no executor for `ast` atoms. The planner refused them ("no executor … yet"). So every
tree question in a `.rr` program was answered by joins, and the transducer's matching, the core of the slice
pipeline, never ran for `.rr` rules.

This record gives the corpus path that executor (§1–§3), then lowers the tree-shaped parts of ordinary `.rr` rules
into matchers (§4).

## Decision

### 1. An `ast` atom reads a relation the transducer fills

Before the program is lowered, `logic::plan::corpus::matchers::lower` rewrites each `ast` atom without captures:

```text
// as written
MATTR(c) :- ast(d, c, {"language": "python", "rule": {"kind": "call", "has": {…}}}).
// as the planner sees it: one input relation per distinct matcher
MATTR(c) :- rr_ast_0(c).
```

- `docof(n, d)` is added only where the rule reads the document `d` anywhere else.
- The relations and their matchers' text are part of the prepared program (`prepared_matcher` rows). A program kept
  before this record has none.

### 2. Each chunk runs the matchers over its tapes

- **Compiled once a process.** Each matcher compiles as a helper policy's forbid clause, as `waves::policies` does
  for authored atoms (`policies::lower_matcher`). The program's first `.policy` supplies its `define` clauses, so
  utils resolve. The lowered matcher and its path and text gates are kept by the matcher's text.
- **Tapes are kept.** A file's tape used to be dropped once its facts were read. It is now kept beside its document
  (`Document::tape`), but only when the program has matchers.
- **One pass per matcher.** For each chunk, `facts::vtab::syntax::with_matchers` runs every matcher's label passes
  and root program over the chunk's tapes (`LoweredMatcher::roots_batch`). That runs on Metal where the run is
  bound to Metal, otherwise on the CPU.
- **Matches become nodes.** A match is a tape preorder id, which is the core id. It maps through the document's
  projection to the batch's node id. A node the projection leaves out has no row.
- **Not covered.** A guest, a text-only file, or a document a matcher's gates exclude has no rows.

### 3. What does not run yet

- **Captures.** An `ast` atom with captures (`NAME: n`) is left as written, and the planner still refuses it. The
  transducer's root programs return the root only (`Builder::roots` captures the start node). Captures need a
  capture output from the root programs, in transducer code.
- **Matchers the transducer refuses.** A matcher it can't lower fails the run and names the reason. An example is a
  metavariable bound twice ("shared binding"), which the slice path refuses too.

### 4. Tree-shaped parts of rules become matchers (the next step)

`kind(r, "K"), field(r, "F", x), kind(x, "K2")` becomes the matcher
`{"all": [{"kind": "K"}, {"has": {"field": "F", "kind": "K2", "stopBy": "neighbor"}}]}`, but only where `x`
appears in no other literal. Several such fields on one root join one `all`. Only Python programs are rewritten.

Parts that use the nodes they bind later need captures. So do existence tests on a field with no kind: a field
child the projection leaves out (an unnamed token) is found by a matcher but not by `field`. Both stay as joins.

## Why

- **The matcher pipeline is the engine's matching.** Tree patterns should run as the transducer's passes over the
  tapes, as the slice pipeline's clauses do, not as one join step per edge. The joins keep what only they can do:
  qualified names, bindings, aggregates and cross-file rules.
- **An executor before the lowering.** Without one, `ast` atoms in `.rr` programs could not run at all. With one,
  lowering a rule's part is just a rewrite into an atom the engine already runs.

## Evidence

- `corpus::matchers::tests::ast_atoms_read_their_matchers_relations`: one relation per distinct matcher; `docof`
  only where the document is read; an atom with captures left as written.
- A test pack asking each tree question twice, as a matcher and as joins, over the ruff fixtures (486 files):
  - attribute calls: 1,353 both ways;
  - identifiers directly in `parameters`, with the document read: 534 both ways;
  - identical on `--metal` and on the CPU.
- The two matchers' passes: 22 ms of CPU over 211,000 tape nodes.
- The Python pack (no `ast` atoms, so no tapes kept): violations and policy rows identical, 28.7 s CPU (26.2 user +
  2.5 sys) at load 85.
- The data-layer programs' `ast` atoms: 248 in 21 files, about 40 with captures. Those programs are written over
  `prelude.rr`'s relations, not `core.rr`'s, so running them in the corpus path is a port of its own (Open). One
  run reached its matchers and stopped on a "shared binding" refusal.

## Prior work consulted

- **0011:** composite matchers as node-label passes plus walking programs; this record runs them from `.rr`.
- **0024:** a chunk's matching as one wave. Here it is one pass per matcher (Open).
- **0036:** the rule language's built-ins made by the transducer; the axes (0105) and now the matchers.
- **`waves::policies`:** authored `ast` atoms compiled as helper clauses. This record reuses its compile path.

## Open

- **One wave and one launch.** The matchers of a chunk run one `roots_batch` each, not one `LoweredWave` launch. They
  are neither priced nor placed by the executor (`Kernel::Program`), and the Metal roots record no device time in
  the profile.
- **Captures,** as in §3.
- **Memory.** The kept tapes, and the label bitsets made per pass (the profile's high-water mark rose 276 MB in the
  matcher span on the 486 files), against the 1 GB budget.
- **Memo hits.** A file read from the per-file memo has no tape. Its file-local rows come from the memo, but a
  cross-file rule reading a matcher's relation over it relies on the memo holding those base rows.
- **The data-layer programs** in the corpus path, over `core.rr`'s relations.
