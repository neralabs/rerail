# 0110: Text only for the nodes read, `filePath` inside a matcher, and `module_of`

Status: proposed (2026-11-11)

## Intro

Decision 0109 left five open items. This record closes three of them,
names the one that needs a choice, and records where the last one stops:

- `text` within the bound was made for every node, when rules read the
  text of a few kinds;
- a `filePath` test inside a matcher (below its root conjuncts) did not
  lower;
- `module_of` had no producer;
- `reference_edge` has none yet (a choice, §5);
- serverplan_quality's probes cannot be compared while the program does
  not run (§6).

It also records two fixes from the merge of the integration branch.

## Decision

### 1. `text` for the nodes rules read

The projection made `text` within the 256-byte bound for every node. It
now makes it only for the nodes some rule reads:

- the kinds a rule fixes (`kind(n, "call")`, or through a relation's
  column, as decision 0109 follows them);
- the nodes in a field a rule names (`field(_, "name", n)`);
- a matcher capture's kind or field, read from the matcher.

```text
// the matcher fixes where NAME is: in the `name` field of its parent
raw_function(f, name) :- ast(d, f, {"rule": {"all": [
    {"kind": "function_definition"},
    {"has": {"pattern": "$NAME", "field": "name"}}]}}, NAME: n),
  text(n, name).
```

A capture's kind or field is the one on its pattern's object, or one beside
it in an `all`. An `inside`'s `field` says where the node holding the
`inside` sits, so it says nothing of the capture.

A rule that reads the text of a node nothing narrows widens the set to
every node. The profile's "nodes with text" line names that rule.
`RERAIL_TEXT_NODES=all` restores every node.

`execution_authority_bypass` over the repository (7,416 files), with the
outputs unchanged:

| projection | interned strings | projection CPU | peak RSS |
|---|---|---|---|
| every node | 5.2 M | 7.9 s | 922 MB |
| nodes read | 2.07 M | 6.3 s | 684 MB |

The whole run's CPU did not move beyond noise: 52.3 s, against 51.8 s and
55.2 s in two earlier runs, at load 65–80. Instructions were 546 G against
562 G for the old projection on the same day. What remains of the
projection is visiting every node (15.7 M scalar visits) to make `node`
and the identities. That is a separate step: push the rules' demand into
which nodes get rows at all.

The ruff pack reads text of nodes nothing narrows, so it is unchanged
(570 G either way).

### 2. A `filePath` test inside a matcher

Only a matcher's root conjuncts became document gates. A `filePath` test in
a `matches` util, an `any`, a `not` or an `inside` was refused.

It now lowers as a fact that holds for a whole document or for none of it:

- every node, where the document's path passes the test;
- no node, where it fails, or where the run gave no path.

It is computed once per document. Each wave gives its tapes their paths
(`TapeDoc::with_path`).

A test runs `{"any": [{"filePath": …}, {"has": …}]}` over two files. It
holds in the file whose path passes, and in the other only by the `has`.

### 3. `module_of` from the package directories

`module_of` is W's `file.module` (`python_module_path`), written in `.rr`:

1. The packages are the directories that hold an `__init__.py` (`""` for
   the root's).
2. Climbing from a package through packages, each package has a root, the
   highest package reached.
3. A file's module is its path below that root's parent, the file by its
   stem, with dots: `pkg/sub/__init__.py` is `pkg.sub.__init__`.
4. Outside a package, or where the climb reaches the root's own package,
   the module is the file's stem.

The climb runs over the package set only. A recursion that builds the
module string as it climbs is refused by the engine (its values grow
without moving along the tree), so the path is cut once at the end.

Over the 3,110 documents in the repository's scope, `module_of` equals
`python_module_path` on disk for every one (3.1 s wall). The documents are
the policy's scope, so a package marker outside the scope is not seen; in
this repository none mattered.

The converter's shim adds `module_of` and `path_module`, and
`prelude.rr` defines `module_of` in place of an input nothing filled.

Two smaller gaps came up on the way:

- **`m = @dot(q, x)` with `m` bound** splits `m` at each dot. The engine
  runs it (`cmp_binds`), but the `.rr` checker called it ungrounded. It
  now agrees with the engine.
- **`@char(n)`** is `sql_char`, checked against SQLite's `char`.

`public_api_triples` now runs over the repository: 0 findings, as its
oracle (9.3 s wall, 34.9 s CPU, 1.30 GB peak RSS, over the 1 GB budget).

### 4. Merge fixes

- **Frames are kept per phase.** The hub kept a program's frame relations
  in a space per program hash. An edited program then started from no
  frame, and two programs could not share a relation. The space is the
  phase's again.
- **Answered world facts reach their readers.** A world relation with
  program facts (`file_line`) is held by the native engine, and its table
  is made only when first needed. Its answers were inserted into a table
  that did not exist. They are now written after the relation moves to its
  table, which is made first.

## 5. `reference_edge` needs a choice

`internal_dead_code` reads `reference_edge`: W's references graph, built
by its linker (`calls::Graph::reference_rows`). Each file is summarised
into definitions and the names it refers to. The summaries are then linked
across files by qualified name, with `self.`/`cls.` and the sibling links.
Two ways to supply it:

- **`.rr` over the import and scope relations.** This is the engine's own
  shape, maintained by the same fixpoint. It means porting the linker:
  per-file definitions and references (the typing programs have `def`,
  `binds`, `resolves`), then cross-file linking by module and name, over
  `module_of` (§3) and `python/imports.rr`.
- **W's graph as a base relation.** This is faster to reach, but it puts a
  whole-corpus W computation on the engine's path, outside the fixpoint
  and its maintenance.

The recommendation is the `.rr` port: the definitions and the import
fixpoint already exist (phase 1), and only the linking is new.

## 6. Where serverplan_quality stops

With §2, `serverplan_quality` lowers past the nested `filePath`. It stops
at a `parse` predicate inside a matcher: a guest parse of an f-string
(`{"parse": {"grammar": "pythonFString", …}}`). Its probes, a port per
entry-point case, cannot be compared until it runs.

## Open items

- **Rows only for the nodes read.** Push the rules' demand into which
  nodes get `node` rows and identities (15.7 M visits in
  `execution_authority_bypass`).
- **`reference_edge`**, as §5 (a choice).
- **A `parse` predicate inside a matcher** (`serverplan_quality`), then its
  probes per entry-point case.
- **`public_api_triples` at 1.30 GB** peak, over the 1 GB budget.
