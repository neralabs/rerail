# 0103: A cache is two rules

Status: accepted (2026-11-10). This is the only form a cache may take. A cache's facts are just facts: nothing in
the engine treats them differently (§2). The caches made before this decision are listed in §8, each with its two
rules and the column types it needs; a test holds that list, and the list may only shrink.

## Intro

**The problem.** The engine keeps results from one run to the next, so that an edit costs what it changes. Each
such result was kept by a mechanism of its own: about fifty tables in the stores, each with its own key, its own
test of whether it is still right, and its own code that writes it. Each had to be checked on its own, and each
went wrong on its own. For example, a phase that read nothing was taken for a phase with no record, so its result
was dropped and made again on every run (fixed in 16c7fde1d). Three later attempts each added one more mechanism
beside the facts: a table of reads and digests, a special binding of kept rows, and a table of a cache's rows read
by key through a hook. All three were removed.

**What changes.** A cache is written in the program's `.rr` file as two ordinary rules:

- **the key rule** derives the key from the input facts, and puts a **version tag** in its head;
- **the output rule** derives the cached result from the key.

The engine evaluates, stores, holds in memory, writes back and maintains both exactly as it does every other
relation. To throw a cache away, the author bumps its version tag: every key row changes, so every output row is
derived again. `.cache C by K` declares which relation is the cache and which its key; only the lint and
retention read it.

## 1. The form

```text
// The parsed form of each annotation text.
.cache ann_tree by ann_key

// The key rule: every value the result depends on (here, the text), and the version tag "v1".
.decl ann_key(s: symbol, v: symbol)
ann_key(s, "v1") :- struct__vann(_, _, s).

// The output rule: from the key, by the expensive step (here, the annotation parser).
// The version stays visible in the output, so a row says which version made it.
.decl ann_tree(s: symbol, v: symbol, t: tape)
ann_tree(s, v, t) :- ann_key(s, v), parse_of(s, "annotation", t).
```

The lint (`front::caches`) refuses the program unless:

1. `C` and `K` are declared, and differ;
2. every rule whose head is `K` has a constant in its head: the version tag;
3. every rule whose head is `C` reads `K` as a positive atom.

Nothing else is asked of the form. The key rule can be any rules; the output rule can read anything the engine
runs, such as a native relation made on demand (`parse_of`), a functor or a join over the documents' facts.

## 2. What the engine does

Nothing of its own. `K` and `C` are relations:

- they are evaluated in their strata like any relation;
- they are kept where every relation is kept: the kept strata of the cross-file part, the per-file memo of the
  file-local part, a session's resident rows, all written by the store's one writer (0102);
- in a session they are maintained by delta like any relation (0077, 0090): a row of `C` goes when what derived
  it goes, and is derived when its derivation is new.

No evaluation, retention cut, support counting or maintenance rewrite asks whether a relation is a cache.
`corpus::cache_architecture` holds that: only `front::caches` and the front that calls it may know.

**The version tag.** The key rule's version is a column of `K`, and the output rule carries it into `C`. Bumping
it (`"v1"` to `"v2"`) changes every row of `K`; every row of `C` is then derived again, as it would be for any
changed input. This is how a cache whose step changed outside the facts (a new parser, a new compiler) is thrown
away: by an edit of the rule, not by a mechanism.

## 3. Column types

A cache's key and output need column types that say what they hold, so that the engine can check a rule and know
which columns may be kept and exported.

| Type | Holds | Stored as | Exportable | Status |
|---|---|---|---|---|
| `symbol`, `number`, `float` | text, integers, decimals | atom or text, integer, the float's key | yes | exist |
| `node` | a node of a document's tree | integer | no: renumbered on an edit | exists |
| `tape` | a parsed text's tape, by its content | as a symbol (its content key) | yes | **added** |
| `digest` | a 32-byte content hash | to be decided | yes | when its cache moves |
| `blob` | an opaque payload: a compiled plan, a matcher, a device model | to be decided | yes | when its cache moves |

A `tape` column holds what `@parse`, `parse_of`, `sub_tape` and `tape_of` give (`core.rr` declares them so). The
front refuses a variable that stands in a tape column and in a symbol, number or node column
(`rr::tape_type`). A tape's node rows are its document's facts, kept as every document's are.

A node within a tape needs no type of its own: its place in the tape is fixed by the tape's content, so it can be
kept as `(tape, number)`.

## 4. The key and the facts

Because a cache's facts are just facts, a change of anything the output rule reads is seen, as for any derived
relation. The key does not hide facts from the engine. Its job is to say, in the program, what the result is a
function of, and to carry the version tag. What the cache saves is the facts' own saving: a key row that stays
keeps the rows derived from it, and the expensive step runs only for a derivation that is new.

## 5. What is not a cache

- **Rules over facts.** Their results are relations, kept and brought up to date from the changes of what they
  read. A phase's exports (0083) are such relations. They need no `.cache`.
- **What a session holds in memory.** It is the resident copy of relations that the store also holds. A value held
  in memory and not in the store is not allowed past the run that made it.
- **Values within one run.** A map that a run builds and drops when it ends is a data structure, not a cache.

## 6. What is built

- **The lint.** `.cache C by K` is read like `.phase` (`front::Program::settings`), and `front::caches` checks §1.
  A refused program's message names the rule.
- **The `tape` type.** `core.rr` declares the tape columns; `rr::tape_type` checks them before lowering; the engine
  stores a tape as it always stored one.
- **Tests.** `corpus::cache_tests`, without a session and in a session: the cache's rows are what a run with nothing
  kept derives, after an edit, a new key and a gone key; a version bump derives every row again, with the new
  version. `front::caches::tests`: a key rule without a version tag, and an output rule that does not read its
  key, are refused. `rr::tape_type::tests`: a tape in a symbol or number column is refused.
- **The guard.** `corpus::cache_architecture` fails on a cache's own table, store, hook or lookup, and on any file
  but the lint that knows a relation is a cache.

## 7. What is not allowed

- A table in a store that holds a cache's rows or keys, other than the relations' own storage;
- a test of freshness in Rust: digests compared, a record of what was read, a "still valid" call;
- a rewrite, cut or count that treats a cache's rules apart from other rules;
- a map in memory that is kept from one run to the next and is not the resident copy of a stored relation.

`corpus::cache_architecture` holds the tables and calls made before this decision. A table or call not on its
list fails the test. An entry is removed when its cache moves, and an entry may not be added.

## 8. The caches made before this decision

Each is listed with its two rules (the version tag left out) and the column types it needs. Where the expensive
step is a whole program (the file-local part, a phase), it is written `@run(program, …)`: the step is that
program's own rules, run for the key.

| Cache (store tables) | Key rule | Output rule | Owner |
|---|---|---|---|
| Tapes and their indexes (`tape_objects`, `tree_pages`, `term_*`, `vocabulary_*`, …) | `tape_key(d, g, h) :- document(d, path), grammar(d, g), content(d, h)` | the document's facts, from the parser for `(g, h)` | syntax |
| The per-file memo (`facts_results`, `facts_plan_sets`, `facts_completions`) | `local_key(d, h, p) :- tape_key(d, _, h), program(p)` | the file-local part's rows for `d`: `@run(p, d)` | engine |
| Compiled plans (`compiled_plans`, `compiled_plan_reads`, `plan_feedback`) | `plan_key(p, s) :- program(p), scale(s)` | the plan's choices for `p` at scale `s` | engine |
| A phase's per-document rows (`phase_granules`, `.owner`) | **moved** (t32 3.2): none needed; the phase's run keeps its file-local rows per document in its memo | — | this branch |
| A phase's run record (`phase_record`, `export_versions`, `store_epoch`, `phase_cones`) | **moved** (t32 3.1): gone; the phase's run keeps its relations as any run's | — | this branch |
| Kept strata and the cross part (`kept_strata`, `cross_*`) | not a cache: the relations' own storage (§2) | — | engine |
| Statistics (`stat_blob`, `fact_stats`, `corpus_stats`, …) | `stat_key(rel, h) :- relation rel with rows digest h` | the statistics of `rel` | engine |
| Checks and completed trees (`local_check_results`, `completed_*`, `tree_result_segments`) | `check_key(d, h, c) :- local_key(d, h, _), check(c)` | the check's verdicts for `d` | engine |
| The derivation graph (`name_bindings`, `derivations`, `derivation_inputs`) | not a cache: what it records is the keys above | — | engine |
| The whole-run cutoff (`run_record`) | not a cache: a run with no change of any input has no delta (§2) | — | engine |


The types each needs, from its tables' columns:

| Cache | Key columns | Output columns |
|---|---|---|
| Tapes and their documents | the source's `digest`, grammar `symbol`, grammar version | `tape`, the node rows |
| Search indexes | `tape`, index version | terms, kinds, counts: `number` |
| Text matchers | language, pattern: `symbol` | the compiled matcher: `blob` |
| Compiled plans, device models | the plan's `digest`, compiler version | the program or archive: `blob` |
| Statistics, plan feedback | relation and column, or plan `digest` and backend | counts `number`, selectivity `float` |
| Checks, completed trees | the source's or tree's `digest`, the catalogue's `digest` | findings: ordinary columns |
| What each file is now | path `symbol` | device, inode, size, mtime `number`, content `digest` |
| Run constants | the observations they read | ordinary columns |

The per-file memo, the cross part's memo, a phase's per-document rows, the run record and the derivation graph
need no type: once caches are plain rules, they are the kept relations themselves.

The order: this branch's own first (a phase's per-document rows, its run record), then the rest by their owners.
Each move deletes the table and its freshness test, writes the two rules into the program that uses the result,
and removes the entry from `corpus::cache_architecture`.

## Prior work consulted

- **ast-index-spike history index, §2.16 (d):** "Key memoized analysis by what it is a function of (pattern,
  language), never by object identity", and "Language must be part of every cache key" (the S238 bug,
  DESIGN_HISTORY L7131-7133).
  - **Agrees.** The key rule is where "what it is a function of" is written, in the program, where it can be
    read. §4 states the rule.
- **History index, §2.5 (a), the lexicon entry (DH L2185-2189):** an in-memory vocabulary silently fell back after a
  reopen. "Storing data in memory ... is dishonest."
  - **Agrees.** What a session holds in memory is the resident copy of stored relations (§5).
- **History index, Part 0 (S174, S178):** statistics held only in process memory meant that a reopened store planned
  differently.
  - **Agrees.** Same as above.
- **Design index, S134:** an incremental load must return its dead rows, or it fills the disk.
  - **Agrees.** A gone key's rows go as any relation's dead rows go, by maintenance (§2).
- **Design index, §18, finding F3:** the earlier linter's fact store had "all-or-nothing cache invalidation".
  - **Departs.** A cache's rows are maintained per row like any relation's; a version bump re-derives one cache, not
    the store.
- **Decisions.**
  - 0077 §3 and 0090: maintenance by delta. A cache is maintained by the same rules as every relation (§2).
  - 0083 and 0092: a fact's key is its definition's content, and only the leaves (definition digests) are stored.
    The key rules of §8 are those leaves, written as rules.
  - 0102 (write-back): the one writer carries a cache's rows as it carries any relation's. No cache writes rows
    of its own.
