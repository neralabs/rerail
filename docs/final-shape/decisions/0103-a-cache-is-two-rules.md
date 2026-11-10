# 0103: A cache is two rules

Status: accepted (2026-11-10). This is the only form a cache may take. Cached facts are just facts (§2). The
engine's part is built in a session; without a session and in the file-local part it is not built yet (§3). The caches made before this
decision are listed in §8, each with its two rules; a test holds that list, and the list may only shrink.

## Intro

**The problem.** The engine keeps results from one run to the next, so that an edit costs what it changes. Each
such result was kept by a mechanism of its own: about fifty tables in the stores, each with its own key, its own
test of whether it is still right, and its own code that writes it. Each had to be checked on its own, and each
went wrong on its own. For example, a phase that read nothing was taken for a phase with no record, so its result
was dropped and made again on every run (fixed in 16c7fde1d). A second form (an earlier 0102) recorded every read
with its digest in tables of its own; it was one more mechanism beside the facts, and it was undone (37f8b67d8).

**What changes.** A cache is written in the program's `.rr` file, as two rules:

- **the key rule** derives the key from the input facts;
- **the output rule** derives the cached result from the key.

Both are ordinary relations. The engine keeps them as it keeps every relation, and runs the output rule only for
keys that are new. Nothing else may be a cache: no table of a cache's own, no digest compared in Rust, no map in
memory that outlives a run.

## 1. The form

```text
// The parsed form of each annotation text, kept while its text is kept.
.cache ann_tree by ann_key

// The key rule: from the input facts, every value the result depends on (here, the text alone).
ann_key(s) :- struct__vann(_, _, s).

// The output rule: from the key, by the expensive step (here, the annotation parser).
ann_tree(s, t) :- ann_key(s), parse_of(s, "annotation", t).
```

`.cache C by K` names the cache `C` and its key `K`. The program is refused unless:

1. every rule whose head is `C` reads `K` once, as a positive atom;
2. each argument of that `K` atom is a variable, and the head of the rule has it; its place in the head is
   the same in every rule of `C`. Those places are `C`'s *key columns*;
3. `C`'s stratum holds `C` alone, and no rule of `K`'s stratum reads `C`.

The key rule can be any rules: joins, negations, aggregates. The output rule's other literals can be anything the
engine runs, such as a native relation made on demand (`parse_of`), a functor, or a join over the documents' facts.

## 2. What the engine does

**Cached facts are just facts.** `K` and `C` are relations. They are stored, held in memory and written back the
same way as every other fact: the kept strata in the cross-file part, the per-file memo's rows in the file-local
part, the resident rows in a session, all written by the store's one writer (0102). There is no table, store,
hook, lookup or read of a cache's own. The cache is only a rewrite of `C`'s rules into ordinary rules:

- **The key is made again.** Each run derives `K` from its inputs. Against `K`'s previous rows (kept as `K` is
  kept), the keys new in this run are `K__plus` and the keys gone are `K__minus`.
- **The output follows its key.**
  - The rows of `C` whose key columns hold a gone key go. The output rule does not run for them.
  - The output rule runs for the new keys only.
  - For a key in both runs, `C` keeps its rows as they were. The output rule does not run for it, and nothing it
    reads besides `K` is looked at.
- **Readers.** `C`'s change goes on to the strata that read `C`, as any relation's does. A run in which `K` did
  not change has no change of `C`.

Written as the rules a session's maintenance already uses (0077 §3), with `C__old` being `C`'s previous rows as
they are kept:

```text
C__del(c…) :- K__minus(k…), C__old(c…).       // a gone key's rows
C__ins(c…) :- K__plus(k…), <the output rule's other literals>.
C(c…)      :- C__old(c…), !C__del(c…).
C(c…)      :- C__ins(c…).
```

(`k…` are `C`'s key columns among `c…`.) A row of `C` has one key, so a deleted row is never derived again: there
is no rederive step.

## 3. Where it is built

- **In a session:** built. `maintain::rewrite` writes the rules above. A cache whose key has no delta stays an
  input of the maintained program, kept as it is: nothing runs and nothing is copied.
- **Without a session, and in the file-local part:** not built yet. The cache runs as its plain rules: its
  answers are right, but the output rule runs for every key. What is needed is the same rules, with `C__old` read
  from where `C` is already kept: the kept strata (a new process maintaining from the kept rows, note t32 stage 4),
  and the file's previous rows in the per-file memo. No new storage.
- **What was removed.** A first attempt kept a cache's rows in a table of their own (`cache_rows`), read by key
  through a hook in `plan::run`. That is a cache mechanism beside the facts, which this decision does not allow. It
  is removed, and `corpus::cache_architecture` refuses it.

## 4. The key holds every value the result depends on

The engine trusts the key. If the output rule reads a fact the key does not name, a change of that fact is not
seen: the key's row is the same, so the kept rows stand. The test
`a_cache_runs_its_output_rule_only_for_new_keys` shows it: a fact the key leaves out changes, and the result does
not.

That is the difference between a cache and a derived relation. A derived relation is kept up to date from the
changes of everything it reads (maintenance, 0077 and 0090). A cache is kept up to date from its key alone. That
is what makes it cheap: one parse per new text, however many facts the parse reads. It is also why the author
must put into the key every value the result depends on: the text and the grammar, a document's path and its
content digest, the program's digest when the result depends on the program.

## 5. What is not a cache

- **Rules over facts.** Their results are relations, kept and brought up to date from the changes of what they
  read. A phase's exports (0083) are such relations: they are kept, and the next phase reads their change.
  They need no key.
- **What a session holds in memory.** It is the resident copy of relations and caches that the store also holds.
  A value held in memory and not in the store is not allowed past the run that made it.
- **Values within one run.** A map that a run builds and drops when it ends is a data structure, not a cache.

## 6. What is built

- **The form is read and checked.** `.cache C by K` is read like `.phase` (`front::Program::settings`), and
  `front::caches` checks the form of §1. A broken form refuses the program, and the message names the rule.
- **In a session.** `maintain::rewrite` maintains a cache by its key (the rules of §2), even when the output rule
  reads a relation made natively, a `nearest` literal or an aggregate: such a stratum would otherwise be derived
  again.
- **Tests.** `corpus::cache_tests`: in a session, the output rule runs only for new keys, a gone key's rows go, a
  fact the key leaves out is not seen. The same tests without a session and in the file-local part are ignored
  until §3's missing part is built. `front::caches::tests`: a broken form is refused.
- **The guard.** `corpus::cache_architecture` fails on a cache's own table, store or lookup API, and on any file
  outside the rule rewrites that knows a relation is a cache.

## 7. What is not allowed

- A table in a store that holds a cache's rows or keys, other than the relations' own storage;
- a test of freshness in Rust: digests compared, a record of what was read, a "still valid" call;
- a map in memory that is kept from one run to the next and is not the resident copy of a stored relation.

`corpus::cache_architecture` holds the tables and calls made before this decision. A table or call not on its
list fails the test. An entry is removed when its cache moves, and an entry may not be added.

## 8. The caches made before this decision

Each is listed with its two rules. Where the expensive step is a whole program (the file-local part, a phase),
it is written `@run(program, …)`: the step is that program's own rules, run for the key.

| Cache (store tables) | Key rule | Output rule | Owner |
|---|---|---|---|
| Tapes and their indexes (`tape_objects`, `tree_pages`, `term_*`, `vocabulary_*`, …) | `tape_key(d, g, h) :- document(d, path), grammar(d, g), content(d, h)` | the document's facts, from the parser for `(g, h)` | syntax |
| The per-file memo (`facts_results`, `facts_plan_sets`, `facts_completions`) | `local_key(d, h, p) :- tape_key(d, _, h), program(p)` | the file-local part's rows for `d`: `@run(p, d)` | engine |
| Compiled plans (`compiled_plans`, `compiled_plan_reads`, `plan_feedback`) | `plan_key(p, s) :- program(p), scale(s)` | the plan's choices for `p` at scale `s` | engine |
| A phase's per-document rows (`phase_granules`, `.owner`) | `granule_key(ph, d, h) :- phase(ph), owner document d with content h` | the phase's file-local rows for `d` | this branch |
| A phase's run record (`phase_record`, `export_versions`, `store_epoch`, `phase_cones`) | not a cache: the phase's exports are relations (§5) | — | this branch |
| Kept strata and the cross part (`kept_strata`, `cross_*`) | not a cache: the relations' own storage (§2) | — | engine |
| Statistics (`stat_blob`, `fact_stats`, `corpus_stats`, …) | `stat_key(rel, h) :- relation rel with rows digest h` | the statistics of `rel` | engine |
| Checks and completed trees (`local_check_results`, `completed_*`, `tree_result_segments`) | `check_key(d, h, c) :- local_key(d, h, _), check(c)` | the check's verdicts for `d` | engine |
| The derivation graph (`name_bindings`, `derivations`, `derivation_inputs`) | not a cache: what it records is the keys above | — | engine |
| The whole-run cutoff (`run_record`) | not a cache: a run with no change of any input has no delta (§2) | — | engine |

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
  - **Agrees.** A gone key's rows are removed (§2), from what was kept, with no run of the output rule.
- **Design index, §18, finding F3:** the earlier linter's fact store had "all-or-nothing cache invalidation".
  - **Departs.** The key is per row, so one changed key invalidates one key's rows.
- **Decisions.**
  - 0077 §3 and 0090: maintenance by delta. A cache is maintained by the same rules, with its key as the only
    position that can change (§2).
  - 0083 and 0092: a fact's key is its definition's content, and only the leaves (definition digests) are stored.
    The key rules of §8 are those leaves, written as rules.
  - 0102 (write-back): the one writer carries a cache's rows as it carries any relation's. No cache writes rows
    of its own.
