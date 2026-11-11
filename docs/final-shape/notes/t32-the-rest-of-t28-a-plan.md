# t32. The rest of t28: a plan, with every cache as two rules

## Intro

t28 made Python typing three phases: P1 `struct.rr`, P2 `types.rr`, P3 `calls_p3.rr`. P3 now answers every call
`calls_cx.rr` answers (6,410 agree), and a session maintains P2 from struct's delta (a function added: 9.4 s to
2.5 s). This note plans what is left, under two constraints:

- **A cache is two rules** (decision 0103). The key rule derives the key from the input facts and carries a
  version tag; the output rule derives the result from the key. Both are ordinary relations: evaluated, kept and
  maintained as every relation is. Bumping the version derives the cache again. Nothing else may be a cache: no
  table of a cache's own, no digest compared in Rust, no rewrite or hook for caches, no map in memory that
  outlives a run. What a session holds in memory is the resident copy of stored relations.
- **Stay compatible with the sibling worktrees.** This plan builds on their parts and makes no second copy of any.

This note replaces its first version (47dd0e29b), which planned on an earlier 0102 ("a cache is stored facts").
rr-align undid that form (37f8b67d8) for the reason 0103 gives: it was one more mechanism beside the facts.

## What the siblings are doing (2026-11-10)

| Worktree, branch | Work that touches this plan | State |
|---|---|---|
| final-shape-rr-align | its 0102 (caches as `cache_entry` tables) undone; c66b24bb4: the constants of phase exports derived once, before the chunks; `held_rows` for them | merged here (0e7305ed3) |
| More-ergonomic-rule-syntax | **0102 write-back**: one writer thread per store; the run goes on from memory | merged here (135d67609) |
| Matrix-offload-spikes | **kept strata across processes** (470e92fde): each derived relation's rows and digest kept per cross program, with early cutoff | merged here, through the two above |
| Two-pass-roots-and-cross-file | "derive again by input delta" (0090 §4, 3c2f02444) in `maintain.rs` and `plan::run` | the hub is merging it now; not merged here |
| wolfeel (trunk) | 0100 §11 chaining in `plan::run`'s level loop; uncommitted executor work | not merged here |
| Final-shape-quality (hub) | 0101 draft (skip strata whose deltas are all empty) | 0101 not built |
| Typing-support | per-definition node ids (uncommitted `syntax/granules.rs`) | uncommitted |
| Current-state-benchmarks | t31, the one-file-edit workplan: B4 keeps relations between processes "as column blocks" | a note |

Nobody else edits `typing/programs/*.rr`, `phases.rs` or `session.rs`. This branch's 0103 work adds nothing to
code others are changing: a cache needs no engine code, and its lint is in `front::caches`.

## Principles

1. **A cache is two rules in the program's `.rr` (0103).** Its key and its output are relations. A new cache,
   moved or made, is written that way, or it is not made.
2. **Rules over facts are not cached.** Their results are relations, kept between runs and brought up to date from
   their reads' changes (0077, 0090). A phase's exports are such relations. Its run record (`phase_record`,
   `export_versions`, `store_epoch`) is a freshness test of its own, so it goes.
3. **Memory is the resident copy.** A session keeps relations in engine form between runs; the store keeps the
   same relations, written by the one writer (0102). A new process reads them from the store. Nothing is kept only
   in memory past a run.
4. **Build where the owner builds.** 0101, "derive again" and the level loop belong to the branches that own
   `plan::run`. This branch uses them.

## The work

Each step is measured before the next, on the 84-file corpus: P3's answers against `calls_cx.rr` and Jedi; P2's
exports against a full run; CPU and instructions retired, cold, warm and after the O125 edits.

### Stage 1. The cache form (done: 0103)

| Step | Change | State |
|---|---|---|
| 1.1 | `.cache C by K` linted (`front::caches`): the key rule has a version tag, the output rule reads the key | built, tested |
| 1.2 | No engine special case: maintenance, retention and support counting treat a cache as any relation. A `cache_rows` table, a lookup hook and a key-only maintenance rewrite were built and removed | built, tested: rows equal a run with nothing kept; a version bump derives them again |
| 1.3 | The `tape` column type: `core.rr`'s tape columns, checked before lowering (`rr::tape_type`), stored as before | built, tested |
| 1.4 | `digest` and `blob` column types, when the first cache that needs one moves (0103 §3) | open |
| 1.5 | `corpus::cache_architecture`: the tables and freshness calls made before 0103 (may only shrink); no cache table, store, hook or lookup; only the lint knows a relation is a cache | built |

### Stage 2. P3 and P2 rules (this branch, now)

| Step | Change | Done when |
|---|---|---|
| 2.1 | **P1 exports annotations as structure**, read off each annotation's own tree in the file: `struct__ann(m, e, a, h)` (annotation `e` in module `m` has alternative `a`, head `h`) and `struct__annarg(m, a, j, x)` (its j-th argument). P2's `alt`, `head_of`, `garg`, `galt`, `pbase`, `cbody` and `cfunc` join on them; no text is split. A quoted annotation's text is still split at `|` in P1 until its string is parsed (a cache by its text, 0103) | **done.** Over the 84-file corpus with typeshed: every row P2 lost was a misreading (an argument's union taken for an alternative: `Iterable[bytes \| bytearray]` gave `bytearray`); 52 rows gained (`pgen` +13, `rcarg` +35, `rgen` +4), 40 fewer open returns; workspace resolution unchanged (4438 of 4867). P1 +1.9 s CPU (6.8 to 8.7 s, load 44 on 18 cores), P2 unchanged (2.5 s) |
| 2.2 | `__all__`: P1 exports a module's `__all__` (`=` and `+=`, not in a dead branch) as `struct__all(m, x)`; P2's `import *` binds what it lists, or with none the public names | **done.** `phase_tests::types_resolves_…`: a listed private name bound, an unlisted one not. P3 over the 84 files: 6,410 agree before and after (and after 2.1), 38–40 s CPU |
| 2.3 | Overloads by keyword: `lfit` by the keyword names a call passes, against `struct__param`'s names | **done** (with 2.4): a library overload whose parameters lack a keyword the call names, and take no `**kwargs`, is refused (`lkwbad`); P1 gives untyped defaults a `param` row. Oracle test `…fits_a_library_overload_by_its_keywords_and_its_literal_arguments_types` |
| 2.4 | Overloads by argument type, where the argument's class is known and the parameter declares one | **done**, for literal arguments: a literal's class against a parameter that names only classes, none a protocol, with the numeric tower (`ltybad`). P3: 6,410 agree before and after; 47 calls no longer left open beside their answer. The lines counted as moved to "no edge" (2) each close a call jedi answers nothing for, beside a miss that was there before (`_build_cross_branch_base_spec`) |
| 2.5 | Async: P1 flags a library `async def`; `async for` and `async with` as implicit calls | **done.** `async for x in s` binds x to the awaited `s.__aiter__().__anext__()`, `async with c as z` z to the awaited `c.__aenter__()` (`iawait`): a workspace `async def`'s result, a stub `async def`'s declared class or `Self`; a plain method returning an awaitable stays open. Oracle test `…awaits_an_async_loops_next_value_and_an_async_withs_entered_value` |
| 2.6 | Nested runs' profile: a rise on a phase's worker is charged to the phase's span | **done.** The pool's jobs open their span under the span open where they were started (`threads`, `profile::span_at`); a phase's run is a span. A cold P3 run shows P1 11.1 s and P2 3.6 s CPU, their chunk jobs under them |

### Stage 3. A phase's state is relations (this branch)

| Step | Change | Done when |
|---|---|---|
| 3.1 | **The run record goes.** A phase's nested run keeps its strata as any run does (the kept strata, or the session). An unchanged run is a run whose inputs have no change: every stratum clean, nothing derived | **done.** The phase store is gone: a phase is its nested run, whose relations are kept as any run's (memo, kept strata, session); its exports are handed to the reader as input rows, whose delta the reader's run takes as any input's (by its documents, or the diff rules). Off the 0103 and 0104 lists |
| 3.2 | **A phase's per-document rows are a cache.** `.owner` and `phase_granules` become two rules in `struct.rr`: `doc_key(d, h)` from each document's content digest, and the per-document exports from it | **done, without a cache:** struct's file-local rows are per document in its run's per-file memo already, so `.owner`, `.positions` and `phase_granules` go with no rule in their place |
| 3.3 | `Session::phase_rows` (6afe7e79f) goes: the session's kept relations are the phase's exports | **done.** `phase_rows` and `phased` (the versions bound by the session) gone |
| 3.4 | The cones (`phase_cones`) go with 3.1 | **done** |

What this costs until K4 (t33): a phase's export statistics are measured from its rows each run, no longer patched by
version deltas (0096 §3), and an unchanged run runs the phase's nested run (all its parts kept) instead of reading a
record.

Measured after 2.5 and stage 3 (2026-11-10, load 36 to 76): P3 over the 84-file corpus agrees with jedi on 6,410
calls, as before; 76.8 s user CPU, 91 s wall, peak RSS 1,292 MB.

### Stage 4. P2 maintained across processes

t28's step 7 left one gap: a new process runs P2 in full after any struct change (3.8 s). The kept strata hold
every derived relation's rows between processes, but a stratum whose reads changed is derived again. A session
maintains it from the deltas instead.

| Step | Change | Done when |
|---|---|---|
| 4.1 | **A new process maintains from the kept rows**, as a session does: the kept strata's rows are the `__old` rows of `maintain::rewrite`, and an input's delta is its kept rows against its new ones | **built** (`kept_strata::load`, `keep`): a run with no session takes the session's branch, its `Retained` read back from the store's frames (space `cross-maintained`, each relation under its identity) and written back after the run (only frames whose rows or inputs changed). A frame records the digests of the inputs it was derived from (its cone), and is read back only while those inputs' frames hold them: frames are shared by identity across programs. Not for a part whose requests the run answers (phase 0's loads). Test: `a_new_process_maintains_the_cross_file_part_from_the_kept_rows`. **Measured** below |
| 4.2 | Measure what reading the kept rows costs, against the 3.8 s full run; keep fewer relations (0099) or read lazily if it is more | **done**, below: 0.06 to 0.09 s a run |

Measured 2026-11-10 on 54 of the corpus's files (`_internal`, `l1100_exceptions`, `l2055_lazy_result`), the t28 spike
with `RERAIL_SPIKE_PROCESS=1` (no session; the process's own caches of programs and files stay warm). P2's CPU; the
full run is the old per-stratum cut (`RERAIL_KEPT_PART=off`). Exports equal to the full run's after every edit.

| Run | A new process | Full run | Strata run (of 110) | Read back | Written |
|---|---|---|---|---|---|
| unchanged | 0.59 s | | 0 | 0.08 s | 0.02 s, queued |
| E1 | 3.15 s | 3.57 s | 97, all maintained (6 recursive) | 0.09 s | 0.02 s |
| E2 | 0.57 s | 2.67 s | 0 | 0.09 s | 0.02 s |
| E3 | 0.40 s | 1.73 s | 0 | 0.06 s | 0.01 s |

Three faults found on the way, fixed:

- A kept relation read by SQL (`want`'s rule, a substring) had no index: SQLite scanned it per probe, 16 s of an
  unchanged run's 17. A kept relation as a source (`Derived`) now answers probes by a sorted index per column,
  built on first probe; that is so in a session too.
- Outputs were not kept (their decoded lines were not), so their strata ran each process and their readers could
  not be maintained. Their rows are kept as any relation's, and a clean output's lines decoded from them.
- An input's frame was keyed by its name only, so programs over one root that bind an input of that name to other
  rows (P3's cross part, its loading cone, the phases: `py_docmod` 63 against 237 rows) overwrote each other, and
  every process found its inputs changed. An input's frame is now its program's.

P3 over the same 54 files, three processes, no edit: the main thread's CPU 28.8 s (cold), 13.4 s, then 4.7 s; P3's
cross part 6.4 s, 0.78 s (1 of 481 strata), 0.91 s; the agree count 4,815 each time. The second process ran the
phases in full. Not the relations' identities (every stratum the second process has, the cold one has with the same
identity); two faults, both fixed:

- A frame space counted a run per program that kept frames in it, not per run. Every program over one root keeps
  frames in one space (the phases, P3's cross part, its loading cone, phase 0), so the cold run's eight programs
  were eight runs, and the types phase's 126 frames were collected (unused for `RUNS` = 4 runs) before the run
  ended. A space now counts the outermost run once (`corpus::run_token`), however many of its programs keep frames
  (`frames::the_programs_of_one_run_count_one_run_of_their_space`).
- A stored prepared program lost the relations it has facts of but no rows (`py_site`, `rr_ctx3`: constants folded to
  nothing), so the struct phase's file-local part had another memo key in every later process than in the one that
  prepared it, and its 183 library documents were derived again. `fact_rel` keeps them
  (`program_facts::a_relation_of_no_facts_reads_back_as_one`); the stored program is the program prepared.

After both, on the same 54 files at load 137 to 160: 92.3 s CPU cold (its first run since the prepared program's
schema moved), 31.7 s, 29.8 s; the second process no longer runs the struct phase's local part (6.2 s cold, 0 after)
or the types phase (0 of 110 strata).

The larger corpus (84 files, 6,410 calls agreeing, load about 90): 78.75 s CPU cold (321 s wall), 23.18 s warm, 20.28 s
the third time (33 s wall). Still open: warm runs find inputs changed every time — `doc, node` (34 of the types
phase's 110 strata) and `doc, types__wgen` (429 of P3's 481), so the 99-relation stratum costs 3.1 s warm.

This is the same for P2, for calls_cx's cross part and for the pack (t31 B4). t31's "column blocks" would be the
kept relations' encoding, not a format of their own.

### Stage 5. Caches in the file-local part; the existing caches moved (the owners, in order)

| Step | Change | Owner |
|---|---|---|
| 5.1 | Caches in the file-local part: plain rules there already; the per-file memo keeps their rows as any relation's | engine |
| 5.2 | The per-file memo as a cache by `(document, content, program)` (0103 §8) | engine |
| 5.3 | Each other cache of 0103 §8 | its owner |

### Stage 6. Strata the delta does not reach (the owners' work, measured here)

E1 maintains 92 of P2's 99 strata for a 3-row delta, 2.4 s against a full run's 2.9 s. 0101 §1 (skip a stratum
whose deltas are all empty) is the fix, in `plan::run`'s stratum loop, built by the hub's owners. This branch
re-runs `spike_t28_a_session_maintains_the_typing_phase` after it lands. Done when E1's P2 costs under 0.5 s.

### Stage 7. Searches (this branch, a decision first)

Generic instantiation (`inst`) and member lookup through the MRO (`mro_find`) as 0083 §4's searches, in place of
P3's eager `gpar`, `gbind`, `wbind`, `lgpar`, `lcbind` and `found`/`lfound` (`wbind` alone was 2.7 M join work
units for 23 rows, t27). A search's answers are the 0103 form exactly: the request is the key, the answer the
output.

```text
.cache mro_found by mro_ask
mro_ask(c, x) :- …                                      // the key: a class and a name some call asks for
mro_found(c, x, d) :- mro_ask(c, x), <the search>.      // the output: where the name is found
```

| Step | Change | Done when |
|---|---|---|
| 7.1 | A decision draft: a search is a program kind; its requests a key relation; a wave's new requests answered in one batch (0065 §2a's loop) | **drafted as 0107**, for review: a search is demand rules (a request relation, answer rules that read it), no program kind and no `.cache` |
| 7.2 | `mro_find` first: it replaces `found`/`lfound` and `need_name`'s patches | **measured, not taken** (0107 §5): as demand rules, `found` pulled six relations into `pv`'s recursion, +13.5% join work there (1.81 M to 2.05 M) for the same answers; the eager `found` was 279 units. P3 keeps it. The full corpus's 116 s (against 66 s, at load 110 against 36 to 76) was not measured again on the full corpus; on the 54 files, the same cold run took 38 s of main-thread CPU at load 75 to 97 and 29 to 32 s at 19 to 48, and the new path's cold run was no slower than the old (28.8 s against 31.9 s) |
| 7.3 | `inst` for the library generics (`lgpar`, `lcbind`, `lwbind`) | **measured, not taken** (0107 §6): on the 84-file small corpus, cold, `lcbind` 316 k, `lgpar` 281 k, `wbind` 103 k (t27: 2.7 M), `gpar` 69 k, `lfound` 5 k, `lwbind` 4 k of 30.9 M join work: 2.5% and 0.3 s together. The cost is `py_try` (4.5 M), `tv` and `tbv` |

### Stage 8. P3 on symbols (this branch, after 2.1 and 7.2)

- Workspace names resolve through `types__binds` and `types__member`, not `qv`/`rr_cx_reach`, which leaves P3
  with no cross-file reach of its own.
- Targets become symbols, placed by `struct__at` at the end.

That also gives the cross-file memo its finer key. This waits for per-definition node ids (Typing-support's
`granules.rs`), so positions stay out of what is compared.

## Order, and what each stage waits for

| Stage | Waits for | Where |
|---|---|---|
| 1 | — | done |
| 2 | nothing (2.1's string annotations: 5.1) | this branch, now |
| 3 | 3.1: nothing; 3.2: 5.1 | this branch |
| 4 | Two-pass's "derive again" merged (same code) | this branch |
| 5 | 1 | engine and the owners |
| 6 | 0101 §1 in the hub | the hub's owners; measured here |
| 7 | 1 | this branch |
| 8 | 2.1, 7.2, per-definition node ids | this branch |

## What this branch must not do

- Make a cache in any form but 0103's. `corpus::cache_architecture` fails on a new table or freshness call.
- Build its own writer, or 0101's skip.
- Keep anything in a session that the store does not also keep. `Session::phase_rows` is such a field today, and
  3.3 removes it.
