# t33: Every cache moved to facts, one by one (decision 0104's moves)

Status: plan (2026-11-10). Built so far: K1 for the cross-file strata (step 4, below), and K3's rule for the cone of requests (below, "A native step is a relation"). It takes decision 0104's eight steps and says, for each
table and map that 0104's checks (`logic::facts_only`) list as "not yet facts", what replaces it.

## A native step is a relation (built)

A stratum that makes requests and answers them inside its fixpoint (the cone of requests: directories, files,
loads; 0065 §2a), and a stratum of produced relations above their requests, are kept like any stratum, in the
frames. A native step's answers are among what its stratum read, recorded as `answer:<p>` with their digest:

- the stratum is clean while what it reads is unchanged and its stored requests, answered again, give the
  recorded answers. Answering again is the observer's `stat` for a directory or a file, and the chunk path for a
  load, which the run takes anyway;
- a native answer is a function of its request: a load asked again gives the same document (`World::loaded_as`),
  not sent through the chunks twice;
- a relation that is a member of the stratum and an input too (a per-file relation, its workspace rows given and
  the loaded documents' rows answered into it) is read as an input;
- a view in a stratum is as its stratum's other relations and reads are (`view_digest`).

Measured (237 Python files, a 204-document typeshed load, CPU user+sys, load 20 to 50): a comment edit 18 to 19 s
before, 1.1 to 1.2 s now; a new import runs the cone in full (11.5 s), and loads what a run with nothing kept loads.
Test: `corpus::tests::the_cone_of_requests_is_kept_while_its_reads_and_answers_are`. Next: maintain the cone from
the delta of its inputs (K4), and drop `World::still` and the whole-run cutoff, which these rules now cover.

## Intro

**What a cache becomes.** 0104's rule, and 0103's form, with nothing added: a cache is two rules in a `.rr` file.

```text
// The key: what decides whether the result still holds. Here, a file's content and the grammar.
doc_key(d, c, g)  :- doc(d, p, _), file(p, c), grammar_of(p, g).
// The result: an expensive native step answers each key it has not answered (0065 §2a).
doc_tape(d, t)    :- doc_key(d, c, g), parse_of(c, g, t).     // parse_of is answered natively
```

The engine stores both relations like any others. On the next run an unchanged file gives the same `doc_key`
row, so there is no delta, and nothing asks for the parse. Invalidation is the engine's own maintenance.

**What may stay in Rust.** A map in memory is fine (a compiled automaton, a decoded tape, a resident relation),
but only as the in-memory copy of facts. What it holds is written as rows of the relations above, behind the run,
by 0102's writer, and a new process reads it back from them.

**How to read this note.** First the four shared pieces that every move needs (K1–K4), since none of the moves
can finish without them. Then each step of 0104, with each of its tables and maps. Then the order of work, the
checks, and what needs a decision.

## The shared pieces

| Piece | What it is | What exists | What is missing |
|---|---|---|---|
| **K1. One stored form for facts** | Every relation of every fact space, stored as frames per (space, relation, partition). The partition is a document key, a granule, or the whole relation | `kept_strata` (whole relations, column varints), the memo's frames per document, `rel_*` over frames (bd12fc52d) | one table for all of them, written only by 0102's writer |
| **K2. Leaf facts** | What the observer sees, as base relations: `file(path, digest)`, `dir(path, listing)`, `path_kind(path, k)`, `env(k, v)`, `engine(build)`, `grammar(g, version)` | the observer (`content_store/observe.rs`), its stat cache in `<store>.files` | the answers as relations; the stat cache written as their rows |
| **K3. Answers** | A request relation answered by a native step, its rows entering as facts | `parse_of`, `splice_of`, `dir_entry`, `file_line`, `load` (0065 §2a, 0058 §4) | one registry (`.answer req -> rel`, 0103's pattern); results that are objects (a tape, an automaton) as a value naming the object |
| **K4. Load and maintain across processes** | A run binds the stored relations as its old state, takes the delta of its base relations against the stored ones, and maintains the rest by counting (0090), or by DRed or levels for recursion | in a session, from memory (0077, 0090); across processes, only `kept_strata`'s clean strata | the session path started from K1; skipping strata whose deltas are empty (0101 §1) |

With K4, the whole-run cutoff is not a mechanism of its own: when every leaf delta is empty, no stratum runs,
and every output is as stored.

### K2 in detail (2026-11-21, Typing-support)

The observer (`content_store/observe.rs`) keeps its answers in a SQLite file of its own beside the store
(`<store>.files`): `observed` (a path's kind, stat, digest, listing, when recorded), `observer_meta` (the journal's
cursors per root and volume), `run_record` (each path's canonical form, the git listings, the whole-run cutoff,
the run constants), `observed_changes` and `git_alias`. It reads a root's part at a time, as a run first watches or
reads under it (`load_under`, a range of paths), and writes what changed behind the run (`write_dirty`). K2 keeps
the same answers as relations of one kept space, in K1's form (`kept_facts`, 0456), and changes nothing in how
they are answered (the journal, the racy rule, validation, Git's blobs):

| Relation | Columns | Partition |
|---|---|---|
| `file_stat` | path, kind, dev, ino, size, mtime_ns, ctime_ns, recorded_ns, digest (a `digest`, t32 1.4) | the root it is under (`load_under`'s unit) |
| `dir_entry` | dir, name, is_dir | the root its directory is under |
| `canonical` | path, canonical path | the root |
| `observer_cursor` | root, volume, cursor | one partition |

A partition is one put (`KeptWhat::Flat`) and one read, as a root's range of rows is now. `git_alias` and
`observed_changes` stay: the first is a name of bytes (a fact store table), the second the observer's journal
(decision 0433, listed as a fact store table). The whole-run cutoff's and the constants' records move with step 1's
other entries (below), not with K2.

What has to be built first: paths as atoms of the content store. The observer is opened on its own, one per store
path in the shared object (`observe::with`, decision 0442), the first time a run asks it, and its file has no atoms;
the kept space needs the store's. So the order is (1) the observer opens with the content store whose atoms it
uses, (2) `file_stat`, `dir_entry` and `canonical` written and read as the space's
partitions in place of the three tables, (3) `observer_cursor`, and `observer_meta` and `observed` go from
`facts_only`'s and `cache_architecture`'s lists. Measured at each step: a warm unchanged run and a one-file edit
over the 84-file corpus (the observer's `stat`s and reads, its load time, CPU).

**Built (2026-11-21).** `content_store/observe/facts.rs`. The observer opens the content store its file is named
after (`<store>.files` beside `<store>`) and keeps three relations in a space of its own (one per observer file, so
a test thread's observer keeps apart, as its file did):

- `observed(path, kind, dev, ino, size, mtime_ns, ctime_ns, digest, listed, listing, recorded_ns)`, the path, the
  digest's hex and the encoded listing as atoms of the store. One partition per directory, not per root: an edit
  rewrites its directory's partition, where a root's is up to `LIST_MAX` answers. A root's part is read as the
  partitions at and under it, and the root's own answer from its parent's.
- `canonical(path, canonical, recorded_ns)`, partitioned as `observed`: the `canonical:` records `run_record` held.
  Collection drops the old records.
- `observer_meta(key, is_text, number, text)`, one partition: the cursors, the observer's own journal's generation and
  floor, its output directories.

Each change is one put of the space, sent through the observer's writer in order after its SQLite writes: the
answers a root forgets and the cursor that vouches for the rest are written together, as the table's transaction
wrote them. The aged cursor, a write a warm run may make, is a job of its own under that name
(`ContentStore::queue_kept_as`). The listing stays a column, not `dir_entry` rows. `git_alias` and
`observed_changes` stay as planned; `run_record`'s other records (the Git file lists, the cutoffs, the constants,
the cone's rounds) are step 1's.

### K3 in detail (2026-11-21, Typing-support)

The native steps were three lists in Rust (`rr::lower`'s `FILE_SYSTEM` and `ENGINE_ANSWERS`, and the languages'
`Answerer`s), with `load` beside them in `corpus::world`, and nothing in the library said which request a native
step answers. Built: each is declared where its relations are, as `.answer REQ -> R1, R2` (`core.rerail`,
`git/*.rerail`, `analysis.rerail`, `c/preprocess.rerail`, `c/compile-commands.rerail`), read like `.cache` and
linted by `front::answers` (its relations declared, none answered by two requests).
`language::tests::every_native_answer_is_declared_in_the_library` holds the engine's answerers and the library's
declarations to one list, so an answerer is registered only for a request the library declares, and the reverse.
An answer that is an object is a value naming it already: `parse_of`'s and `splice_of`'s tapes are `tape` columns,
and a digest or a blob is a column of its type (t32 1.4).

**A relation's identity.** A relation of a program is stored under the digest of the rules it is derived from,
the identities of what they read, and the engine (`plan::identities`). A rule edit makes it another relation,
empty, so it is derived again and its readers see the delta; a relation two programs derive alike is one frame.
A derived frame records the digest each relation its stratum read had then, and is current while each still has
it (03 §1's versions, at the grain of a relation).

**Identity by naming, for every relation** (user, 2026-11-10). A cached relation is a relation like any other:
its identity names its rules and what they read, so a rule edit makes it new, with no join of the program's facts
in any key. A version, when one is wanted, is a column of the facts, written in the rule that makes them. Bumping
it is a rule edit like any, so the relation and everything derived from it are derived again. No directive and
no special handling:

```text
.decl my_cached_fact(key: symbol, result: symbol, v: symbol)
my_cached_fact(k, r, "v1") :- my_key(k), r = @expensive(k).    // "v2" derives it again
```

## The moves

Each line: what the table or map is today (its key, and how it is checked), and what replaces it. "Goes" means
removed with nothing in its place but K1–K4.

### Step 1: the observer, the constants and the whole-run cutoff (this branch)

| Today | Key and check today | Becomes |
|---|---|---|
| `observed`, `observer_meta` | a path's stat; reused while `stat` is equal; a rotating validation counter | K2's base relations; the stat cache written as `file_stat(path, dev, ino, size, mtime_ns, ctime_ns, recorded_ns, digest)` in K1; the counter is one fact |
| `run_record`, `corpus::reuse`, `World::still`, `inputs::still`, `git::still`, `unchanged`, `unchanged_phase`, `changes` | the run's text, options, root, modes and engine; each consulted read compared in Rust | goes: K4 with empty leaf deltas |
| `inputs.rs` `CONSULTED` | the inputs, artifacts and `base` revisions a run read | answers to the input requests (K3), whose rows are the record |
| `git.rs` `STATE` | the index read once per run, its file's digest | the index file is a leaf (K2); `git_tracked` and `git_has` answers are rows keyed by it |
| `constant_rows` | the programs, relations, options, root, engine; `World::still` | relations of the constants' space (no documents), stored in K1, maintained from `env`, `dir_entry` and `file_line` deltas |
| `derivations`, `derivation_inputs`, `name_bindings` | names bound to digests; validity by a join | the corpus path's uses go (phases' granules: step 5; own counts: step 6). The tables go when the workspace pipeline's use goes (step 8) |

### Step 2: rule files and programs

| Today | Key and check today | Becomes |
|---|---|---|
| `rr.rs` `PARSED`, `LIBRARIES`, `PROGRAMS` | a file's path and digest; the files' digests | a rule file is a document loaded by `load(path, _, "rr")` (phase 0 already does); its statements are 0074's program facts, stored in K1 under the file's `file` row |
| `phase0.rs` `KEPT` | the seed; held while every module is as it was | `bootstrap.rr`'s output relations (the include graph), stored in K1 |
| `corpus.rs` `COMBINED`, `PREPARED`, `prepared.rs`'s memo entries | the texts' digest; the texts, additions, stage, dialect, build | `prepared(p, …) :- program(p, digest), engine(b), …` answered by preparing; its rows are the `prepared_*` program facts 0074 already writes. **Pointer built** (2026-11-21, `prepared.rs` `pointer`): the `prepared_*` and `prepared_program` rows are kept facts of the store's prepared programs' space, a partition a program (its pointer plan: build, grammar, key); `PREPARED` is their in-memory copy (state). The programs' texts stay sources |
| `choices.rs` `KEPT` | a digest of the program, the build and three switches, "a scale at most GROWTH times theirs" | an answer keyed by the program's facts and the statistics it read (with step 6) |

### Step 3: a document's facts

| Today | Key and check today | Becomes |
|---|---|---|
| `facts_results`, `facts_plan_sets`, `facts_completions` (the per-file memo, 0052) | content, path, plan digest (build, grammar, node-id scheme, program, options, facts) | the per-file part's relations in K1, partitioned by document. A document whose base relations have no delta is not run (K4 over a per-document partition). `plan_digest`, `with_options` and `with_rows` go; `doc_keys` stays (it is identity). **Started** (2026-11-22). The tables are a store of opaque entries with six users, not one cache. A kept partition can now be held by a source and a plan (`kept_facts::HOLDER`), so it dies when the memo row it replaces would have; `plan::kept_entry` keeps an entry of program facts that way. Moved: the prepared pointer (which had lost its collection when step 2 moved it) and the slice's content memo (`slice/memo.rs`). Left: `combined.rs`, `preparation/parts.rs` and `choices.rs`, whose entries are read inside the store writer's record steps (`name_program`, `apply_plans`), where a kept read would wait on the writer itself: they need the entry carried in the record's body, or a read of the wave's gathered puts. `facts/memo.rs` is the workspace pipeline's (step 8). The tables go with the last of them |
| the projection spike (a075764a3) | (tape, relation) | the base relations as derived relations of `doc_tape`, in K1 per document. After RER-131 renumbers the nodes |
| tapes: `source_objects`, `tape_objects` (kept: they are objects) | (content, tag, grammar version, format) | `doc_key` and `doc_tape` as in the Intro; the object tables hold the bytes the answer's value names |
| `guests.rs` `KEPT` | (text, grammar) | `parse_of`'s answer rows (K3) |
| `cross_rows`, `cross_blocks`, `cross_components` (0076) | an import component's file bytes | goes: cross-file relations in K1, partitioned by the document of their first location column, maintained by K4. 0092 §6: keys of bytes have no cutoff |
| `slice_rules`, `slice_document`, `slice_match`, `slice_span`, `slice_fresh` | the slice's rules and documents | **built** (2026-11-21, `slice/kept.rs`): relations of the scope's space in K1, a document's rows its own partition. A run writes only the partitions of the documents it evaluated, reused or lost; the wave reads the others where they lie, through `kept_facts` (a temp view, no copy). The SQL column statistics of the two tables went with them. `slice_fresh` is the run's scratch |
| `corpus_stats` | a path's stat to its source | goes: K2's `file_stat` |
| `corpus_plans`, `corpus_plan_relations`, `corpus_manifests`, `corpus_manifest_plans`, `corpus_roots`, `corpus_live`, `corpus_state`, `corpus_decided`, `corpus_swept` (0053) | each run's manifest of what it used | collection is reachability: an object or a frame is live while a stored fact names it. How long an old state is kept is a `.cache` directive on the relation |
| `session.rs` `SESSION` | the session's memo, cross part, phased exports, written outputs, statistics | the resident copy of K1's relations, written behind by 0102 |

### Step 4: the kept strata

| Today | Key and check today | Becomes |
|---|---|---|
| `kept_strata`, `kept_strata_programs` | the cross program's digest; each input's digest compared in Rust | **built**: `content_store::frames`, the cross part's relations by identity in the root's cross space, each with its reads' digests; the per-program key and the input-digest rows are gone. On a two-program test, the second program's first run evaluates 1 of 2 strata (the shared one is clean), and a rule edit 1 of 3 |
| `recursive_spike.rs` `CARRY` | the spike's carried state | goes with the spike, or into K1 |

### Step 5: the phase store (the typing branch)

**Done** (investigate-bend-vs-datalog, t32 stage 3): the phase store's tables, `.owner` and `Session::phase_rows` are
gone. A phase's exports are its nested run's outputs, kept as that run keeps every relation, and handed on as input
rows. The constants' rows (`constant_rows`, step 1) stay in the same `<store>.phases` file until step 1 moves them.

| Today | Key and check today | Becomes |
|---|---|---|
| `phase_granules` | (phase, granule) rows as text | the phase's exports, relations of its space in K1, partitioned by granule |
| `phase_record` | the phase's key and what it consulted | goes: K4 |
| `phase_cones` | each document's cone inputs | goes: the delta reaches what it reaches |
| `export_versions`, `store_epoch` | a counter per export, for a session's rebinding | K1's own per-relation change counter, or goes |

### Step 6: planning and statistics

| Today | Key and check today | Becomes |
|---|---|---|
| `stat_blob`, `fact_stats`, `fact_value_stats`, `kind_size`, `edge_freq`, `tape_kind_counts`, `term_freq`, `term_kind_freq`, `term_node_freq`, `term_coverage`, own counts (a name binding) | one blob per plan; patched by its own delta code (0096) | aggregate relations over K1's relations (`count` per relation, column and document), maintained by counting like any aggregate |
| `compiled_plans`, `compiled_plan_reads`, `plan.rs` `PLANS`, `SESSION_PLANS`, `KEPT`, `derived.rs` `MEMO`, `SESSION_MEMO` | a plan key; statistic intervals | a plan is an answer keyed by its stratum's definition and the statistics' intervals it is valid over (03 §6 already records those intervals). `compiled_plans`, `compiled_plan_reads`: **built** (2026-11-22, `stats/plans.rs`): a kept space, a partition a plan (`compiled_plan`, `compiled_plan_bytes`, `compiled_plan_read`); read from a space opened before a candidate transaction, written after it |
| `plan_feedback`, `keeps.rs` `OBSERVED`, `table.rs` `PAID`, `accel_models` | what a run observed | facts the run produces about itself, stored in K1. `plan_feedback`: **built** (2026-11-22), a relation of the compiled plans' space. `accel_models`: **built** (2026-11-21, `content_store/accel_models.rs`): `accel_model(key, archive)` in the store's models' space, a partition a key; the archive a source of the store by its SHA-256, pinned with no end as the table kept it |

### Step 7: text patterns and indexes

| Today | Key and check today | Becomes |
|---|---|---|
| `text_matcher`, `text_term`, `text_pending`, the stored automata (0075's memo entries) | the patterns' key | `dfa(g, a) :- text_group(g, patterns), …` answered by determinizing; the automaton is an object its value names |
| `strings.rs` `MATCHERS`, `GROUPS`, `functions.rs` `CACHE`, `text.rs` `LOWERED` | a pattern | in-memory forms of a pattern fact, made again from it: nothing to write (compiling a regex is cheap; determinizing, above, is not). `CACHE`: **done**, listed as state (2026-11-21) |
| `text.rs` `NAMES` | a production's name | a profiling counter: state, not a cache. **Done**, listed as state (2026-11-21) |
| `prefilter_sets`, `term_postings`, `vocabulary_*`, `search_index_sets` | the store's own triggers and indexes | derived relations over tapes, in K1 per document, with the indexes they need |

### Step 8: the workspace pipeline

`tree_pages`, `tree_result_segments`, `completed_*`, `candidate_*`, `local_check_results`,
`context_check_results`: moved with that pipeline by its owner, or retired with it. Not planned here.

## Order of work

1. **K1**: the one stored form, with `kept_strata` as its first user (step 4). Smallest proof: the cross part's
   strata keyed by relation identity, not by program.
2. **K2**: the observer's answers as base relations, its stat cache as rows.
3. **K4**: a new process loads K1 and maintains it from K2's deltas. Then step 1's cutoff goes.
4. **Step 1**, then 3 (the memo, the cross memo, collection), 2, 5, 7, 6.

## Checks

- `logic::facts_only` (0104): every table and map listed; the not-yet lists may only shrink. Each move lowers
  its pins.
- **An unchanged rerun asks no answer and runs no stratum.** A test on a small corpus counts answers made and
  strata run in a second process: both must be 0.
- **Every key is a rule.** An answerer is registered only for a request relation that a `.rr` rule derives;
  the registry is checked against the library.
- **Cold equals warm equals incremental.** The session's equality check (`session::full`), run across
  processes too.

## Needs a decision

Decided (user, 2026-11-10):

1. **K1's layout is frames** per (space, relation, partition), as chosen for SQL access (bd12fc52d).
2. **This branch builds what the other session is not doing:** K1 to K4, and steps 1 and 4.
3. **Identity by naming for every relation**; a version is a column of the facts, bumped in the rule (above).

Decided (user, 2026-11-21):

4. **Typing-support-final-shape takes on the rest of this plan** (t32 5.3): K2, K3 and step 1 first, which no
   other branch is building; then steps 3, 2, 6 and 7. K4 for each document and the statistics stay with the sync
   branch's Q332, merged from the hub as they land.

   Checked 2026-11-22: no branch is moving K4 or the statistics' tables to kept facts. The sync branch's Q332 is
   about warm edits making only the changed documents. What other branches build near it is the corpus path's
   statistics kept as kept facts: `final-rerail-shape` (a structure's statistics, `statkeep` format 4) and
   `final-rerail-shape-rr-align` (a semi-join's rows and statistics, Q396). So this branch takes the rest too. It
   takes the SQL path's statistics (`fact_stats`, `kind_size`, `edge_freq`, `term_*`) and the text matchers (step
   7) once the hub brings in those branches' work and `engine-crating`'s matcher crate, so it builds on them. The
   `corpus_*` tables are collection's bookkeeping, not caches (kept facts' own collection reads `corpus_plans`), and
   stay; `run_record` goes with K4, not as a move of its own.
