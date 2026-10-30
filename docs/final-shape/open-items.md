# Open items ledger

Status: live, started 2026-10-27. Each item says what is open, why it matters, how it will be
closed or validated, and where it stands. Items are added at the phase they are found and are
never deleted: a closed item keeps its row with the commit that closed it.

| Id | Found | Item | Status |
|---|---|---|---|
| O1 | Phase 1 | Crash recovery: pending writes lost in a crash must always be recomputed | closed (2026-10-27): marker, recovery and (1)–(3) validated |
| O2 | Phase 1 | Nothing calls the push path yet | open |
| O3 | Phase 1 | A new name is interned by a synchronous SQLite write | open |
| O4 | Phase 1 | Spill files are not recovered after a crash | open (depends on O1) |
| O5 | Phase 1 | Page collection can drop pages bound only in the overlay | open |
| O6 | Phase 1 | Overlay byte accounting is an estimate; single pushing thread | open |
| O7 | Phase 2 | `tid` and text-hash sidecars wait for the vocabulary | open (Phase 3) |
| O8 | Phase 2 | Tape format: v9np (structure-only, regular) is not decided or built; W runs v8 | open |
| O9 | Phase 2 | Kind histograms are digests only; their rows are not stored | open (Phase 3) |
| O10 | Phase 2 | Core facts of a non-UTF-8 blob, an unknown language or a parse stopped at a limit are `ABSENT` | open |
| O11 | Phase 6 | §12's "unbounded value cycle" has no definition of a bound; T's programs recurse through `@cat` | open, **decision needed** |
| O12 | Phase 6 | `nearest` has no executor after the native tape functions were dropped; Phase 6's check needs one | open, **decision needed** |
| O13 | Phase 6 | Symbols as atoms in the planner's SQL | SQL side done (decision 0009); native strata open |
| O14 | Phase 7 | `logic::plan` still runs its own stratum loop; it should emit wave steps for `waves::schedule` | open |
| O15 | Phase 8 | S's oracle (`native_oracle.rs`) needs S's `matcher/` deltas, which no copy row covers | copy placed (2026-10-29): S's oracle runs as copied on P's anchor engine (`findings::tests::s_native_oracle`, `matcher::p_anchors`); open only: which comparison survives (O19) |
| O16 | Phase 0 | 05 says port `mimalloc`; W already runs a tuned jemalloc | closed (2026-10-29): jemalloc stays the global allocator (user decision); `mimalloc` remains a dependency only |
| O22 | Phase 9 | P's text prefilter on three-valued clauses against W's consumers' pass | closed (2026-10-29): option (a), user decision |
| O23 | Phase 4 / 9 | Lowered matchers' node labels on Metal (06 S1, decision 0011) | open |
| O24 | Phase 4 / 10 | Semantic predicates as labels from fact producers (06 S1, decision 0011) | open |

## O1. Stale-fact detection after a crash

**Requirement.** If a process crashes, every write it held in memory (the overlay's top map, frozen
layers, spill files) is lost. Every fact that depended on a lost write must be recomputed on the
next run, always, without a full rescan. A crash may cost work but never a stale fact.

**Why it is open.** The push path records what it observed in the same place it records what it
derived:

1. `observe_root` binds the scope's root tree name (`{scope}#tree:`) and every rebound path and
   tree name. That binding is the observer's only memory of what it saw.
2. `propagate` then recomputes the readers of the rebound names wave by wave.

Both go through the same `Graph`. With the store directly, step 1 commits before step 2 starts.
With the overlay, layers are written back oldest first, so step 1's writes can reach SQLite while a
later wave's derivations are still in memory. After a crash at that point:

- SQLite binds the root to the new tree, but the derivations reading the rebound names still
  record the old digests.
- The next push compares the new root against the bound root, which is the same tree, so it
  rebinds nothing and `propagate` gets an empty batch.
- The stale derivations are invalid by the validity join (`derivation_valid`), but nothing asks:
  the join is only evaluated on demand for one output.

The same holds one wave down: a wave whose outputs were rebound and written back, but whose readers'
recomputation was lost, leaves readers that no rebound name will ever reach again.

**What must be validated.**

1. *Write-back is a prefix.* SQLite after any crash holds a prefix of the overlay's write sequence:
   layers are written oldest first, one transaction each, and a layer is dropped from memory only
   after its transaction commits. Test: abandon the overlay (no flush, no `Drop` write-back) after
   each layer boundary of a multi-wave push, reopen the store, and check that the state is the
   direct-store state after the same number of writes.
2. *Every invalid derivation is found on restart.* Test: for each crash point of (1), reopen and
   check that recovery's frontier equals the set of derivations whose recorded input digests differ
   from current bindings (computed by brute force), and that after recovery's propagation every
   derivation is valid and every output equals a clean from-scratch run.
3. *Crash points cover both stores.* The direct store (crash between observe and propagate, and
   between waves) and the overlay (crash with layers in memory, spilled, and partly written back).

**Proposed closure.**

- A `clean` marker in the store: cleared before a push writes anything, set only after the push's
  last layer is written back (`flush`). An open that finds it cleared runs recovery.
- Recovery is one statement, the validity join over all derivations: `derivation_inputs` joined to
  `name_bindings` on `name_atom`, keeping outputs where a recorded digest differs from the current
  binding (an unbound name reads as `ABSENT`). Its outputs seed `propagate` as the first frontier,
  so cutoff and waves work as on any push. The cost is one scan of `derivation_inputs`, paid only
  after an unclean shutdown.
- Observed names are rebound, never recomputed, so recovery cannot recompute them. They are safe
  because the observer re-diffs from its bound root, and a write-back prefix never holds a root
  binding without the path and tree bindings bound in the same or earlier writes.

Until (1)–(3) pass, the overlay must not be used outside tests.

**Closure (2026-10-27).** Built as proposed:

- `push_pending` (schema): the marker. `ContentStore::begin_push` commits its row before a push writes
  anything. `end_push` deletes it; the overlay's `end_push` flushes first. `push_root` calls both, so a
  push that fails or crashes leaves the row.
- `ContentStore::invalid_derivations`: the validity join over all derivations, one statement.
- `observe::recover`: when the marker is set, the invalid derivations are the first wave, then
  `propagate` from the outputs that changed, then `end_push`. Called on open, before any push.
- Tests, `facts/observe/tests/recovery.rs`. The push from V1 to V2 takes four waves, and each run
  crashes at one of them.
  - Through the store: covers a crash between observe and propagate, and between waves.
  - Through the overlay, with every write freezing one layer: layers kept in memory, spilled,
    written back one or two behind, and written by the background writer.
  - After each crash: (1) SQLite equals the direct push after exactly `written` of its writes.
    (2) Recovery's frontier equals the brute-force set of derivations whose recorded digests differ
    from the bindings. After recovery and the push run again, every derivation is valid and the
    store equals a push that never crashed. With recovery's frontier emptied, both tests fail.

Still open: spill files left by a crash are not removed (O4). Recovery does not read them.

## O2. The push path is not wired

Nothing outside tests calls `observe::push_root`. The observers (W `git_workspace.rs`, the
filesystem watcher, editor buffers) are wired into the workspace scan when Phase 2's first consumer
(core facts) exists. Closure: a workspace scan pushes its root, and core facts are derivations that
read `#path:` names.

Phase 2 update: the consumer exists (`facts::families`: `#core:` reads `#path:`, `#kinds:` reads
`#core:`). Wiring waits on two things. First O1: a scan that pushes through the store directly has
the same crash hazard as the overlay (observe commits before propagate). Second, the
`git_workspace` tests build temp repos with `git add .`, which the agent git hook blocks, so the
wiring cannot be tested in this environment; it needs a run outside the hook.

Phase 3 update: two more consumers wait on the same wiring. `prefilter::publish` (a tape's
prefilter rows, `kind_size` and `edge_freq`) runs only in tests, and `prefilter::select` has no
caller, so nothing captures a production batch for the anchor lab. Phase 3's check ("`anchor_lab`
steps no worse than P on the same captures") therefore cannot run yet: P's captures are P's private
store, which the lab no longer reads. Closure adds: publication in the push path when a tape is
stored, then a corpus run with `RERAIL_SQL_PROFILE_DIR` to capture batches, and the lab and
`candidate_replay` over them against P's numbers.

## O3. Synchronous interning

A new name is interned into `atoms` with one SQLite transaction per batch, under the write lock.
Warm edits rebind existing names; a cold scan pays one write per batch. Closure: measure on a cold
scan of the gate corpus; if it is visible in CPU time, intern ahead in the overlay with a reserved
range and write the atoms with the first layer that uses them.

## O4. Spill files after a crash

Spill files are not reopened after a crash; SQLite stays consistent and their layers are lost.
This is correct only if O1 holds: lost layers must be recomputed. Closure: O1, plus deleting stale
spill files on open.

## O5. Page collection and the overlay

Reclamation deletes `tree_pages` whose digest no `name_bindings` row holds. A page bound only in an
unwritten overlay layer can be deleted; the next lookup misses and recomputes it, so this costs work,
not correctness. Closure: collect only after `flush`, or take the overlay's bound digests as extra
roots.

## O6. Overlay accounting and concurrency

The overlay counts bytes by an estimate per entry and assumes a single pushing thread (readers may
be concurrent). Closure: measure real resident bytes against the estimate under the 500 MB budget;
a second pusher needs per-scope ordering of layers.

## O7. `tid` and text-hash sidecars

`core::Sidecars` holds `par`, `cidx`, `cidx_named` and line starts. `tid` (each token's id in the
language vocabulary) and text hashes need Phase 3's vocabulary. Closure: Phase 3 adds them to
`Sidecars`, keyed by (tape digest, vocabulary length) as 03 §16 says.

## O8. Tape format

**Two different "v8"s.** P's tape spike (2026-10-25, `research/tape-v8-measurements.md`,
`experiments/tape-v8/`) measured a "v8" proposal (SoA columns, absolute u16/u32 positions) and
rejected it: 11.1–13.5 B/node, 1.24–1.59× v7 (1.47–1.90× after zstd). W's `TMPTAPE8` is something
else: v7 plus N's supertype table (`c3dd82e4`, `3cf93edb`). No other tape format past v7 exists in
any worktree, branch or repo under `~/orca` (searched 2026-10-27).

**Target.** P decision 0003 (`decisions/README.md`): **v9np**, a structure-only, regular,
column-stored tape. `kf` u16 with flags folded into the kind code space (named, extra, missing,
error and contextual aliases as distinct codes); `start(i) = base[i/16] + off[i]`; `end(i)` from
the start of the node's last descendant plus `tail[i]`; `size` u16/u32 per document; no stored
parent (rebuilt with a stack where a consumer needs it). Measured 8.25–8.55 B/node (0.93–1.01× v7;
zstd 0.94–1.06×). Gains: random access (v7 113–490 ns → v9 23–199 ns), random spans, SQL span
extraction 2.7–8×, and columns that upload to JIT/Metal as they are, without restoring into
`MetalNode` (v7 restore 3.7–7.8 ns/node). Sequential scans are not faster. Status: **proposed,
pending pre-lock measurements**, never accepted.

**Requirements from the discussion** (P session 2026-10-25):
- The format must be completely regular. v4 to v7 were irregular in intermediate versions (overflow
  pairs, sorted flag lists, alias tables), and it doesn't work unless it is regular.
- ast-index-spike experimented with multiple trees for different ranges within a file. That has to
  be taken into account (embedded documents, Blade, Vue).
- Don't overload the tape with things only associated with the source structure. Per-file alias
  string tables, the digest and length header, and Blade's omissions go to sidecars keyed by the
  content digest, or are recomputed. Derived-document maps and host errors already live in
  `file_documents`.
- Blade stores nothing more or less than the tree-sitter-php tape it replaced. Today its
  compiled-view tape drops nodes over synthetic placeholder segments, so a directive call has no
  `function` child. Proposed fix: the plain tree-sitter-php tape, with omission through the segment
  map sidecar. Error spans stay in the tape as tree-sitter emits them, as ast-index-spike does. It
  is fine for Blade to store something compatible with the generic structure, in a generic way.
- Tapes must stay processable in SQL with ast-index-spike techniques. A format change has to show
  clear benefits; a size increase (8 → 11 B/node, about 30%) was not acceptable, hence v9np.
- Tempo keeps cached, pinned, page-allocated uncompressed tapes and source (§ residency, O5). The
  at-rest format and the resident page format should be designed together.

**Pre-lock measurements** (`research/tape-regularity.md` "Open measurements"). The spike used
crates.io grammars, not Rerail's vendored and patched ones, so each must be rerun on Rerail's:
1. Alias and folded code-space counts per Rerail grammar (≤ 1023; the spike saw 194–277).
2. Whether `extra` is uniform per kind in Rerail's grammars.
3. At-rest zstd size against v7/v8 on the full data-layer repository and CPython.
4. GPU: column upload against `MetalNode` expansion, end to end on the per-edit gate.
5. Width distribution: documents above 65,536 nodes or 64 KiB of source (spike: 24 of 6,251).

**Not yet decided:** where W's supertype memberships go in v9np (a column, or a sidecar keyed by
the tape digest, since they are a grammar property of the structure); whether the parent column is
kept (only if SQL parent climbs prove hot: a climb was 1.6–3× faster than interval containment).

**Closure.** Run the five measurements on Rerail's grammars; record an accepted decision (cite
0003 and the spike indexes); then copy P `experiments/tape-v8/src/{main.rs, sql.rs}` into
`cr/tape/v9np.rs` and adapt (05 Phase 2). Until then everything reads tapes only through
`TapeDoc`'s `transducer_*` accessors, so the switch stays local to `tape.rs`.

## O9. Kind histograms are not stored

`#kinds:` derivations record the histogram's digest. The counts themselves are recomputed from the
tape when read. Closure: Phase 3's `file_kind_counts` (the prefilter's access path per file) stores
them, written by the kinds derivation.

Phase 3 update: `tape_kind_counts` exists (count, descendant sum and preorder postings per tape and
kind atom), written by `prefilter::publish`. What remains is O2: publication on the push path, after
which `#kinds:` can read the counts instead of decoding the tape.

## O10. Files without core facts

A path whose blob is not stored, is not UTF-8, has no dialect, or stops the parser at a limit gets
core and kinds outputs of `ABSENT`. That is consistent (an edit that makes it parse rebinds it), but
a rule must treat `ABSENT` as unknown, not as empty. Closure: Phase 8's findings layer reports these
files as Unknown under the loose contract.

## O11. What bounds a value-creating cycle

§12 rejects "cycles through value-creating builtins with no bound" but does not say what a bound is.
T's own programs have such cycles: `qn($n, @cat($q, $a)) :- attr_of($n, $o, $a), qn($o, $q)` in
`calls.rdl`, and likewise `qv`, `qopen`, `modn` (`calls_rust.rdl`) and `resolves` (the workloads).
Each terminates only because it recurses along a tree edge (`attr_of`, `field`, `nearest`): finite
and acyclic. A cycle through `@cat` over a graph with a cycle would create names forever.

Options: (a) a relation declared acyclic (`.acyclic field, child`) bounds the cycle when every
recursive step joins through one; (b) recursion through `nearest` or a tree-edge input counts as
bounded by construction, because tree structure is not a relation (§12); (c) a depth bound,
`widen`-style, per value-creating rule. Until this is decided, `logic::front` does not raise the
error (`front.rs` header). Closure: the decision, then the check, with T's five programs passing.

## O12. An executor for `nearest`

T ran `nearest($n, $s, set)` with `tape_nearest` over a per-run copy of the tree (`tape_load`), and
its join steps `near`/`down`. Phase 6 row 1 dropped them, because file-local work runs in the
transducer (Phase 4 item f) and tree structure is not a relation. Every one of T's programs uses
`nearest`, so the Phase 6 check (T's goldens reproduced) needs an executor. Options: (a) lower
`nearest` with the other file-local rules into transducer programs (Phase 4 f), with its facts
arriving as seed rows; (b) a CPU binding over W's tapes, read from the slab cache (Phase 9), kept as
the reference semantics as `flat` is for lattices. (a) is the target shape; (b) would let Phase 6's
check run before Phase 4 f and Phase 9. Closure: the decision, then the Phase 6 check.


Note (2026-10-27): `logic::plan` ports T's SQL lowering of `nearest` too, a recursive CTE over a
`tape(n, par, size)` input relation, so T's programs run end to end over T's facts (row 5 step 1's
goldens). That is (b) in SQL, over a per-run copy of the tree, and not the target shape.

## O13. Symbols as atoms in the planner's SQL

`logic::native` reads and writes symbol columns as store atoms (C12; row 1 step 3), while
`logic::plan` (row 5 step 1) runs T's SQL over text symbols, as T's facts have them. A run that mixes
native strata with SQL strata would join atom ids against text, so the native strata
(`native_stratum`, `native_set_fixpoint`, `native_fixpoint`, `join_spec`, `observe`, `live_stats`)
are not ported yet. Design for the next step: inputs are fact families whose symbol columns hold
atoms (§10.6), as the product stores them; the run's connection is the content store (its `atoms`
table, with derived relations in `temp`); string constants are interned at lowering, so the
planner, `stat_val` and `stat_kind` all key on atom ids; `@cat` and the `rdl_int` split become
functions over the same interner (`setfix::Interner`), flushed after each statement; outputs are
decoded through `atoms`. The test fixture is T's, atomized at load. Closure: that step, with the
native modes' outputs equal to the SQL goldens.

Status (2026-10-27): the SQL side is done (decision 0009): `logic::plan::lower` and `run_atoms`, over the
base-relation vtabs (decision 0008) with T's statistics atomized, give T's plans and outputs on the fixture
(`t_programs_over_atoms_plan_and_run_like_t`). Open: the native strata over the lowered program, and the planner's
statistics from `stats::ingest` instead of T's tables.

## O14. The planner's stratum loop and the frontier loop

`waves::schedule` (decision 0010) runs wave programs: steps that insert into `new_<relation>`,
read `delta_` relations of their own stratum, and are selected by SQL wave by wave. `logic::plan`
(`run`) still runs its own loop: semi-naive rounds per strongly connected component, with
`CREATE TEMP TABLE … AS` per round, and `count(*)`/`count(DISTINCT)` refreshes between strata (DLRE
trap T18). Lattice relations (`merge_lattice`) have no step form yet. Closure: the planner emits
a `schedule::Program` (one step per rule and delta variant, its stratum from `strata`), with
lattice merges as a fold kind, and T's programs give the same outputs through `schedule::run` as
through `run_atoms`; the planner's counts then come from `rows.<relation>` at the boundary.

## O15. S's oracle needs S's matcher deltas

Phase 8 row 5 keeps S's async-call oracle as a test. Its `check()` compares every result with
`native_oracle::Native`, which uses S's matcher API: `Corpus`, `CorpusSlot`, `anchor_plans`,
`FileAnchorIndex::build_at`, `GrammarMatcher::admits_source` and `invalid_syntax`,
`AnchorPlan::needs_index`, and `phase_clock`. S changed `matcher/*` relative to main (57 files,
+11.5k lines, with `phase_clock.rs`). Rule 4 says such deltas are ported onto W's version, but no
row of the plan lists them. So the oracle stays outside the module tree (an attempt that remapped
its other imports is in the stash `wolfeel-oracle-placement-attempt-2026-10-27`). The fixture
paths in `tests.rs` also need one `..` fewer. Closure: add a row for S's matcher deltas that the
oracle needs, or replace `Native` with W's matcher as the comparison.

2026-10-27: why this stays blocked under copy-the-closure. Measured against its merge base with main
(f3f92c24), S's matcher delta is 75 files, +14.5k/−0.7k lines. Half of it is P's
`anchors/sql/*` (`lower/{children,waves,lifetime,ownership,render}.rs`, `reader.rs`, `tree.rs`,
`store.rs`, …). `Native` reaches that half through `anchor_plans` and `FileAnchorIndex::build_at`.
05's "Explicitly not copied" excludes the D and P `anchors/sql/lower*` relation evaluation, and
Phase 3 ported the rest as `prefilter/` with those parts dropped. Copying the closure would put
excluded code back, so this needs a decision, not a copy: (a) a row for the oracle's closure,
with an exception to the exclusion for test-only use, or (b) `Native` rebuilt on W's matcher and
`prefilter::select`.

2026-10-28: placed with option (b), as a mutation of the copy. `findings::async_reference` (test-only)
compares against W's matcher: `FileAnchorIndex::build` per file (its CPU in `prepare_ns`, so
`index_ns` is 0), `select` mapped to node ids, the required-text check inline, and no
`invalid_syntax` gate (W's DSL has none). Its tests pass, including the two native comparisons.
Still open: whether the oracle should instead compare against `prefilter::select` on the content
store, once that path runs clauses.

2026-10-29: S's oracle as copied is placed beside that adaptation as `findings::tests::s_native_oracle`
(O19): with P's anchor engine in the tree (`matcher::p_anchors`, O20) and P's `invalid_syntax` gate
ported (O18), it runs S's `Corpus`/`anchor_plans`/`build_at`/`selected` path as written; the only
mutation compiles each grammar's P plan from its matcher and keys it by W's plan. `check` asserts
both oracles match the stateless native.

## O16. `mimalloc` against W's jemalloc

Phase 0 lists `mimalloc` among the dependency deltas. G installs it as the global allocator on
unix (`static GLOBAL_ALLOCATOR: mimalloc::MiMalloc`, `mimalloc =0.1.52` without default features,
and `cranelift-jit` with `selinux-fix` so JIT code gets its own mappings). W already has a global
allocator: jemalloc on non-Windows (`native/src/lib.rs`), with `dirty_decay_ms:1000,
muzzy_decay_ms:0` (immediate decay cost a tenth of the data-layer benchmark's CPU), a purge after
every apply and again when its response is freed, and Tree-sitter's allocator routed through it
(the system allocator served a parse at 10%). A crate has one global allocator, so this is a
choice, not a port. Keeping jemalloc until it is decided: replacing it means measuring CPU and RSS
on the data-layer gate (M1) with each, including the purge behaviour that keeps nothing dirty at
rest. Closure: keep jemalloc and drop `mimalloc` from 05, or measure and switch.

2026-10-29: the dependency is ported (`6ba8fe1d`, with `cranelift-jit`'s `selinux-fix`); only the global
allocator choice remains.


## O17. N's contract suites against W's semantics

N's `query/mod.rs` is placed as the test-only `demand::driver::catalog`, with N's relational
lowering (`plan/`), N's `sql_backend.rs` and N's two contract suites. This is a deviation: 05's
"Explicitly not copied" excludes the relational lowering, but placing the suites needs it, so it is
compiled under `cfg(test)` only and leaves production `demand::vtab` alone. 184 of 186 tests pass.
Two are `#[ignore]` because W's behaviour differs from N's:
`abstract_kind_terminals_execute_from_tape_class_memberships`, because W's compiler rejects
abstract supertype kinds such as `expression` (N lowered them through tape class memberships), and
`production_contract_phase_one_oracles_match_committed_results`, because W's failure for
`clause-syntax-error-unknown` (`def broken(` under `not has identifier`) differs from N's
committed `source-partial` oracle. Closure: decide whether W adopts N's semantics (supertype
lowering, error-region reporting), then re-enable or update the goldens. Also decide whether the
relational lowering stays test-only or replaces `demand::vtab`'s lowering.

## O18. P's relational stage, placed test-only

Placing P's `batch.rs` (Phase 9 row 2) brought its closure: P's relational stage (`relational.rs`,
`relational/**`, `compile/relational.rs`), now `residency::admission::relational` under
`cfg(test)`, with `ClauseKind::Relational` and the `relational` clause form compiled only under
test. A relational clause carries SQL, which rules do not (the findings contract), so it is not a
production form. Mutations to fit W, each failing closed where W lacks P's piece:
- inputs go through P's typed sink (`inputs.rs`, restored in `b6380d7c`), fed from
  `stateless_rows`' witnesses after the relation is read rather than streamed through P's
  `stateless_rows_to`; W's JSON projection reaches the sink as a typed row
  (`Captures::take_projection`, test-only). P's sink tests pass. Still outstanding, to restore
  with P's `FactQuery` shape: `narrowing_a_consumer_keeps_the_shared_ast_and_unmodified_schemas`
  (P keeps the projection off the shared AST, W does not) and the half of
  `sql_facts_keep_typed_values…` that sets `FactQuery::anchor`;
- every input anchors at the witness span (P's `FactQuery::{anchor, each, closure}` are not in W);
- `references` (P's native reference graph) runs: P's `calls` delta is ported onto W's `calls`
  (3-way merge on P's true merge base with W, `5cf4246e`: `closure.rs`, `mod.rs`, `summary.rs`,
  `php.rs`, `tests.rs`, `contracts.rs` now in production), with only its call-site changes
  elsewhere (`Through` → `Mode` in `CallsTerm`/`Terms::questions`, `CallsView::build_modes`, the
  collector's regex bound, `matcher::file_path`, `ClauseKind::calls_query`, `workspace/calls_state.rs`
  whole). The summary extractor is P's version 18 (reference tables). P's and D's stages compile
  the `references` form and read `reference_edges` as P and D wrote them. P's other deltas to
  `compile`, `eval`, `matcher/resolve.rs` (declaration sites) and `workspace/global.rs` are not
  ported, so a workspace still ignores relational clauses;
- no `CorpusSlot` or `TapeCacheSlot` (W's matcher builds per-file anchor indexes and has no tape
  cache); the workspace path (P's `workspace/global.rs`) is not ported, so a workspace ignores the
  clause.
P's `"syntax":"valid"|"invalid"` AST gate (`GrammarMatcher::invalid_syntax`, `admits_source`) is
ported into W's DSL (compile, `eval/query.rs`, `workspace/{evaluate, results}.rs`, as P wrote it),
so the copied test policies carry it again. No assertion is removed: the two tests that need P's
`FactQuery::{project, anchor}` shape run `#[ignore]`d. `calls/contracts.rs` is in production with
P's `calls` delta; `violations/projection.rs` is placed test-only and unused. Closure: decide
whether any of the stage survives as an engine-internal consumer (Phase 11 row 1, B2, rewrites the
admission loop rather than copies it) or is removed once the census and admission rules are carried
by Phase 7's chunked loop.

**2026-10-29: P's workspace path is ported, test-only.** P's deltas to `workspace/{global, api, mod,
results, evaluate, documents, ingest, crossfile, memory, tests, context_tests}.rs` and their closure
(`compile/{mod, evidence, line_state}.rs`, `eval/{mod, query, evidence}.rs`, `model.rs`,
`query/{mod, line_state, name_closure}.rs`, `violations{.rs, /semantic.rs}`, `content_store/objects.rs`,
`matcher/{semantic, binds, parse, resolve}.rs`, `literal/{identifier_words, python_fstring}.rs`
and 40 smaller files) are three-way merged onto W (rule 4, base `5cf4246e`), W's side at every
conflict. A workspace now compiles and evaluates a relational clause under test: producers from
`compile_relation_producer`, batched across rules by `evaluate_batched_relations`, joined into
`evaluate_global`. Everything that reaches P's stage (the `Relational` arms, `scoped_source_bytes`,
`Document.syntax`, the corpus upkeep in `ingest.rs`, `anchor_corpus`, `file_relation_results`) is
`cfg(test)`, so W's production path is unchanged. The earlier bullets' "a workspace ignores the
clause" no longer hold under test. P's typed `Captures` and projection rows replace W's JSON bridge
(`violations::projection` is now compiled in production, as P wrote it); P's sink, anchor and
projection hunks are restored in `residency::admission::relational` and `waves::relational`, so
the two `#[ignore]`d tests above run again. Adaptations at conflicts, recorded so they are not
re-decided:
- W keeps its `!three_valued` guard on a matcher's required text (`compile/mod.rs`): P's line
  computes it for clauses that read values, which refuses a consumers' pass on a file without the
  text (W's `an_evicted_file_kept_for_a_consumer_that_ran_out_keeps_its_other_results`). P's line is
  kept beside it as a comment, and P's test of it is ignored (O22);
- P's anchor-corpus tests (`positional_index_lives…`, `stateless_fact_extraction_uses_selective_anchor_postings`,
  `relational_inputs_share_one_anchor_index…`) run under `p-anchor-corpus` and are ignored without it (O20);
- W's retained-bytes profile fields (`record_retained`) are kept beside P's profile;
- P's duplicates of W items (a second production `Relational` variant, `calls_query`, the tape's
  `mod index` block, a by-value `index_source_digest`) are not taken twice.

## O19. Copies placed beside their adaptations

Restoring copies that later commits rewrote or removed put the copies in the tree, as copied,
beside the adaptations made from them:
- D's relational stage (`c759569d`, `8d3c3137`, `e2789364`, `1e28006e`) is `waves::relational` under
  `cfg(test)`, next to P's (O18). `CompiledClause::data_layer` carries D's plan for a `relational`
  clause, compiled by D's compiler, and `eval::data_layer_rows` reads its inputs. Mutations, each
  failing closed: no `matcher::Corpus`/`CorpusSlot` (the owner is an open flag, the workspace is
  W's `CodeRuleDslWorkspace`); a plan reads `reference_edges` from P's ported reference graph
  (O18); the syntax digest hashes the source (W's tape keeps none). The restored
  `waves/owner/{pool, execution}.rs` and `waves/schedule/batch{,/tests}.rs` are D's stage's own
  files. Two cut tests run again `#[ignore]`d (O18's projection items).
- M's `lmap` (`951514fe`) is `facts::lmap`, beside `facts::overlay`, its adaptation.
- M's Y1 reference graph (`758cfa6a`) is `facts::ygraph`.
- M's facts-a `Facts` with `bindings` and `typedefs` (`66a0d6a1`) is `facts::facts_a`, beside
  `facts::core`, its adaptation; `Grammar::field_name` reads W's private kind catalog for it.
- G's admission and cutover spikes (`f5de8998`), positional search (`positional_search`) and
  literal anchors (`packed::literal_anchors`, `packed::sql_backend`) (`b56d8889`), and tree result
  segments with snapshot trees (`content_store::tree_result_segments`, `ce1b1584`), beside the
  content store and packed transducer that adapted them.
- D's projection (`facts::d_projection`, `06abf031`) beside `facts::runs`.
- D's anchors module and SQL anchor engine with statistics (`matcher::d_anchors`, `c857812b`)
  beside W's `matcher::anchors` and `prefilter`, as P's (O20).
- T's SQLite extension entry (`logic::native::extension`, `eceb3b3e`) and its own set engine
  (`extension::setfix`, `45ae696b`) beside `logic::native` and `logic::native::setfix` (symbols as the
  store's atoms). The planner's native modes run on T's pair in tests; production would run them on
  the adaptation once the planner's native strata read atoms (O13).
- S's async-call oracle as copied (`findings::tests::s_native_oracle`) beside `native_oracle` (O15).
- P's census (`workspace/memory/census.rs`) is `residency::admission::census` again, verbatim, now
  that P's typed projections are in the tree; `2d41e7d2` had cut it onto W's JSON projections.
- P's and D's anchor corpora (`p_anchors/sql/corpus.rs`, `d_anchors/sql/corpus.rs`) and their
  `Semantic::FilePath` arms are restored as written (D's with only the `waves` path changed); W's
  `matcher::anchors` gains the FilePath arm.
Closure: per pair, decide which survives. The adaptation carries the plan's shape, so the copy is
removed (a mutation, after its tests are carried over or retired), or the copy replaces it.

## O20. P's anchors module and SQL anchor engine, placed test-only

P's `matcher/anchors.rs` and `anchors/**` (`2814d875`, with `faec6fa4`'s `sql/**` and `8a6a69f1`'s
`fields.rs`) are `matcher::p_anchors` under `cfg(test)`, beside W's `matcher::anchors`. W's
`prefilter` is the adaptation of P's `sql/**` onto the content store (rows `8165bd54`,
`0afc80d4`), so P's private store, session, schema, decoded lowering and preparation tests run
here as P wrote them. Mutations:
- W's matchers carry W's `AnchorPlan`, so P's engine compiles P's plan from the matcher
  (`AnchorPlan::compile`) where it read `matcher.anchors`;
- P's `admits_source` and `invalid_syntax` gate are ported into W's `GrammarMatcher` (O18), so
  P's and D's plans call it as written; P's `Semantic::FilePath` arm is dropped;
- test-only additions to shared code: `TapeDoc::index_source_digest` (computed: W's tape keeps no
  digest), `TapeNode::incoming_field`, `test_support::Walk::AnchorBuild`.
`parallel_connections_do_not_share_a_global_page_cache_mutex` is ignored: W's SQLite keeps
`ENABLE_MEMORY_MANAGEMENT`, which P's `.cargo/config.toml` undefines. Closure: decide whether W
takes P's SQLite flag (it is a process-wide setting), and carry P's decoded lowering into
`prefilter` (or retire it with relations moving into transducers, §7) before `p_anchors` goes.

**2026-10-29: P's anchor-corpus path, behind a feature.** P's workspace swaps W's per-file
`FileAnchorIndex::build`/`select` for its corpus (`FileAnchorIndex::build_at`/`selected`,
roots prepared for every rule before the workers run). That swap is compiled only under
`cfg(all(test, feature = "p-anchor-corpus"))`: `workspace/evaluate.rs` keeps both `run_jobs`,
`prepare_candidates` and `evaluate_document` (W's under `not`), and `eval/query.rs::anchor_roots`
answers `None` without the feature. P's corpus keys prepared roots by its plan's `Arc` address, so
`matcher::p_anchors::plan_for(grammar)` maps W's plan to one P plan per matcher (a test-only bridge).
`positional_index_lives_with_the_workspace_across_applies_and_deletions` runs with the feature and
is ignored without it. Build: `cargo check --lib --tests --features p-anchor-corpus`.
Closure: the corpus swap is the architecture's per-run anchor index (point 1); decide it with
Phase 3 rather than by default.

## O21. G's literal anchors and positional search index, placed test-only

`bb4c9605` merged G's grid closure (`b90a7b0a`) into N's packed runtime but kept W's removals of
G's search index and literal anchors, so seven G tests in `packed.rs` and two in
`metal_semantic_conformance.rs` were `#[cfg(any())]`. They now run with their closure:
- G's positional index, literal and kind anchor loaders and literal materialization
  (`transducer.rs` lines 3395–3783, `1c040f4a`) are `transducer::packed::literal_anchors`, and G's
  SQL match vtab is `transducer::packed::sql_backend`, both under `cfg(test)`;
- G's store side (`f9f175f3`) is `content_store::positional_search` under `cfg(test)`: G's value
  and text FTS streams and kind instance vocabulary, created on first use beside W's kind-only
  `search_kind_fts`, published by `put_positional_search_indexes_bulk` (W's
  `put_search_indexes_bulk` takes W's index). `rusqlite`'s `array` feature is a dev-dependency
  feature for G's `rarray` statements.
- The semantic differentials use every node as a root: G's anchor plans read G's
  `AstMatcher::Program`, which W's matcher does not have. Both are `#[ignore]`d corpus runs.
G's admission and cutover spikes (`00229423` had deleted them) are back as copied.
Closure: W's search index is kind presence by design (Phase 3 row 5); decide whether G's value
and text streams feed the prefilter's term postings or are removed with these tests.

## O22. P's text prefilter on three-valued clauses

P computes a matcher's required text for every clause (`compile/mod.rs`: "requirements read only
source-text tests, never values, through monotone connectives"). W computes it only for two-valued
ones (`!three_valued`). They disagree on a consumers' pass. In
`an_evicted_file_kept_for_a_consumer_that_ran_out_keeps_its_other_results`, P's prefilter refuses
`impl.py` (no `async `), so the file records no failure and is evicted. The result then reads its
stored results, and with the injected read failure the whole apply fails instead of reporting
`adapter-error`. The test fails in P's own tree at `cbee7a03` with the same panic
(`crossfile_tests.rs:224`), so this is P's defect, not the merge's. W's guard is kept in
production; P's `a_three_valued_rule_keeps_its_source_prefilter` is `#[ignore]`d with this reason,
and P's line is kept as a comment beside W's. **Decision needed:** take P's prefilter (and make an
evicted read failure on a consumers' pass non-fatal, as `api.rs` already does for one file), or
keep W's guard and retire P's test.

2026-10-29 (O19, placement): rows 119 and 224 of 05 now have modules at their planned paths.
`prefilter::vocabulary` names `content_store::vocabulary` (P's vocabulary functions, made
`pub(crate)`). `residency::metal` names `transducer::packed::metal_backend` (G's no-copy buffers,
the module and `MappedBuffer` widened to `pub(in crate::code_rule_dsl)`). The code stays where its
closure was placed, so nothing is copied twice.

2026-10-29, O16 closed by the user's decision: W's tuned jemalloc stays the global allocator.

2026-10-29, O22 closed with option (a), the user's decision: P's prefilter is taken for every clause
(`compile/mod.rs`), and P's test runs again. An evicted file whose stored results cannot be read
at result time reports each candidate clause as `adapter-error` (`STORED_RESULT_UNAVAILABLE`),
transient, rather than failing the apply (`results.rs::evicted_file_states`). `api.rs` treats such
a transient state as a failed read, so the next apply that offers the file evaluates it in full.


## O23. Lowered matchers' node labels on Metal

**Requirement.** Rule F1: no operation runs on one backend only. A lowered clause (decision 0011)
tests its node labels with `SUPERTYPE`, which both interpreters execute. The labels themselves are
per-tape bitsets the CPU interpreter reads through `TapeDoc::with_labels`.

**Why it is open.** The Metal backends read the class table they uploaded with the tape, which has
no labels. The label passes (complement, constrained pattern, fixpoint rounds) also run on the CPU.

**Direction.** Upload the label bitsets as a column beside the class table, and run the passes as
kernels in one command buffer: a complement is a word-parallel not, and a program pass is the
existing dispatch. GPU waves cost about 1.2–1.6 µs each, against 386–890 µs for a host round trip
(metal-enqueue notes), so no pass may need the host between passes. Check: the S1 gate's root sets
are equal on CPU and Metal (Bend's rule that every binding must give the same keys).

**2026-10-30, S2:**
- *Done* (`metal_program_backend::label_passes`):
  - the labels sit in a prefix of the supertype buffer, which `has_supertype` reads;
  - every pass is a GPU kernel, pattern leaves included (one generated kernel per clause);
  - a clause is one command buffer with no host waits; recursive rounds are encoded and skipped
    on the GPU once settled;
  - the parity gate passes for 87 of 87 clauses.
- *Still open:*
  - launches average 36–69 per clause across the default sample's batches, but a clause with a
    recursive group of unknown kinds takes up to 559; skipped rounds still count. The
    under-100 target per evaluation is S4's check;
  - label-domain programs start at every node;
  - each clause is its own evaluation until S4 batches a wave's clauses.

## O24. Semantic predicates as labels from fact producers

**Requirement.** 209 of the 286 Python `ast` clauses are refused by S1's lowering. Most of them
read semantic predicates:

| Predicate | Clauses |
|---|---|
| `resolves` | 94 |
| `parse` | 13 |
| `arguments` | 13 |
| `resolves.binding` | 11 |
| `formatArgs` | 6 |
| `value` | 6 |
| `binds` | 5 |
| `layout` | 2 |

The rest are refused for `count` (25; since S2, 10 lower and 15 are refused for the semantic leaves inside them), shared bindings (18), constraints below the top level
(12), parameterized utilities (3) and comparisons (1).

**Direction.** A semantic leaf is a node label produced by its fact producer
(`transducer::semantic_facts`, rows before SQL composition), not an operation inside the program.
Its sub-rules are lowered as labels of their own. Shared bindings are joins on capture columns
between labels, in the wave's SQL.
