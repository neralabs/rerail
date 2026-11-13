# 0109: Long text twins, tree ancestry for frames, and what the symbol-graph ports need

Status: proposed (2026-11-10)

## Intro

Decision 0108b left three engine defects open:

- A rule reading the text of a node whose kind its body does not fix
  still saw the 256-byte bound.
- `@rx` applied to a node, not a string, gave no rows, silently.
- A run without `RERAIL_TAPE_CACHE` could reuse an earlier run's results.

This record fixes all three. It also explains why
`execution_authority_bypass` was slow and fixes its port, and it records
exactly where the remaining data-layer programs stop.

## Decision

### 1. Long text: kinds through relations, then twin rules

The kinds whose text a rule reads now follow derived relations to a
fixpoint. A column of a derived relation holds the kinds its rules give
the head variable there. For example, `nodes(n)` defined from
`kind(n, "call")` carries `call` into every rule that reads `nodes(n)`.

That still leaves rules whose node can be any kind, such as the text of a
`field(c, "function", f)`. Making every node's text at any length was
measured on a repository sample (one file in 18) and rejected:

| projection | instructions | store |
|---|---|---|
| bounded (the old behaviour) | 97.2 G | 28.8 MB |
| every node at any length | 137.6 G (+41%) | 54.4 MB |
| twins (this decision) | 101.0 G, profiled | 27.8 MB |

Instead, each such rule gets **twins**. A twin is the rule with that
`text(v, t)` replaced by a call that reads only the nodes the rule binds:

```text
// the rule as written: text within the bound
calls(n, t) :- kind(n, "call"), field(n, "function", f), text(f, t).
// its twin: the text of the same f when it is past the bound
calls(n, t) :- kind(n, "call"), field(n, "function", f), t = @rr_node_text(f).
```

`@rr_node_text(f)` gives the node's text only when it is longer than the
bound, so the rule and its twin never both hold for one node. It reads the
text from the batch holding the node's document; a weak registry by
document key finds that batch.

A rule with k such atoms gets 2^k − 1 twins. Past three atoms the run falls
back to projecting every node's text. No twin is made for:

- a field whose every kind is a leaf, by the grammar
  (`field(x, "attribute", v)` is an identifier, whose text is always held);
- a short constant (`text(v, "doc")`).

Over the large test file, the twins give the same rows as projecting every
node's text (1,695 nodes over 300 bytes).

### 2. A text test on a node tests its text

`@rx(w, re) = 1`, with `w` a node (a variable bound in a node or loc
column), read the node's id as a string. It is now rewritten to
`text(w, t), @rx(t, re) = 1` with `t` fresh, and so is every other text
test. The twins of §1 then cover long nodes.

### 3. Run reuse is keyed on every setting

The reuse key held four `RERAIL_*` settings. Any other setting that
changes the plan, such as `RERAIL_AST_PARTS`, reused results made under
another value. The key now holds every `RERAIL_*` name and value, except
the cache's own path.

A profiled run, or one that asks for a `RERAIL_DUMP_*` file, does not
reuse, so its side output is written. The matcher dump is written whether
the prepared program was made or kept.

### 4. Why `execution_authority_bypass` was slow

Over the repository (7,416 files, 23.9 M nodes) the run took 41.1 s wall
and 75.9 s CPU, for a check that finds nothing. The profile split it into
three parts:

- **Parsing:** 27.7 s CPU and 279 G instructions. This is measured with a
  program that reads only `node`, and every check pays it.
- **The port's frame ancestry:** most of the rest.
- **Text projection and the matchers:** what remains.

The port copied the SQL's `parents`: the frame `p` whose span contains
`f`'s, with the greatest start.

```text
frame_ancestor(f, p) :- frames(f, _, fe, _, _), docof(f, d), frames(p, _, pe, _, _), docof(p, d),
    start(f, fs), start(p, ps), ps < fs, pe >= fe.
```

The plan read every frame of the chunk for each frame before it tested the
document: 34,648 rows became 45.9 M. The depth counted over these pairs is
the depth the qualname fold already computes.

The fold had its own fan-out. It stepped from a frame to every node whose
nearest frame it is (6.8 M rows), and only then kept the frames.

Now both follow the tree:

- `dl_frame_parent(f, a)` is each frame's nearest frame ancestor, read
  from the frame.
- `@qualdepth` sits beside `@qualname` in the converter.

`contexts` (166,109 frames), `sites` (18) and the findings (0) are
identical over the repository.

| port | wall | CPU (user + sys) | instructions | peak RSS |
|---|---|---|---|---|
| span pairs | 41.1 s | 75.9 s | 765 G | 1,361 MB |
| tree ancestry | 18.1 s | 51.8 s | 545 G | 922 MB |

Both runs were at load 33–86, with a CPU-only engine and a fresh tape
cache. The rule work on the main thread fell from 25.3 s to 4.7 s.

The largest remaining program cost is the bounded `text` projection,
7.9 s CPU. It interns 5.2 M strings because `dl_frame` reads the text of
`name` fields. The projection makes `text` for every node, not only for
the kinds that are read (open item).

### 5. `serverplan_quality`

`file_content(d, s)` is now the text of core's `source` span, which holds
the whole file at any length.

`@substr_bytes` is `sql_substr_bytes`: SQLite's
`substr(CAST(x AS BLOB), y, z)`, NULL for an empty text as SQLite gives
it. It is checked against SQLite.

The port now stops at a matcher whose `filePath` test sits below its root
conjuncts: inside a util it `matches`, then inside an `any` and an
`inside`. Only root conjuncts become document gates (open item).

### 6. What the symbol-graph ports need

`public_api_triples` and `internal_dead_code` read `module_of` and
`reference_edge`, and their converter prelude lacks `path_module`.

**`module_of` is not `doc`'s module.**

- `core.rr`'s `doc(d, path, module)` is the whole path with dots, and
  `a/__init__.py` gives `a`.
- W's `file.module` climbs only through directories that hold an
  `__init__.py`. `pkg/sub/__init__.py` gives `pkg.sub.__init__`, and a
  file outside any package is its stem.

A faithful `module_of` needs the package directories: the documents named
`__init__.py`, or the disk.

**`reference_edge` is W's references graph.** Its edges join two
definitions:

- the source is the function or class holding a reference;
- the target is a definition in any file.

The references are resolved through scopes and imports, with `self.`
and `cls.`, the `siblingLinks` (decorated `_lazy` twins), and linking
across files. No `.rr` library computes this yet. The nearest are
`python/imports.rr` (modules, aliases, exports) and
`data-layer/purity-calls.rr` (calls by name).

## Open items

- **Bounded text for read kinds only.** Make `text` only for the kinds and
  fields that a rule reads, as §1 does for long text. In
  `execution_authority_bypass` that is 7.9 s CPU.
- **A `filePath` test below a matcher's root conjuncts** (in a `matches`
  util, `any`, `not` or `inside`): lower it as a per-document constant.
- **`module_of`** from the package directories, as in §6.
- **`reference_edge`** as `.rr` over the import and scope relations, or as
  W's graph supplied as a base relation.
- **serverplan_quality's probes** need a port per entry-point case
  (0108b).
