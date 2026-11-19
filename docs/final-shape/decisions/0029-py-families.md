# 0029: the `py.*` fact families

Status: accepted (2026-11-03, list step 25)

## Context

05 Phase 8 row 1 places S's Python producers (`facts/producers/python/`) out of `#[cfg(test)]`, to
be "fed from `prefilter/` candidate sets, not full tape scans", writing "`py.*` fact families with
derivation rows", keeping a55956e3's demand pruning. List step 21 left the data-layer rules' demand
waiting on them: a magic rule can only narrow a clause by reading what the wave has bound before the
batch, and the policies' rules read nothing else yet.

The copies are S's `call_batch::collect` (one pass over every tape row for kind and context
dispatch), `collect_arguments` (argument rows for the calls asked for only) and
`python_structure::collect` (the general walk). Their one caller was the test-only oracle
`findings::async_reference`.

t11 (`notes/t11-streams-since.md`, from N1562's plan `:129-161`) gives the target relations and two
rules:
- `py.call(doc, call, callee)`, `py.call_site(doc, report_node, call, ref_node)`,
  `py.exec_context(doc, node, owner, context_kind)` and
  `py.call_arg(doc, call, arg, ordinal, keyword_atom, spread_kind)`;
- "Each fact has one owner for its semantic identity and covered domain";
- "Empty demand means no work. Missing required demand is an error, not permission for a full scan."

## Prior work consulted

- **ast-index-spike** (`prior-work/ast-index-spike-design-index.md`):
  - §17: cross-file semantic resolution is out of the syntax layer; "semantic edges are a separate
    layer keyed by symbols" (D S020 L2049-2058). A semantic-fact channel `t_fact(fid, pre, fkey,
    fval)` is "sought like vidx" (D S079), and a fact-anchored query seeds from it (D S099
    L5474-5548). **Agrees:** the families are a separate layer keyed by the syntax layer's node ids,
    and a rule can seed from them.
  - §8, linter needs (LA 4 L166-212; LCS 5 L234-251): 77 single-file syntax relations and 12
    cross-file or call-graph relations. The call graph's edges are syntactic, and "the answer might
    legitimately be keep a derived edge table" (LCS 6 L268-272). **Agrees:** call sites, contexts
    and arguments are syntactic per-file facts; resolution and closure stay in the rules.
  - §8, precomputed containment and semantic booleans (`scope_id`, `in_loop`, …) were the linter's
    workaround for missing expressiveness (LCS L41-60). **Agrees, so the domain is narrow:** the
    families hold one context per *call*, not a scope per node.
- **Decision 0026 and the spike's D14** (per-node arrays persisted beside the tape, 88–93% of the store):
  **agrees.** The rows are not stored. A wave makes them in memory from the core facts it holds, as
  the core facts are made from the tape. What is stored is one derivation per document: its digest.
- **Lessons ledger:**
  - #11, narrow per-family tables: **agrees.** There are five narrow integer relations, not one
    wide row or an entity table.
  - #13, demand-driven extraction: **agrees.** A family is made only when a program reads it.
  - #14, per-file ownership: **agrees.** There is one derivation per document, re-run when its core
    name rebinds.
  - #16, fix candidate sets before fusing: **agrees.** The producer starts at the `call` postings.
- **P decision 0005** (named derived facts): **agrees** on these points:
  - a native producer is reviewed engine code;
  - the facts are named after concepts, not rules;
  - identity is by digest;
  - invalidation follows the dependency cone.

  **Departs** on caching. 0005 caches the rows per (digest, tape identity); here the rows are
  rebuilt (0026), and only their digest is kept.

## Decision

1. **Five families, rule-independent, all integers** (`facts::vtab::syntax::PY`, `PY_COLUMNS`):

   | Table | Columns | Rows |
   |---|---|---|
   | `py_call` | `d, call, callee` | every call with a `function` field |
   | `py_call_site` | `d, report, call, ref` | `report` is the callee less transparent parens; `ref` is the report where it is a name or attribute, else -1 |
   | `py_exec_context` | `d, call, owner, kind` | the call's nearest execution boundary: the function, class or lambda whose body holds it (-1 for the module) |
   | `py_call_arg` | `d, call, arg, ordinal, keyword, spread, value` | each named argument |
   | `py_attribute` | `d, node, object, name` | each attribute callee's object and name |

   - Node columns are the syntax layer's ids (`d << 32 | preorder among named non-comment nodes`,
     decision 0008), so rules join `node`, `text`, `child` and the rest directly.
   - `kind` and `spread` are name atoms (`module`, `sync_function`, `async_function`, `class`,
     `lambda`; `none`, `positional`, `keywords`).
   - **Departs from t11 on `keyword`:** it is the keyword's name node, not an atom. Its name is
     `text(keyword, s)`, so making a row needs no interning.
   - The `py.` of t11's names becomes `py_`, since `.` is not a bare table name.
2. **From candidates, not a scan.**
   - `facts::producers::python::families::rows` reads one document's core facts. It starts at the
     kind postings of `call` and finds each call's context by climbing to the nearest owner whose
     body holds it.
   - The semantics are S's: `py_families_equal_s_producers` checks the rows equal `collect` plus
     `collect_arguments`' over every context kind, on S's sources and the 169 fixture files.
   - S's producers stay as copied, as the oracle.
3. **Made on first read.**
   - The families are bound with each chunk's base relations (`syntax::sources`, `with_nearest`).
     Their rows are made on the first read of any of them, for the batch's Python documents.
   - A program that reads none does no work, and a batch without Python documents has empty
     families.
4. **One derivation per document, recorded on first read** (`families::py_record`, `py_inputs`,
   `Families::py`):
   - The output is `#py:<path>` and the producer is `rerail.py/v1` with the rows' `VERSION`. The
     input is the document's `#core:` name. The digest is over the rows.
   - A wave whose steps, or whose demand statement, read a family records the derivations it lacks.
     It then reads the families as inputs of its own families.
   - Propagation re-runs a derivation when its core name rebinds (`Families::recompute`).
   - Rows hold no byte positions, so an edit that adds or removes no node (whitespace, the text of a
     comment or string) keeps the digest, and the wave's families short-circuit.
5. **Demand reads them before the batch.**
   - The policies declare each family their rules read (`declare_py`).
   - The demand program counts the families as available (`demand_program`), so a magic rule keeps
     a family literal and narrows its clause to the documents that have such rows.
   - `py_families_narrow_demand_before_the_batch` checks this.

## Consequences

- The data-layer rules (list step 30) can read calls, contexts and arguments as relations, and their
  demand narrows by them before matching.
- Two kinds of row are not families yet: S's attributes beyond callees, and assignments, sequences
  and with-items (`python_structure::collect`). They are added as families, with this shape, when a
  rule reads them.
- `py.binding_site` and `py.ref_resolution` (t11) are resolution and stay in the rules.
- The rows are made twice where a wave records a derivation for the first time: once for its digest,
  once in the batch.
- What is not exercised yet:
  - recompute through propagation (`Families::py`) runs only with a real push;
  - the families are not yet read by a rule on the slice's path. The data-layer rules are the first
    readers (list step 30).

## Measured

At load 42; CPU is user + sys.
- **Python 3.13's standard library**, through the logic tool:
  - The program reads `py_call_site`, `py_exec_context`, `py_call_arg` and `text`, and finds 775
    calls by name in async functions and 103 of their keyword arguments.
  - The `local` phase, including making the families' rows for every chunk, took 0.06 s user. The
    whole run took 1.50 s user, 1.24 s of it parsing, with peak RSS 79 MB.
- **T's programs**, which read no family: they derive T's outputs unchanged:
  - Python: 7 of 7;
  - Rust: 4 of 4;
  - the fixture with `--chunk 200`: 7 of 7.

  No family's rows are made for them. The corpus path now measures only the relations a rule reads,
  so binding the families costs nothing.
