# Open items ledger

Status: live, started 2026-10-27. Each item says what is open, why it matters, how it will be
closed or validated, and where it stands. Items are added at the phase they are found and are
never deleted: a closed item keeps its row with the commit that closed it.

**Ids.** From 2026-11-08, a new item takes a mnemonic id, `O-<area>-<topic>` (`O-artifact-store`,
`O-git-merge-base`), not the next number. Branches drift apart in numbers (on 2026-11-08 one was at O146 and the
others at O143) and collided three times in a day (0094 §9); a name says what the item is and two branches
choosing the same one are writing about the same thing. Numbered items keep their numbers.

| Id | Found | Item | Status |
|---|---|---|---|
| O1 | Phase 1 | Crash recovery: pending writes lost in a crash must always be recomputed | closed (2026-10-27): marker, recovery and (1)–(3) validated; 2026-11-01: called by every slice run before its push (`a_run_after_a_crashed_push_recovers_it`) |
| O2 | Phase 1 | Nothing calls the push path yet (ingest wired in S3) | closed for the observers (2026-11-01): the slice's napi `CodeRuleSlice` pushes a directory snapshot with editor buffers over it, or a Git `HEAD` (W's `git_workspace` discovery); the anchor lab's corpus capture moves to Phase 3's check |
| O3 | Phase 1 | A new name is interned by a synchronous SQLite write | closed (2026-11-06): decision 0049, atoms are arena segments written with the rows that name them; 3 appends per full-corpus run instead of 104 commits |
| O4 | Phase 1 | Spill files are not recovered after a crash | open (depends on O1) |
| O5 | Phase 1 | Page collection can drop pages bound only in the overlay | open |
| O6 | Phase 1 | Overlay byte accounting is an estimate; single pushing thread | open |
| O7 | Phase 2 | `tid` and text hashes | closed (2026-11-02): decision 0026, read from postings and the tape; nothing per node |
| O8 | Phase 2 | Tape format: v9np (structure-only, regular) is not decided or built; W runs v8 | closed (2026-11-01): decision 0025, v9np (user decision; measured on Rerail's grammars); `tape.rs` stores v9np (format 10), supertype memberships folded into the codes (0025 amended 2026-11-02: no table beside the tape); 0.72–0.92× v8's stored size |
| O9 | Phase 2 | Kind histograms are digests only; their rows are not stored | open (Phase 3) |
| O10 | Phase 2 | Core facts of a non-UTF-8 blob, an unknown language or a parse stopped at a limit are `ABSENT` | open |
| O11 | Phase 6 | §12's "unbounded value cycle" has no definition of a bound; T's programs recurse through `@cat` | **closed** (2026-11-03): option (b), `logic::front::cycles` |
| O12 | Phase 6 | `nearest` has no executor after the native tape functions were dropped; Phase 6's check needs one | **closed** (2026-11-03): option (a), `typing::file_local` |
| O13 | Phase 6 | Symbols as atoms in the planner's SQL | **closed** (2026-11-03): SQL side (decision 0009), native strata (list step 22) |
| O14 | Phase 7 | `logic::plan` still runs its own stratum loop; it should emit wave steps for `waves::schedule` | closed (2026-10-30): `logic::plan::emit`; independent strata share waves; plans from the boundary's counts |
| O15 | Phase 8 | S's oracle (`native_oracle.rs`) needs S's `matcher/` deltas, which no copy row covers | copy placed (2026-10-29): S's oracle runs as copied on P's anchor engine (`findings::tests::s_native_oracle`, `matcher::p_anchors`); open only: which comparison survives (O19) |
| O16 | Phase 0 | 05 says port `mimalloc`; W already runs a tuned jemalloc | closed (2026-10-29): jemalloc stays the global allocator (user decision); `mimalloc` remains a dependency only |
| O22 | Phase 9 | P's text prefilter on three-valued clauses against W's consumers' pass | closed (2026-10-29): option (a), user decision |
| O23 | Phase 4 / 9 | Lowered matchers' node labels on Metal (06 S1, decision 0011) | open |
| O24 | Phase 4 / 10 | Semantic predicates as labels from fact producers (06 S1, decision 0011) | closed (2026-11-03, decision 0030): facts as labels, S1 158 of 286 lowered (from 77), all exact; the rest is O36 |
| O25 | Phase 4 / 9 | A wave's recursive label rounds on Metal: one evaluation under 100 launches (06 S4) | closed (2026-10-30): sweeps in node order, 77 launches |
| O26 | Phase 4 / 9 | The wave's label passes on Metal take seconds: host buffer sizing and the one-thread-per-file sweep (06 S4) | closed (2026-10-30): labels only where a clause has candidates; sweeps on the CPU between GPU levels; 18.8 → 6.4 s on 8 fixtures |
| O27 | Phase 4 / 10 | The findings layer's Unknown domains: syntax errors, capture anchors, `noqa` parents, report ceilings (06 S5, decision 0013) | closed (2026-10-30): all four decided in SQL; Unknown only where W's clause fails |
| O28 | Phase 4 / 9 | The wave's labels on Metal no faster than on the CPU: 64 interpreter lanes, unoptimized measurements, per-task host planning (06 S4) | open, narrowed (2026-10-31): S7 4.7 s wall / 10.8 s CPU on Metal against 19.7 s / 20.7 s (0015, 0017–0021); GPU labels 0.7–0.9 s; host: prefilter SQL 1.3 s (anchor index, `lab.rs` loop), CPU roots 0.75–0.9 s, sweeps 0.5 s, push 0.5 s; 2026-10-31: walks carry 8 cursors (was 4), so no label program falls back to the interpreter in the sweeps (498k calls), and sweep entries are 8 bytes (node, pass); labels.sweep 650 → 234 ms |
| O29 | Phase 4 / 9 | A recursive label through `not has not` settles one level deep: SIM300's `probably` misses `X == 1 + 2 + 3`, `X == [[1]]` (06 S7) | closed (2026-10-31): a sweep with complements sets or clears each roots verdict; S7 equal for every (rule, code) |
| O30 | Phase 4 / 10 | S7's sample (250 files, 1.5 MB) on the CPU: peak RSS 972 MB against a 500 MB budget; labels 24 s of a 32 s cold audit, single-threaded (06 S7) | closed (2026-10-31, decision 0022): S7's engine-only footprint peak is 477 MB on Metal (was 0.99 GB) and 447 MB on the CPU backend (was 575–607), measured as `phys_footprint_peak` after the audit. Freed heap pages are returned at stage ends; SQLite allocates from jemalloc; the GPU's label prefix, scratch and counts are sized by the run; roots stream into rows with digested structure keys. Left: the driver's fixed ~170 MB per GPU execution, and sparse host labels if the corpus grows |
| O31 | Phase 2–9 | Per-node structures beside the tape (decision 0026) | closed (2026-11-02): none is stored (S7's store: at most 0.12 rows/node, `derivation_inputs`); the in-memory ones are readers' working state, which 0026 allows (user ruling) |
| O32 | Phase 3 / 11 | The SQL fallback's relations cost 7.7× P's VM steps (correlated `EXISTS` per scope row) | closed (2026-11-02): uncorrelated set operations, 2.21 G → 0.091 G (0.32× P), same rows |
| O33 | Phase 4 | G's grid executor (`ExecutionBackend::Metal`, not on the production path) exceeds the 99-launch bound on five parity tests | closed 2026-11-02: superseded by 0024 (decision 0028); kept as a copy until step 31 |
| O35 | Phase 4 / 9 | The Metal label run's interpreter workspace against the 256 MB buffer budget: S7's real files (results arena) | closed (2026-11-03): groups single-dispatch (O34); S7's 250 files agree on all four backends |
| O36 | Phase 4 / 10 | Semantic leaves still refused as fact labels: shared bindings, binding predicates, multi-target sub-rules, `parse`, under-`not` unknowns (decision 0030) | open |
| O37 | Phase 4 / 8 | The lowered B006 clause reports 4 mutable defaults on S7's `f010.py` that W does not (`Optional[…]`, `Annotated[…]` annotations); `the_slice_finds_ws_findings_on_real_files` ignored | open |
| O38 | Phase 9 | The interpreter's per-lane transient workspace (lanes × states, sized before any task runs) exceeds the buffer budget for 5 of 39 interpreted S2 clauses and for 70 of 158 under `RERAIL_HOST_SWEEPS`. 2026-11-04: tasks are implicit in segments and lanes are fitted to the free budget (decision 0033); 37 of 39 equal | closed |
| O40 | Phase 9 | Two interpreted S2 clauses (ruff-async251, ruff-sim300) are recursive groups whose host rounds exceed the 200-launch limit (`BackendLaunchLimit`); their rounds belong on the device | open |
| O41 | Phase 10 | Bounded cross-file rounds (k = 2) leave calls.rdl short of its fixpoint on the vtab corpus (a third file crossing adds 8 of 5,112 hop rows); the unconverged frontier is reported but not yet marked pending | closed (2026-11-07): the typing programs no longer run in rounds; the cross-file part is one fixpoint (decision 0065 step 1) |
| O42 | Phase 10 | Terms, declarations and dependency summaries wired into the corpus runner (`corpus::typing`); left: terms inside the engine's lattice, variance | closed (2026-11-04) |
| O39 | Phase 9 | S7's sample, cold on Metal: the footprint peaks at 871 MB (budget 500 MB) in the label runs (`3a.plan` and the roots' partition), after `WalkFrames` removed the 3,017 MB of interpreted retries (decision 0032). 2026-11-04: the roots are rules over the codes (decision 0033), with no node lists (70 M roots before) and no walk items; the peak (889 MB) now rises in the GPU run and the roots' partition. Roots that are walks no longer size each file for a backend that never runs (one packed batch per file, ~130 MB over the GPU run's): 752 MB, in the GPU run. The kernels keep no thread memory (the driver's scratch, decision 0034): 603 MB, 335 MB after the audit | open |
| O43 | Phase 10 (step 30) | Launch cost on the device: each chunk pays one GPU wake-up from idle (about 0.85 ms, 12 of 19–22 ms of host wait on async-sync-boundary at 1 in 8, decision 0040) because the GPU sits idle while the CPU runs the chunk's joins. The joins themselves are small (about 0.1 s of 7.7 s CPU, at most 245K rows into a step over the run), so the goal is not join speed: it is to keep the chunk on the device in one launch. Close by running a chunk's native joins on Metal in the same command buffer as its nearest ancestors (one command buffer, one wait a chunk): plan chosen on the host from statistics as now; each step a sort or hash join kernel over the shared relations, counting then writing (or writing into a reused block and retrying on overflow); negation as an anti-join; text tests as chunked matchers; aggregates and cross-file rounds stay as they are. Loading inputs, building indexes and the per-chunk statistics (about 2.8 s of CPU) are the same shape and move with it. Validated by the profile's device phases: one launch per chunk, schedule (GPU waking) no longer paid per chunk, findings unchanged | open |
| O60 | Phase 6 / 11 | The matcher atom's captures, `ast($D, $N, {…}, NAME: $v)`: parsed, no executor (t17) | open |
| O61 | Phase 6 / 11 | A builtin catalogue: the data-layer programs use 30 builtins, one built (t17) | open |
| O62 | Phase 6 / 11 | One value lattice, the shortlex-least route: four data-layer recursions, no implementation, `widen` loses the witness (t17) | open |
| O63 | Phase 6 / 11 | Tree built-ins beyond `nearest` are written as builtins (`@within`, `@depth`); two kind-filtered walks stay Datalog recursion (t17) | open, narrowed (2026-11-06): `.rr`'s `within(n, a)` is the engine's tree relation, read off the tape per looked-up node (decision 0050 §4); the `@within`/`@depth` programs and the two kind-filtered walks remain |
| O64 | Phase 11 | The 113 regex forbid clauses of the data-layer rules have no logic form until rewritten as matcher atoms (PIAT §5.1, t17) | open |
| O65 | Phase 11 | Two data-layer stages outside the fragment: `call_paths` (a value cycle) and `block_results` (`NotStratifiable` over the tree) (t17) | open |

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

S3 update (2026-10-30): the ingest side is wired. `Families::push` observes a root (a directory
snapshot or a Git `HEAD`, `facts::observe::roots`), encodes and stores each new or changed tape
once, builds core facts from it, and publishes its prefilter rows, each a derivation that cuts off
on its digest. Named `git add` works in tests, so a committed repository is tested. What remains:
a production caller (W's workspace scan, or the slice's entry points, S6), and the corpus capture
for the anchor lab.

2026-11-01 update: the production caller is the slice's napi method, `CodeRuleSlice`
(`slice/napi.rs`). `audit`/`apply` take an observer: `"directory"`, the working tree's snapshot, or
`"head"`, the checkout's `HEAD` tree through W's `git_workspace::discover`. Both push through the
overlay with its writer, after recovering any push that did not end (O1). Left: editor buffers as
an observer (an overlay snapshot of unsaved sources), and the corpus capture for the anchor lab.
Buffers since: `roots::buffered_root` lays each buffer's bytes over the directory snapshot (a
file on disk or not yet, in a directory on disk or not yet), and `audit`/`apply` take them with the
directory observer. A file watcher is the host's: it calls `apply` on a change. The corpus capture
is Phase 3's check.

## O3. Synchronous interning

A new name is interned into `atoms` with one SQLite transaction per batch, under the write lock.
Warm edits rebind existing names; a cold scan pays one write per batch. Closure: measure on a cold
scan of the gate corpus; if it is visible in CPU time, intern ahead in the overlay with a reserved
range and write the atoms with the first layer that uses them.

**Measured (2026-11-06, notes T10 §1b):** it is visible. On the 9,010-file corpus, 104 chunks
intern 640k new names, one transaction each. The batch worker spent 77% of its samples there
(about 6–7 s CPU), mostly in SQLite's page cache and WAL. SQLite's own counters per run:

| Setting | Misses | Spills | Pages written |
|---|---|---|---|
| 1 MB cache | 158,806 | 168,121 | 261,856 |
| 32 MB for the transaction | 107,432 | 0 | 202,009 |

**Done:**
- A bulk append (more than 1,024 new names) gets a 32 MB cache and a 16k-page checkpoint
  interval for its transaction only. Both return to the store's settings after it.
- Per run, interning went from 77% to 44% of the batch phase, and sys from about 6 s to 3.5 s.

**Still open:** about 200k pages, roughly 800 MB, are written per run. Each commit rewrites
about 1,900 leaf pages of `atoms_by_value`, so the volume scales with the number of commits, not
the number of names. The fix is fewer commits: allocate ids in memory and write the rows later.
That needs:

- one in-process mirror per database, shared by the store and the planner's symbols
  (`logic::native::setfix::Interner`). Today each catches up from the table.
- every store write transaction appending the pending atoms first, so no durable row refers to an
  unwritten atom.
- an in-process reader of `atoms` by value (vocabulary SQL) flushing first.

**Closed (2026-11-06):** decision 0049. On the full corpus: 3 appends of 29.6 MB instead of 104 commits and about 800 MB. Atoms are stored as segments of the arena with no value index, SQL reads them through a virtual table, a lease covers other processes, and the decision sets out recovery after a crash.

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

## O7. `tid` and text hashes

Closed by decision 0026 (2026-11-02, user decision): nodes live in tapes. There is no `tid` column
or array and no text hashes. A text predicate's nodes are the union of its terms' postings plus the
uncovered nodes, confirmed on their text; parents, child positions and lines are read as 0026's
table says. The per-node arrays `core` held were removed, and so was the `tid` vector step 10
added.

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
  string tables, the digest and length header, and Blade's omissions are kept off the tape: in
  `file_documents`, or recomputed (decision 0026 rules out structures keyed by the digest). Derived-document maps and host errors already live in
  `file_documents`.
- Blade stores nothing more or less than the tree-sitter-php tape it replaced. Today its
  compiled-view tape drops nodes over synthetic placeholder segments, so a directive call has no
  `function` child. Proposed fix: the plain tree-sitter-php tape, with omission through the segment
  map in `file_documents`. Error spans stay in the tape as tree-sitter emits them, as ast-index-spike does. It
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

**Not yet decided:** where W's supertype memberships go in v9np (resolved by 0025's amendment: in the codes); whether the parent column is
kept (resolved 2026-11-02, 0025 amended: no; parents are rebuilt at decode).

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

**Closed (2026-11-03): option (b), decided by the user.** `logic::front::cycles::check` runs in `parse`:
- **A value cycle is in the flow of values between columns**, not of relations: a value a builtin made
  (`@cat`, `@flat_bin`, in a head or by `$v = @f(…)`) reaching that builtin's arguments again. A
  component whose made values never come back makes finitely many: T's `qv(@cat($q, $x), …)` sits in
  `calls.rdl`'s cross-file component `{base, bv, found, need_attr, pv, qv, sub}`, but `$q` and `$x`
  come from columns no made value reaches. Values made into a lattice's value column are bounded by
  the lattice (`UnboundedLattice` checks its height or `widen`), as `cv`'s `@flat_bin`.
- **A value cycle's component is bounded by the tree** when one node column per relation (its key)
  moves one way along tree edges in every rule: from the head's key to each recursive atom's key,
  strictly for a rule that makes a value, and not backwards for the others. Tree edges are `child`,
  `field`, `tape`, `nearest`, and what a lower stratum's relation guarantees between its node columns
  in every rule (`attr_of(n, o, a)`: `o` below `n`).
- **T's five programs pass**, with exactly `qn` (calls), `modn` (calls_rust) and `resolves` (the three
  workloads) bounded by the tree; a cycle along a graph relation, or down one way and up another, is
  `Error::UnboundedValueCycle` (`named_errors`). A data-layer or cross-file rule that makes names
  around an import or call graph is refused; (c), a depth bound, is the fallback if one needs it.

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

**Closed (2026-11-03): option (a), decided by the user** ("implement the part of step 29 you need to
make it a transducer program"). `typing::file_local::nearest` is the first file-local stratum:
- **One flat-lattice system per chunk**, a cell per (set, node): a node whose parent's kind is in the
  set is seeded with its parent, any other copies its parent's cell. The transducer's file-local FIX
  solves it (list step 17, `metal_lattice` on the device work queue: one launch for all of a
  program's sets); the CPU solves the same system as the reference.
- **The strata read it as an input**: `Lowered::transduce_nearest` rewrites each `nearest($n, $a, set)`
  into `nearest_<set>($n, $a)`, and `facts::vtab::syntax::with_nearest` binds those relations beside
  the base relations of the same batch (looked up by `n` and by `a`), with their statistics made at
  bind time (`syntax::statistics`).
- **T's programs derive T's outputs** that way on both bindings
  (`t_programs_with_the_transducers_nearest_derive_t_outputs`); the device's values equal the CPU's
  and a parent climb's (`typing::file_local::tests`).
- Kept, not the target: T's SQL lowering (the recursive CTE over `tape`) and its native climb, which
  `t_programs_over_atoms_plan_and_run_like_t` still uses to reproduce T's plans line for line. Open
  for list step 29: the wave path binding `nearest_<set>` per chunk, and the other file-local strata.

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

**Closed (2026-11-03, list step 22).** `logic::plan::run_atoms_native` runs T's native modes over atoms, on
`logic::native` registered on the store's connection, with `nearest` from the transducer (O12). On workload,
workload_demand and calls, every mode (`fixpoint`, `fixpoint,vtab`, `fixpoint,direct`, `upsert`, `setfix`,
`setfix,join`, `all`, `program`) gives T's outputs (`t_programs_run_natively_over_atoms_like_t`); `program` runs 23,
24 and 72 components natively. What over atoms needed:
- **Lattice keys are compacted to dense cells.** `lattice_fixpoint` sized its arrays by the largest key; T's node
  ids were dense, the store's are `d << 32 | index`. The result vtab looks keys up by binary search.
- **A transfer's operator is its name atom**; the bin table is mapped to `+`, `-`, `*` before the native solve.
- **Every native column is an integer over atoms** (`sym_char`): a `sym` column holds name, kind or field atoms,
  and only name atoms have text, so the native store's live statistics and their constants stay integers.
- **The join engine's loaded inputs are per run** (`setfix::forget_inputs`, keyed by database): T cached them by
  SQL text for its process; the base relations here are rebound per batch, and `calls` read `workload`'s rows.
- Dropped over atoms: T's `nearest` and `kseed` modes, which read a per-run copy of the tree (`tape_load`).
Open: the planner's statistics from `stats::ingest` instead of T's tables (list step 29, with the typing
programs' own ingestion), and these strata on the wave path (O14's `emit` covers the SQL strata only).

## O14. The planner's stratum loop and the frontier loop

`waves::schedule` (decision 0010) runs wave programs: steps that insert into `new_<relation>`,
read `delta_` relations of their own stratum, and are selected by SQL wave by wave. `logic::plan`
(`run`) still runs its own loop: semi-naive rounds per strongly connected component, with
`CREATE TEMP TABLE … AS` per round, and `count(*)`/`count(DISTINCT)` refreshes between strata (DLRE
trap T18). Lattice relations (`merge_lattice`) have no step form yet. Closure: the planner emits
a `schedule::Program` (one step per rule and delta variant, its stratum from `strata`), with
lattice merges as a fold kind, and T's programs give the same outputs through `schedule::run` as
through `run_atoms`; the planner's counts then come from `rows.<relation>` at the boundary.

**Closed (2026-10-30).** `logic::plan::emit` makes one stratum per component (constant rows
before them all), one step per rule over whole relations (a recursive component's reading only
lower strata), and per own positive atom one delta variant planned with its delta first.
`schedule` folds lattice relations by key as `merge_lattice` does (`Program::lattices`).
- **Waves follow dependencies, not strata.** A wave's frontier is every stratum with no pending
  stratum upstream of it (`temp.wave_after`, the transitive closure of the strata its steps read
  from), so independent strata share waves. workload takes 15 waves (from 50), workload_demand 18
  (from 44), calls 64 (from 117). calls' 64 are its dependency depth: a chain of about 23
  non-recursive strata and fixpoints of up to 14 waves (`pv`, `vopen`, `open_sync`).
- **Counts at the boundary.** Steps are rows the loop selects, so plans are made before a run. A
  run's boundary counts (`schedule::Outcome::rows`, `rows.<relation>`) are the next emission's
  derived cardinalities; without them, the estimates. Planned from the first run's counts, 10, 27
  and 58 steps change plan.
- `t_programs_as_wave_programs_give_run_atoms_outputs`: both emissions of workload,
  workload_demand and calls equal `run_atoms`.
- **The wave's step.** `waves::matches` gives its program as RDL over two batch inputs (the
  chunk's demand relation and `temp.wave_document`, `Inputs::batch`), planned from row
  estimates in place of statistics, and `emit_batch` emits it; the S4 gates are unchanged.

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

**Closed (2026-11-02, user decisions).** 186 of 186 pass.
- Supertypes: N's semantics. `kind: <supertype>` compiles to `AstMatcher::Supertype` (`matcher::Supertype`: the class and its transitive concrete kinds, `GrammarDialect::supertype_kinds`). It matches the nodes the parser reported as members, read from the tape (`TapeDoc::transducer_has_supertype`; a tree that is no tape's reads unknown). It lowers to `SUPERTYPE` in the packed runtime (`MatchTree::Supertype`), with the concrete kinds as its domain and anchor, and to N's class-membership terminal in the relational lowering. `slice::tests::a_supertype_rule_finds_the_same_members_on_every_backend` checks the CPU interpreter, JIT and both Metal bindings. The rule reference documents it.
- Syntax errors: W's 50% unread rule stays. `clause-syntax-error-unknown` is re-blessed in `production_contracts_081_108.json` to W's `source-incomplete` ("error regions cover 91% of it").
- The relational lowering stays test-only. Whether it replaces `demand::vtab`'s lowering is left to Phase 6.

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

**Closed (2026-11-03)** by decision 0030 (list step 26). A semantic leaf is a fact label: W's
predicate, with its single-target sub-rules as slots, makes (node, targets) rows before any pass,
and the slots are labels of their own. S1 lowers 158 of 286 clauses, all exact on the 60 fixtures;
S2 finds the same roots on Metal for all 158. What is still refused is O36.

## O25. A wave's recursive label rounds on Metal

**Closed (2026-10-30)** by candidate 2, recursion in node order, on the GPU (`label_sweep`,
06 S4). The wave is 1 command buffer with 77 launches. The measurements below are from before.

06 S4's Metal check: the wave (87 lowered clauses over a chunk) as one evaluation under 100
launches (`metal_launch_budget.rs`). Not met; `the_wave_is_one_evaluation_on_metal` is ignored with
this item as its reason.

What the wave needs, measured on 15 fixtures:

- **Rule depth.** The clauses' labels are scheduled by global dependency level, merged across
  clauses (`compile_matcher::metal`). The non-recursive passes take 45 launches: about 15 levels at
  3 kernels each (interpreter, sink, settle).
- **Recursion.** 8 merged recursive groups cost 72 launches per round, because a round re-runs
  every level of its group. One group has 10 levels.
- **Rounds.** Measured on the CPU, 63 recursive groups settle in 2–3 rounds, counting the round
  that confirms nothing changed. The worst take 5–8, and `ruff-f704` takes 18 (`inside` …
  `matches: yield-ok`, the nesting depth).
- **Bounds.** Rounds are encoded up front, to a bound the host computes: 25 (the tree's depth) or 154
  (the longest sibling run, where a group recurses along siblings). A skipped round still counts as
  a launch, so the wave exceeds the 600 limit.

Why this is not a tuning problem: a recursive label is evaluated as a Jacobi iteration. Every node
is recomputed in parallel each round, and the recursion moves one tree edge per round. Launches are
then rounds × group levels × 3, while the data dependency is rule depth plus recursion depth.

Candidates, in rough order of reach:

1. **Rounds the GPU decides.** Encode a group's rounds once, for example as a Metal indirect command
   buffer whose executed range the settle kernel writes, so the cost is the real depth, not the
   bound.
2. **Recursion in node order.** For a group that recurses only toward smaller preorders (ancestors,
   preceding siblings) or only toward larger ones (descendants, following siblings), one pass in
   that order settles it. On the CPU that is one pass. On the GPU it needs the interpreter to
   evaluate in that order.
3. **Fewer kernels per level.** Fuse the sink into the settle where a label starts empty and only
   grows: 2 launches per level instead of 3.

The CPU wave is unaffected: its labels settle per group by fixpoint, and the root programs run as
one `demand(batch)` per chunk.

## O26. The wave's label passes on Metal

**Closed (2026-10-30).** Measured on the gate (8 fixtures, load about 15) before the change: the
label passes took 17.6 s, of which the host's `size()` was 6.7 s (workspace assignment 5.2 s for
1.35 M tasks) and the GPU 10.9 s. Every dispatch had 64 lanes, and the sweeps (8.8 s) ran one
thread per file, the largest file setting the time. The fixes:
- **Labels only where they are read.** A clause's labels are its own and read only by its root
  program, at its candidates. `LoweredWave::labelled` and `labelled_metal` take `active` (per
  clause, per tape: whether it has candidates) and compute a clause's labels only there, on both
  backends: 1.35 M tasks become 686 K, and the CPU wave on 8 fixtures goes from 12.1 s to 7.0 s.
- **Starts that cannot match are not started.** A label domain on Metal starts at the kinds its
  label can have (`Labels::bound`), and a program whose first tests are a kind or a grammar
  supertype starts only where they hold (`Entry`).
- **Workspace assignment** computes a program's part of a task's capacities once per program, and
  `plan_shared` tests its kinds by a precomputed mask: `size()` 6.7 s → 1.4 s.
- **Sweeps where they are fast.** A sweeping group follows each file's nodes in order: on the GPU
  one interpreter thread a file, which is far slower than a CPU core (group 9: 4.2 s on the GPU,
  1.6 s on the CPU). The wave's Metal labels now run the wide levels on the GPU and each sweeping
  group with the CPU's `sweep`, the labels passed between them as words (`Plan::initial`).

Result: the Metal wave on 8 fixtures 18.8 s → 6.4 s (CPU 6.6 s); on 48 fixtures 8.8 s (CPU
9.8 s); 0 differ on both. Measured on the way: groups with complements take three sweeps on the
CPU as on the GPU, so the claim that one reverse sweep settles them (decision 0011) does not hold
for these clauses; both iterate until a sweep changes nothing.

06 S4: on Metal, `the_wave_finds_ws_matches_on_metal` (8 fixtures) takes 17.6 s, of which the roots
are 178 ms. Measured on 4 fixtures: the host's buffer sizing (`size()`) takes 3.5 s and the GPU's
flush about 7.3 s. This predates moving the roots to Metal. Candidates: size buffers from the batch's
counts instead of walking it, and split a recursive group's sweep over more than one thread per
file (subtrees whose verdicts do not read each other). Closure: the wave on Metal no slower than on
the CPU for the same fixtures.

## O27. The findings layer's Unknown domains

**Closed (2026-10-30).** The first version of 06 S5's layer published Coverage `Unknown` for four
cases it did not decide; on 48 fixtures that was 61 of 2928 domains, all syntax errors. Each is now
SQL over facts (decision 0013):
- **Syntax errors.** `error(d, s, e)` holds the file's regions; a witness of a clause reading around
  them is dropped where its span meets one, by W's three overlap cases. A file W's parser did not
  read, and a clause not sound under errors, fail in W, and stay `Unknown` (`source-incomplete`).
  On 48 fixtures: 30 such domains, each one W fails.
- **Capture anchors.** The wave's one capture-anchored clause (W605's format spec) anchors at a
  regex group with `each`. `group(c, d, n, s)` holds each binding's start, one violation each. A
  metavariable anchor reads its binding from W's matcher at the root the wave matched, since the
  lowered root programs keep only labels. The probes cover a single capture, a multi-line
  multi-capture and an unbound one.
- **`noqa` parent statements.** `stmt(d, s, e)` holds the file's import and expression statements;
  the innermost holding the anchored byte is the one no deeper one holds.
- **Report ceilings.** `total(r, x)` counts each rule's violations over the scope, and
  `ceiling(r, m)` gates the rule's findings.
- **Kept approximations:**
  - W's `noqa` mapping takes the first range holding an offset, and the layer takes every one
    (equal where ranges do not overlap);
  - the module comes from the scope's documents, not a host's `module`.

## O28. The wave's labels on Metal are no faster than on the CPU

**Found (2026-10-30), by profiling** (timers per segment, `sample` stacks, then an optimized
build).
- **The measurements were unoptimized.** This crate builds at opt-level 0 in tests; only its
  dependencies are optimized. Every host number so far (sizing, the CPU sweeps, the CPU
  baseline itself) was therefore of unoptimized code. Only the GPU's own time was real.
  - `cargo test --profile profiling` builds this crate at opt-level 3, incrementally and without
    LTO.
  - Optimized, on 8 fixtures at load about 80: the CPU wave's labels take 1.88 s and Metal's
    3.42 s.
- **The interpreter ran 64 lanes.** A lane is a persistent thread that takes tasks from the
  level's cursor, and the program backend's `EXECUTION_LANES` is 64. So a level of 686 K tasks ran
  on two SIMD groups, about 10 K tasks a thread: 1.23 s of GPU time over 9 segments.
  - Lanes were bound by memory, not by the GPU. A lane reserves its transient states up front.
    Each task's state limit is its work estimate, `nodes × words × 4096`, capped at 65,536, so
    nearly every task got the cap.
  - So 64 lanes already held 4 M records per kind, and 4,096 lanes would need 268 M.
- **First use compiles the pipelines.** Building the interpreter and label pipelines from source
  takes 0.57 s, once per process, inside the first label segment.

**Fixed:**
- **Narrow lanes.** Label levels run 1,024 lanes of at most 4,096 states each (`NARROW` in
  `metal_label_passes`), the same records as before over sixteen times the lanes.
  - A task that needs more fails its batch, which runs again with the old sizing (`WIDE`,
    `Outcome::widened`). Sweeping groups keep `WIDE`.
  - 2,048 states fail on the fixtures and 4,096 hold; no gate widened.
  - The GPU's time on 8 fixtures went from 1,232 ms to 233 ms.
- **The CPU sweeps between GPU segments run each file on its own thread** (rayon): a file's sweep
  reads and marks only its own labels. 0.63 s became 0.23 s.
- **The pipelines compile on a thread when a Metal wave is made** (`label_passes::warm`), so they
  overlap the push and the prefilter.

**Result** (optimized, 8 fixtures, load about 40), with 0 differ on every gate:

| | Before | After |
|---|---|---|
| Whole wave, CPU (load about 80 before) | 2.58 s | 1.38–1.54 s |
| Whole wave, Metal (load about 80 before) | 4.37 s | 0.92–0.98 s |
| Labels, Metal, without the compile | | about 0.96 s |
| Labels, CPU | | 1.10 s |
| Labels, Metal, one chunk of one document | | 57 ms |

The S2 parity gate (87 clauses one at a time, 60 fixtures) runs without widening any batch.
S6's bench audits 48 fixtures cold in 1.79 s of CPU time on Metal, against 2.27 s on the CPU.

**Left**, which is why this stays open. Metal is now faster than the CPU, but not by much.
- **Host planning per task.** `size()` builds a `PackedBatch` and assigns workspace for each of
  686 K tasks, 287 ms. The tasks are (program, node) pairs a kernel could enumerate from each
  level's domains.
- **The GPU's interpreter.** One thread runs one task, and threads diverge. 226 ms remain.
- **The sweeps' CPU walk.** 227 ms, one file's group setting the time.
- **Conversions between label words and sets** around each CPU sweep, about 0.2 s.
- **The CPU path's labels are single-threaded.** A parallel CPU path would move the comparison.

**Fixed (2026-10-31), decision 0015.** Stage timers (`prof`, with `RERAIL_PROF`) on S6's bench
(48 fixtures) gave these causes:
- 2.51 M interpreter tasks cold, about 160 per node, of which 35% produced a result;
- a 930 MB workspace cleared on the host;
- a one-workspace pool, replaced whenever the levels' sizes varied.

Fixes:
- **Micro walks.** Label programs that are only walks (380 of 560) run one thread per
  (pass, start) with a fixed stack (`label_micro`), and on the CPU sweeps as well.
- **Untouched workspace buffers**, so pages are committed only where the GPU touches them.
- **A pool that keeps the widest workspace.**

Cold audit, CPU time: Metal 1.23 s → 0.92 s, against 1.79 s on the CPU backend. Labels, cold:
1.09 s → 0.43 s. Peak RSS: 2.1 GB → 284 MB. The one-file edit takes 0.22 s against the CPU's
0.40 s. Every gate shows 0 differing.

**Still left (superseded by the next part):**
- **Text-test programs** (12% of roots) are interpreter tasks.
- **`size()` and the flush** take about 270 ms cold.
- **The CPU sweeps between segments** take about 85 ms warm.

**Fixed (2026-10-31), decision 0017, at S7's size.** On 75 of S7's files, labels took 18.2 s. The
cause was three whole-run reruns: a walk past 2^16 steps, and two WIDE fallbacks. The remaining
interpreter tasks (19.3 M of 21.3 M) were regex text tests.

Fixes:
- walk steps scale with the file;
- text tests are bits computed once per (value, kind), on the host, in parallel;
- a Metal wave's roots run on the CPU's JIT.

Labels on those 75 files: 18.2 s → 2.2 s. S7's 250 files cold: 12.1 s wall and 19.6 s CPU time on
Metal, against 27.0 s on the CPU backend.

**Left:**
- `execution_partitions` builds batches to test fit (2.4 s);
- the CPU backend's labels are single-threaded (21 s of its 27 s), which makes the comparison
  flatter than the shapes are.


## O29. A recursive label through `not has not` settles one level deep

**Found (2026-10-31), by S7's gate.** SIM300's (`ruff-sim300-yoda-conditions`) utility `probably`
is recursive through a negation: a `binary_operator` is `probably` when it does not have a child
that is neither an operator nor `probably`. The same holds for lists, tuples and dictionaries. One
level agrees with W (`X == 4 + 1`, `X == [1, 2]`). Two levels do not: for `X == 1 + 2 + 3` and
`X == [[1]]` the lowered clause finds a yoda condition and W does not.
- **The cause is the plan, not a backend.** The CPU sweep, with micro walks or without, and Metal's
  label passes give the same three keys on the stdlib's `_pydatetime.py`.
- **S1's fixtures never nest these constructs** in a comparison, so S1 and S2 pass.
- **Where to look:** how `levels` orders a recursive group's programs and complements (decision
  0011, `sweeps`). Marks only add, so a complement computed before its program's marks at a
  descendant is never retracted.
- **Pinned by** `slice::tests::a_recursive_label_settles_at_every_depth` (ignored) and listed in
  S7's `KNOWN`.

**Closed (2026-10-31).** `program.roots` (S1's path, rounds) agreed with W on the repro, and so
did the wave with its sweep replaced by rounds. So the sweep was at fault.
- **The cause:** a sweep's program marks only added. A program at a node could read a child's
  complement before that complement's source was settled there. The complement was true while
  unset, and the mark stuck after it flipped.
- **Metal had the same answer** because its sweeping groups run the same CPU `sweep` between GPU
  levels.
- **The fix follows the sweep's own rule:** with complements, a group's programs mark only their
  roots. So a roots verdict at a node is set or cleared there, as a complement's is, including
  when the node leaves the program's label domain. The sweeps then iterate to the rounds' fixpoint.
- **Checked by:**
  - the regression test, no longer ignored;
  - S7: 710 keys against W's 710 on both backends, 0 (rule, code)s differ;
  - S1/S2, S4–S6 and the vtab tests.
- `KNOWN` is empty.

## O30. S7's sample exceeds the memory budget, and the CPU's labels are single-threaded

**Found (2026-10-31), by S7's gate** (250 files, 1.5 MB, load 60–75):
- **Peak RSS is 972 MB** on the CPU backend, against the 500 MB budget. Metal peaks at 3.9 GB.
- **A cold audit takes 31.7 s of CPU time**, of which labels take 24.0 s. CPU labels run on one
  thread (O28). Candidates take 1.8 s and the push 0.7 s. W takes 14.6 s of CPU time (2.1 s wall,
  in parallel) for the same 710 keys.

S8 measures these with interleaved arms. Where the memory goes is the first question.

## O31. Per-node structures beside the tape

Closed (2026-11-02). The user's ruling: only per-node structures stored in the database must go
(ast-index-spike: "nothing may *store* one row per node"). None is: S7's store, 322,472 nodes,
peaks at `derivation_inputs` with 0.12 rows/node. `no_relation_stores_a_row_per_node` and S7 now
assert at most 0.25 rows/node per table. The in-memory structures the linter first listed
(`TapeDoc`'s records, `Facts`, the facts vtab's projection, `MetalNode`, Forte's and literal
sub-grammars' trees) are readers' working state and stay.

## O32. The SQL fallback's relations

List step 12's lab: the fallback (`lower::fallback`) runs 2.21 G VM steps on the 63 batches where
P runs 0.285 G and the production lowering 0.052 G. Each relation is a correlated `EXISTS` per
scope row over an unindexed materialized set (`related_on_tape`), the shape the spike measured
re-running its body per row (S203: 296.7 → 31.2 ms once materialized). **Closure:** relations as
joins over each scope's neighbourhood from `nera_tree`, materialized once and indexed on
`(fid, pre)`, measured in the lab against P before list step 20 gives the fallback a driver.

**Closed (2026-11-02).** Each relation is an uncorrelated set operation read from the side where
the neighbour is one `nera_tree` step away: has from inner's parents or ancestors `INTERSECT` the
scope; inside, precedes and follows from the scope's parent, ancestors or next sibling tested with
`(fid,pre) IN (SELECT fid,pre FROM inner)`, which SQLite indexes once; recursive sibling relations
join scope and inner on their shared parent. On the 63 batches: 2.21 G → 0.091 G VM steps (P
0.285 G), execute CPU 57 s → 2.3 s, rows identical (1,312,845);
`the_sql_fallback_relates_on_the_tape_exactly` still holds the eleven relation rules to the matcher.

## O33. G's grid executor and the launch bound

List step 13 bounds an evaluation at 99 launches. G's grid executor (`metal_grid_backend`,
`ExecutionBackend::Metal`) plans its stages with 16 checked frontier waves each
(`CHECKED_FRONTIER_WAVES`), and on five of the packed runtime's parity tests it is refused with
`BackendLaunchLimit`: `all_backends_execute_every_verified_instruction_and_mode`,
`cpu_jit_compiles_general_scm_control_flow`, `scm_host_predicates_filter_text_kinds_lines_and_relations`,
`scm_output_directives_transform_text_adjacency_and_language`, and
`scm_quantified_runs_preserve_each_distinct_middle_capture`. These tests still assert the CPU
interpreter's and JIT's parity. They skip the grid executor's comparison only where it refuses
(`tests::grid`) and print `O33`. The production wave does not use this executor: its Metal work is the
label passes (44 launches on S7) and the set steps (1).

**Closure:** the grid executor's frontier waves planned from the batch (its plan counts its
launches, as `label_passes` does), or run on list step 17's device work queue (decision 0027). Otherwise it is removed
with the copies it belongs to (list step 31).

**Closed (2026-11-02, user decision; decision 0028).** Superseded by 0024: the grid executor is neither ported to the work queue nor put on the audit path. The pack census finds that 71 of about 15,600 lowered programs need a general executor. Four of its ideas are kept, with file pointers, for a small work-queue executor if that share grows: digit-ordered terminals, exact duplicate removal of active states, bulk closure scans and exact page boundaries. The code stays as a copy until list step 31.

## O34. Device rounds for groups whose members are other pipelines

**Closed (2026-11-03).** The groups that did not fuse had members that were interpreter tasks only
because of cursor slots:
- The lowering gave every walk a new cursor slot and never reused one, so a program with many
  sibling walks needed more than a micro walk's 8 (`micro::CURSORS`). It became an interpreter
  task, and its group could not fuse.
- Slots are now a stack by walk nesting (`Builder::walk`): a slot is free once its walk restores
  it. A program holds as many as its deepest nesting.
- **S2:** 104 of 104 groups fuse (was 103), no task is refused for cursors (was 28,564), and the
  most launches for one clause is 34 (was 51).
- **S7's 250 real files:** the four backends find the same 1,259 keys.
  - The Metal backends took 9.1–9.4 s of CPU; the CPU backends 11.2–12.3 s (load 47).
  - Before, 3.4 M tasks were refused for cursors and 25 groups did not fuse.

The text below is the item as it was.


Decision 0027's device work queue runs only the code compiled into its kernel; file-local lattice
FIX is its first user. A recursive label group whose members are interpreter tasks or per-clause
pattern kernels (other pipelines) keeps host-planned rounds with device early exit and a rerun.
On S7 there are none: 8 groups fused (`Runner::group`), 0 unfused. G's grid executor's frontier
waves stay as O33 describes them. Widened or non-flat lattices run on the CPU.

**Closure:** such groups' members lowered to micro walks (then they fuse), or run inside a queue
kernel; or measured as rare enough on the full corpus that host rounds are the shape. G's executor
goes with O33.

**Measured (2026-11-03, list step 26): no longer rare.**
- On S7's sample, one file's first attempt plans these rounds:

  ```
  rounds [0, 64, 0, 20, 0, 64, 0, 20, 0, 20, 0, 20, 0, 64, 0, 7, 0, 64, …]
  ```

  That is about 340 rounds, the 64s at the round cap.
- The plan counts every group as fused, one dispatch for all its rounds, and plans 57 launches.
- These groups' walks use cursors or go deeper than 32 frames, so they do not fuse. Each round
  then costs launches per level, and the run exceeds any launch bound. The interpreter's
  workspace is also what exceeds the buffer budget (O35).
- The clauses that fact labels unlocked (decision 0030) are where these groups come from.

**Next design step:** micro walks with cursors and deeper frames, or such groups inside the device
work queue (0027), so each group is one dispatch. A retry ladder is not the fix.

## O35. The Metal label run's workspace and the buffer budget

**Closed (2026-11-03)** by O34: groups that fuse need no interpreter workspace, and `the_backends_find_the_same_keys_on_real_files` runs again. The notes below are from before.

**Requirement.** A wave's label run on Metal fits Metal's 256 MB buffer budget on real files.

**Status (2026-11-03).**
- **Done:**
  - A lease over the budget runs again with half the states per lane, down to 512 (step 26).
  - A roots-mode label pass keeps one result per task (`packed_batch::ONE_RESULT`), since its sink
    reads only whether the root matched. Its results arena is sized by that.
  - Pooled workspaces are freed before a capacity retry; they counted against the budget across
    runs.
- **Still open, and mostly O34:**
  - On S7's real files, recursive groups that do not fuse run on the interpreter. Their workspaces
    and per-level task buffers (up to 510 MB for one level) exceed the budget, and their rounds
    exceed the launch bound.
  - `the_backends_find_the_same_keys_on_real_files` is ignored with this item.
  - `the_wave_finds_ws_matches_on_metal` fails about one run in two inside the `waves::` group
    (parallel Metal tests share the budget's ledger). It passes alone.

**Closure:** O34. Groups that fuse need no interpreter workspace.

## O36. Semantic leaves still refused as fact labels

S1's refusals after decision 0030, by reason:

| Reason | Clauses |
|---|---|
| shared binding | 32 |
| arguments binds | 22 |
| parse | 17 |
| resolves.binding | 14 |
| constraints | 13 |
| unknown fact under not | 12 |
| compare | 6 |
| formatArgs | 6 |
| utility call | 3 |
| binds | 2 |
| binds occurrence or redefines | 1 |

The fact-shaped ones are:
- predicates that bind (`arguments`, `formatArgs`, `binds`);
- sub-rules tested on many targets (occurrence, `redefines`, formatArgs' unused or missing);
- `parse`, `host`, `embedded`, `calls`;
- facts that may be unknown under `not` in three-valued clauses.

Shared bindings are joins on capture columns between labels (O24's direction).

## O37. B006 reports annotated defaults W does not

Found at list step 27 (2026-11-03). `the_slice_finds_ws_findings_on_real_files` gives 1,259 keys
against W's 1,255 on S7's 250 files. Only `ruff-b006-mutable-argument-default` differs: 4 keys only
ours, none only W's, all on `f010.py` lines 267–272. Those are parameters annotated with
`Sequence[int] | None`, `Optional[abc.Mapping[…]]` and `Annotated[Union[…], …]`, with defaults `[]`,
`{}` and `set()`.
- **Where it comes from:** the matcher, not the findings layer. The four backends already found the
  same 1,259 keys before step 27 (O34), and the findings layer's statement only changed form there.
- **Likely cause:** B006's annotation test is a fact label (decision 0030), and W's verdict on
  these annotations' `resolves` differs from the lowered one.
- **Test:** ignored with this item as the reason, until the lowered clause agrees.

## O38. The interpreter's per-lane workspace is sized before any task runs

Found at list step 28, stage 5 (2026-11-04). The label passes' interpreter (the fallback for walks
that outgrow their frames, and every program under `RERAIL_HOST_SWEEPS`) leases its workspace
before it runs.
- **Fixed on the way:** durable outputs. A label program's results and final captures were
  records reserved per task at their worst case and summed (4.2 M results and 8.4 M captures on
  eight files, over the 256 MB budget). Label programs now mark their bits as they publish
  (`packed_batch::MARKS`), so they reserve neither. The interpreted S2 went from 15 to 34 of 39
  clauses, all equal to the CPU's.
- **Still wrong:** the transient arenas (pending, captures, cursors, candidates) are lanes × states
  per lane, fixed before the run (1,024 × 4,096 narrow; the sweeps' wide lanes unbounded, ~200 MB of
  pending alone). The host halves the states on a refusal, which is a ladder, not a size.
- **Shape to take:** size lanes from the workspace share and a task's measured need, not a fixed
  lane count with states guessed and halved.
- **Tests:** `lowered_clauses_find_the_same_roots_on_the_resident_interpreter` counts the 5 refused
  clauses; the packed batch refuses the same ones (`RERAIL_PACKED_INTERPRETER`).
- **Closed (2026-11-04, decision 0033 item 5):** an interpreted level builds no task records. Its
  evaluations name their root segments, the device finds each task's root from them, and the lane
  pool is sized from three quarters of the free buffer budget. 37 of 39 equal; the other two are
  O40.

## O39. The cold audit's footprint peak and the budget split

Found at list step 28 (2026-11-04). S7's sample, cold on Metal, measured with
`RERAIL_PROF_STAGES=1` (every stage's footprint, live heap and Metal buffers) and `footprint` on
the process (`RERAIL_DIAG_PAUSE_AT`).
- **Fixed on the way** (peak 3,017 → 603 MB):
  - only a walk's overflow retries (decision 0032);
  - roots as rules (decision 0033);
  - roots that are walks are not sized for a backend that never runs;
  - no thread memory on the device (decision 0034).
- **What is left at the peak** (the second label run, 603 MB):
  - the process at start: 63 MB;
  - the driver's fixed cost of a process's first submission: ~113 MB (measured alone, whatever
    the work);
  - driver-private memory beyond that: ~90 MB;
  - Metal buffers: up to ~240 MB during encode (the residency's slabs and the run's workspace);
  - the heap: 230 MB live. That is the tapes (~62 MB at `1.tapes`), the candidate sets' leaves
    (~36 MB) and the plan and encode state (~45 MB in the run).
- **The arithmetic does not close:** 63 + 113 + 256 (the Metal buffer budget) = 432 MB, which
  leaves ~70 MB of heap under a 500 MB total.
- **Budget raised (2026-11-04):** the user set the total to 1 GB for now ("just put it up to 1GB and
  we will deal with it later"), so 603 MB is within it. The shape below stays the fix.
- **What the shape needs** (not a tuning):
  - The Metal buffer budget must be set from the total. It is the total less the process base,
    the driver's fixed cost and a heap share, not a separate 256 MB.
  - The heap must not hold what the slabs hold. A wave's tapes are resident in slabs and also
    decoded in the heap, as `TapeDoc` (for the CPU's roots, rows and facts, decision 0032 §6).

## O40. Interpreted recursive groups run their rounds from the host

Found at list step 28 (2026-11-04). ruff-async251 and ruff-sim300, interpreted, are recursive
label groups: each round is a launch from the host, and they pass the 200-launch limit per
evaluation (`KERNEL_LAUNCH_LIMIT`), so they are refused (`BackendLaunchLimit`). On Metal the same
clauses run as fused groups whose rounds stay on the device.
- **Shape to take:** the interpreter's rounds run on the device (the persistent dispatch and work
  queue of the walks), not one host launch per round. Raising the limit is not the fix.
- **Test:** `lowered_clauses_find_the_same_roots_on_the_resident_interpreter` prints both.

## O41. Bounded rounds leave the unconverged frontier unmarked

Found at list step 29 (2026-11-04). The cross-file part runs in bounded rounds
(`typing::cross_rounds`, k = 2, I11). A recursive atom whose node may lie in another file reads
the previous round's rows; within a file the fixpoint is exact. On the vtab golden corpus:

| Program | Hop rows per round | Result |
|---|---|---|
| workload.rdl | 17,674 → 17,680 → 17,680 | converged after round 1 |
| calls.rdl | 5,044 → 5,104 → 5,112 (→ 5,112 at round 3) | unconverged at k = 2 |

calls.rdl's attribute chains (`found` through `base` across files) need a third crossing, so
with k = 2 they stop 8 rows short. `RERAIL_CROSS_ROUNDS=<n>` overrides k as a diagnostic.
- **Shape to take:** rows a further round would change are "pending" (03 §13: distinct from ⊤):
  the hop relations' last-round delta marks its consumers open, so a rule reached only through it
  reports unknown, not a missing finding. Today the run only reports `converged: false`.
- **Also:** each round reruns the whole cross-file part. The digest cutoff should be per document
  (rerun only the dependents of documents whose interface rows changed), as `typing::rounds` does.
- **Test:** `own_ingestion_derives_what_t_facts_do` prints both and asserts them.
- **A false negative on the runtime oracle (2026-11-06).** `typing::oracle_tests` scores calls.rr against
  `fixtures/python/calls`. `pkg.exported_helper(2)` in `main.py` reaches `pkg/util.py`'s `helper` through
  `pkg/__init__.py`'s `from .util import helper as exported_helper`: three files. At k = 2 the rounds stop
  unconverged (488 → 576 → 595 hop rows) and the call gets **no edge**, where CPython ran `helper`. That is not
  unknown, so a rule reads it as "calls nothing". With k = 6 the rounds converge after round 4 (596 rows) and
  the call agrees (84 agree, 9 depart, as the spike scored).
  - **Why the program cannot mark it itself.** Its opens are three-valued through negation. For example,
    `vopen(c) :- callee(c, e), !pv(e, _, _), …` and `attr_open(c, a) :- …, !found(c, a, _)` read the absence of
    a hop relation's row as "no value". Under a bounded round, that absence is not final.

- **Narrowed (2026-11-06, decision 0047).** k is 4. A program that declares `unconverged(0)` gets a pending
  round when round k ends unconverged. `calls.rr` marks unknown, in that round, a call with no target whose callee
  is not rooted outside the workspace (`python_calls_program_cut_short_is_unknown_not_no_edge`). Left open:
  - pending marking for the other programs (`calls_rust.rr`, workload);
  - the per-document cutoff, since each round still reruns the whole cross-file part. *Superseded by decision 0065
    (draft):* no rounds at all. The cross-file part becomes one fixpoint, so nothing is cut and nothing is
    pending. The cutoff (0064) was measured and withdrawn.

## O42. Terms, declarations and dependency summaries are not yet wired in

Found at list step 29 (2026-11-04). The three write-new modules of Phase 10 exist with their
own tests:
- `typing::terms` holds:
  - interned terms, with unions carrying an `open` flag, and widening past depth 4 or 8 members;
  - must/may traits, with `proven`/`possible` and `some`/`every`;
  - one-way instantiation;
  - the `type_term`/`type_arg` tables.
- `typing::declare` reads `rerail.types/v1` stub syntax, fills gaps only, and reports conflicts.
- `typing::deps` holds:
  - identity from `RECORD` and lockfiles;
  - content-addressed summaries in a machine-wide cache;
  - the re-export demand, with a budget per package;
  - Pending kept apart from ⊤.

Nothing calls them yet:
- **Programs:** the typing programs' `type_of`/`returns` still carry name atoms, not term ids.
  The step is a `type_term`/`type_arg` input pair, and the join of their lattice values through
  `Terms::union`.
- **Runner:** `corpus::run` reads no `rerail.types/v1` files and does not demand dependency
  summaries for the imports it resolves. Nothing writes summaries yet: a summary is the
  cross-file part's exported signatures, printed as a stub.
- **Variance:** variance for library types is not modelled. A check that needs it gives unknown.

**Closed (2026-11-04).** `logic::plan::corpus::typing`, through `corpus::run_typed`:
- **In:**
  - `rerail.types/v1` files, then the dependency summaries the corpus's imports demand (imports
    read from the syntax), become `declared_ret(q, t)` facts of the program.
  - workload.rdl's rule types a call through them only where the corpus does not define `q`.
- **Out:**
  - each node's `type_of` names are one union term (`x.node_type`, `x.type_term`, `x.type_arg`);
  - a declaration that differs from what the corpus derives for a name it defines is reported;
  - modules whose summaries are missing are pending, and their calls stay untyped (not ⊤).
- **Summaries:** `Typing::summarize` writes each installed module's functions, with the return
  names derived and the `from` imports re-exported, to the cache under its `RECORD` identity.
- **Test:** `typed_corpus_reads_declarations_and_dependency_summaries`. It summarizes a site,
  reports a conflict, types an application's calls from the summaries and a declaration, and
  leaves them pending with an empty cache.
- **Left (not blocking):**
  - Terms are made from the program's name sets after the run. A lattice whose values are term
    ids, with `Terms::union` as its join, is the next shape: it carries generics and
    instantiation into the rules.
  - Variance is not modelled.

## O60. The matcher atom's captures

The data-layer rules' relational inputs and budget evidence project captures (`capture.X`, `.startByte`, regex
groups, `transform` text): 1,672 projections in 54 clauses of 14 rules (`gate/data-layer/census/`). §12's atom
binds only the document and the node. The front end now reads `ast($D, $N, {…}, NAME: $v)` (a match that leaves
`NAME` unbound gives no row); `logic::plan` still refuses every `ast` literal. **Closure:** the §3 lowering returns
exported captures with each match.

## O61. A builtin catalogue

The data-layer programs (`cr/logic/programs/data_layer/`) use 30 builtins; only `@cat` is built. They are SQL's
scalars (`@rx_matches` 224 uses, `@concat` 66, `@substr` 60, …), three string helpers (`@drop_components`,
`@join_nonempty`, `@glob`), and tape builtins (`@within`, `@depth`, `@line`, `@id`, `@qualname`, `@class_path`,
`@class_prefix`). **Closure:** a fixed catalogue, each entry marked value-creating or not (§12's fourth
exclusion needs that mark), with `@rx_*` on §16's regex path.

## O62. The shortlex-least route lattice

Of the 70 recursions in the data-layer rules, 4 need a lattice beyond `set`, `flat`, `min` and `max`: `hierarchy`
(internal-dead-code, public-api-triples) and the two sink searches (serverplan-state-tripwires), each the least
route under (length, text). The native lattices are `i64`-valued, and `Min`'s widening jumps to its top, losing the
route. Without `widen` the declaration fails `UnboundedLattice` (`internal_dead_code.hierarchy_unwidened.rdl`).
**Closure:** a catalogue lattice bounded by its edge relation, or the length as a `min` lattice with the route in
the findings layer (t17 § 6).

## O63. Tree built-ins beyond `nearest`

28 recursions in the data-layer rules are tree walks that `nearest` alone does not express: subtrees, the
outermost function, depth (t17 § 3). The programs write them as `@within($x, $r) = 1` and `@depth($x)`, since a
new literal would change `logic/plan`'s exhaustive matches. Two walks through chosen kinds (dispatch `annotations`,
steps `payload_values`) remain Datalog recursion over tree edges, against §7. **Closure:** `within` and a
kind-filtered climb/descent (the matcher's `inside`/`has` with `stopBy`) as tree built-ins.

2026-11-06: the `.rr` built-in `within(n, a)` is no longer lowered to an id range or a recursive `below` relation
over `child`. The engine serves it from a relation read off the tree (decision 0050 §4): n's ancestors for a bound
n, a's subtree for a bound a. The converter writes it. The `.rdl` programs' `@within`/`@depth` and the two walks
are unchanged.

## O64. The data-layer's regex forbid clauses

109 forbid clauses regex the file's text (108 with line options, 8 with a line state machine) and 4 its path, in
14 rules; 9 rules have nothing else. `yaml::add` refuses them. PIAT §5.1 (`notes/t1` :446) found 107 expressible as
AST rules, 2 needing line positions, 4 path-only. **Closure:** the rewrite, done for the gate's pack.

## O65. Two data-layer stages outside the fragment

serverplan-quality's `call_paths` carries dotted text along attribute chains (`path || suffix`), a cycle through a
value-creating builtin that the front end does not check. `block_results` folds a JSON map in block order; over the
tree it is a negation of `raises` in its own component (`serverplan_quality.block_results.rdl`), stratified only by
depth. Both are declared inputs in the programs. **Closure:** a tape builtin for attribute-chain text, and a
bottom-up tree fold (or per-file lattice in the dispatch, §7) for `block_results`.

## O66. Device text matching is still a byte-at-a-time NFA

Decision 0038: text matching is chunk-at-a-time. The host slots are done. On the device, a text
test is still a `ByteNfa` stepped per byte per path, under a work limit. **Closure:** a chunked
automaton per file, where each workgroup computes its chunk's state map and the maps are composed
by a prefix scan, differentially tested against the host's DFA.

## O67. Memory over budget at 1,000 files

serverplan_quality on 1,000 data-layer files peaks at 1,055 MB RSS (t18 § 3), over the 1 GB
budget. The push and the first chunk's tapes reach 693 MB, the candidate sets (dense bitmaps, every
intermediate kept) 900 MB, the label passes 1,033 MB. The scheduler's chunk budget counts core facts
only. **Closure:** chunks sized by what a chunk holds (tapes, bitmaps, labels), candidate sets
freed as their last reader runs, and a 1,000-file run under the budget.

## O68. The Metal label path recomputes host facts per split

On Metal, serverplan_quality's labels cost 22.6 s CPU at 1,000 files against 17.2 s on the CPU,
with host facts 12.6 s against 5.2 s, and peak RSS 1.6 GB (t18 § 3). A batch whose label levels
outgrow the device is split in halves, and each half recomputes its facts. **Closure:** facts
computed once per batch and shared by its halves.

## O69. Rules over derived relations plan without statistics

The wave plans its rules once, before any chunk, and a derived relation is priced at 1,000 rows
whatever it holds. Bound base relations now carry their vtab indexes and distinct counts (t18 § 5),
but a rule joining derived relations still orders from that guess. **Closure:** the planner's
observed cardinalities (`stats.obs`) fed back between chunks, or a replan after the first chunk.

## O70. No ancestor relation over syntax ids

Authored rules find a node's nearest enclosing scope by joining every scope's byte range in the
file (t18 § 4): quadratic per file, 10 s of the authored logic's 24 s at 1,000 files. The wave's
rules cannot use `nearest` or `tape`'s subtree intervals without arithmetic. **Closure:** with O63,
an `ancestor`/`nearest` relation over syntax ids, bound with the chunk like the base relations.

## O71. The converter still builds `.rdl` fragments inside

The converter's emitter assembles each rule as `.rdl` text and translates the result to `.rr` at
the end (decision 0043). The output is `.rr`, but the emitter's own strings are the old dialect.
**Closure:** the emitter builds `.rr` (or the `Program`) directly; `rr::translate` then serves only
the copies, and goes with them at step 31.

## O72. The newer data-layer rules: converted where they can be, SQL stages by hand

The converter reads 100 rules: the 60 of the earlier rewrite (`candidates/rules-ast`) first, then the 40
newer ones from Data-layer-rule-expressivity (10 candidates, 30 certified). It converts 341 of 908 clauses.
Its largest gap is relational clauses: 174 clauses of SQL stages in 41 rules (93 of them in
bounded-composed-graph-paths). These are not converted; they are written by hand in `.rr` (list step 30).
The newer branch's revisions of 24 of the first 60 convert worse (admin-async-purity: 0 of 40 events, from
40), so the earlier versions are kept; four of batch 7's probes miss on them (quotes in comments, allowed
sources). The replay reads every batch: recordings 1-6 give `2350 155 153 0` as before; batches 7-8 and
the probes give `1638 20 16 0`. Of their events, 645 have configured oracles and 235 compare the message
or oracle metadata, which the replay does not read yet. The census still counts the first 60.
**Closure:** step 30's hand-written `.rr` for the SQL stages and the other gaps; the replay reads configured
oracles and messages; the census counts 100.

## O73. Hand-written `.rr` constructs that lowering refuses

Lowered now (decision 0046, reader section): `.violation` at a node, a span or a `@place`; `code "X"`,
`code {c} in "re"`, `tag` and `suppress` lines (`suppress TOOL "rule", … [over {c}]`, and `fix [safe] F "title"`,
kept as findings-layer settings and not applied yet); `match` captures, `match_all` and `match_group`,
arithmetic, the string functions, Soufflé's `cat` (any arity)/`max`/`min`/`to_number`/`to_float`/`%`, floats;
`@lower` as Unicode lowercase; `@dot("", b) = b`; `sum` (0 of nothing) and `join x by k sep "s"` (the distinct
rows of its value and keys; no `sep` joins as they are; `""` of nothing), aggregates over terms, inside terms,
and compared with each other; `.input` with parameters; `within`, `within_stop`, `nearest` (a named ancestor
negated too), `inside` and `has` over kind sets and over node-valued sets (`has` tests the set on the
descendant); `precedes_stop`; `!within_stop`, `!split_dot`, `split_dot` at the last dot only; negated
conjunctions and disjunctions (a helper relation per alternative); `must, may`; `unset`; recursion that makes
values bounded by the tree, a count, a shrinking text or a function between rows (`front::cycles`). A place is
the loc value `-2 - (path << 32 | line)`; its finding is at that path and line. A function with no value gives
no row. Still refused by name (census of 2026-11-06, clauses alone): `ordered by` (37 clauses: eslint-one-var,
the owner-code sketch), `@place` as a value outside a `.violation`'s head (3 clauses, 12 files),
`dfs_first`/`dfs_first_memo` (2), `~` patterns (1), `mean`, `=> edits`, a `.violation` at a span the engine does
not make (regex-capture spans: noqa-sec-ratchet, ruff TD00x; tokens), and `!within_stop`, `!precedes_stop` or
`!split_dot` with a node unnamed. `UnboundedValueCycle` still stops 15 library clauses (73 files) whose bound is
not one of those (`hex_place`, `norm`/`sci`, `unescape`, `slot_walk` over tokens, `b005_char`, `era_re`;
pyflakes' `negate`/`number`). `hand_written_rules_lowering_report` (with `RERAIL_RR_REPORT_EXTRA`) on
`final-rerail-shape-lowering` before this merge: 118 files, 75 lower; More-ergonomic-rule-syntax's 949 packs, 393.
`dfs_first` and `dfs_first_memo` are now computed natively over the edge relation (`logic::native::dfs_first`), so
`data-layer/lazy-architecture.rr` lowers (944 rules); they are no longer refused.
**Closure:** list step 30.

## O74. Translated programs declare the base relations themselves

Each translated program declares the engine's base relations (`node`, `text`, `child` …) instead of
`#include "core.rr"`. Decision 0046 made `core.rr`'s base relations for a program that includes it; the
translated programs are T's, and their recorded outputs are T's projection (comments not counted among
children, `text` of identifiers, T's `op`), so including `core.rr` would change what they compute. They stay
as they are until T's parity tests read T's view by its own names (as `op_t`). **Closure:** the programs
include `core.rr` and their tests read T's view by name.

## O75. The `.rdl` copies and T's tools

T's 10 programs (`typing/programs`, `plan/golden/programs`), the old parser `front::parse`, and T's
reference scripts (`front/rdl.py`, `magic/golden/generate.py`, `plan/golden/generate.sh`) are the only
`.rdl` left. **Closure:** step 31 (O19) removes them.

## O76. Base relations and inputs the engine does not make yet

Hand-written rules read relations the engine did not make: `parsed` and `string_annotation` (strings parsed as
code) were read as empty, each named in the profile. 2026-11-07: `parsed` is made as guest documents of the chunk,
asked for in rounds (decision 0055); `string_annotation` is made natively from Ruff's table
(`facts::vtab::syntax::annotation`), nested annotations found in their outer annotation's guest. `option`,
`option_item`, `file` and `package*` are made too (decision 0046). After the data-layer engine merge (ca930d974):
repository files are read (`corpus::inputs`: JSON, YAML, JSON Lines, TSV, lines, `dir=` inputs, and `input_error`
rows for missing or malformed files). The revision and stamped-artifact readers are still not written.

## O77. Lowering shortcuts in `.rr`

- A `match` capture is computed per row (`rx_group`, one cached regex per pattern), not as a chunk-at-a-time
  text test. Over a `source` or `src_line` span, captures are spans (`rx_span`, decision 0054); on a node they
  are still symbols. `match_all("re", sp, m)` and `match_group` (three and five arguments) are relations of each
  regex's matches in the batch's spans (`logic::strings::as_relations`, `facts::vtab::syntax`).
- `unset(x)` is the id -1. Its text is "" (`text(x, t)` of it lowers to `t = ""`), and `!unset(x)` is
  `x != -1` (decision 0046). Inside an aggregate, `nearest` or `ast` it is still refused.
- `within(n, a)` where nothing else gives `n` lists the nodes below `a` by following `child` down from the
  `a`'s the rule gives (named children only), not by reading the id range `a + 1 ..= a + size`; the native
  path has no range read yet.
- `earlier` compares `visit_index`, CPython's `ast.NodeVisitor` order (`facts::vtab::syntax::tree`), made for
  Python documents only (O80).
- The value-cycle check (`front::cycles`) accepts the bounds decision 0046 lists (parts of values, values from
  lower strata, guarded counters, keys moving one way along document order or shortening a string); a bound it
  does not know is refused, never run. It cannot see an involution such as pyflakes' `negate`, whose values stay
  two per input.
- An axis over a set of nodes or of derived kinds, and `precedes_stop`, are recursions over `child`, not the
  transducer's tape pass.
- `join` reads its symbol columns through the symbol table's text (`sym_text`), so a kind or field atom as a
  join value or key would read the wrong text; no library join uses one.
- `@place` needs symbols stored as atoms: text-mode SQL refuses it by name.
**Closure:** each is lowered as `core.rr` describes, or the description changed.

## O78. Where the `.rr` library and rules differ from the recorded findings

The library (`docs/rule-syntax/lib`) and the data-layer rules now live here; More-ergonomic-rule-syntax's pass is
brought over as it lands. Hand-written rules on the recorded events: 97 of 98 exact (was 64). Fixed:
- `resolve_export` kept no row for a name defined in its own module; `public_name` read `__all__ = []` as no
  `__all__`; public-api roots are public names, module roots functions only.
- `ref_site` counted a binding target as a read (`bind_only`).
- An unimported `contextlib.suppress(…)`/`suppress(…)` and a bare `TYPE_CHECKING` match by text, as the linters do.
- POSTOPDISPATCH001 asks for an `earlier` comparison; `cmp` reads `op_at` (O79).
- public-api's class methods follow the linter's `public_methods_of_class`: the class, then its bases
  breadth-first, the first public non-property method of each name (a property does not claim its name).
  Written five levels deep, since recursion that makes new keys is refused (`UnboundedValueCycle`).
- swallowed-exceptions follows the linter's scope depth: a raise in a def, class or lambda inside any handler does
  not count, so a handler inside a def inside a handler is reported (the `nested-handler` probe).
- missing-async-pairs compares the linter's `node.args.args`: ordinary parameters in order, not `/`-only,
  `*`-only, `*args` or `**kwargs`.
- The replay says when a hand-written rule does not lower (it fell back to the converted rule silently).

Left: public-api's chainable-builder reads `text` of a return annotation. `core.rr` (and the rule-syntax pass's
ALIGN-FINAL-SHAPE.md) give every node a `text`; the engine's is T's, identifiers only (1 event).
**Closure:** `text` of every node in the engine, as a value and not only as a test (0042 covers tests). Done for
programs that include `core.rr` (decision 0046), up to 256 bytes or token-like (O80).

`logic::profile::tests::explain_shows_engines_plans_devices_and_decisions` failed once in the parallel `logic::` run
at load 40-55 and passes alone: shared state between tests, not looked into.

## O79. `op` of a comparison: closed

**Revised (decision 0046, the coordinator's choice):** `op` follows `core.rr`, every anonymous token of a node in
every language, for programs that include `core.rr`. T's view below is `op_t` in both shapes, and `op` in T's
shape; the parity test reads `op_t`. `op_at` is every non-punctuation token in order, so Rust's `&&` of a `let`
chain has one.

Before:
`op` stays T's: one row per node, binary, boolean and unary operators, augmented assignments and `async`, so T's
parity tests hold. A comparison has several operators in order (`a < b <= c`), so it has its own relation, the one
`core.rr` already declares: `op_at(n, i, s)`, n's operator tokens (the `operator` and `operators` fields) from 0.
The library's `cmp` reads `op_at(c, 0, o)`. Not yet: Rust's `&&` in a `let` chain (`complexity/rust.rr`), whose
tokens have no field.

## O80. `core.rr` base relations not made yet, or made with a bound (decision 0046)

- `parsed` is made as guest documents of the chunk asked for in rounds (decision 0055); `string_annotation` (O76)
  natively, from Ruff's table (`facts::vtab::syntax::annotation`). A guest is parsed as `( … )` and judged by
  CPython's module grammar, where W asked `ast.parse(value, mode="eval")`: the two can differ (`(yield x)`).
- `match_all`, and `match` captures as spans, are made over `source` and `src_line` spans (`rx_span`, one scan
  per file and pattern) and over symbols (a native relation): decision 0054. 2026-11-08: over a node of a kind
  and over an earlier capture too, each distinct text matched once per batch (decision 0081); a node bound to no kind is still
  refused, and the scan is per distinct text, not chunked (O122). `match_all("re", sp, m)` and `match_group` over any
  span are relations the batches make (O77). (`docof` of a span is made: the lowering reads `core.rr`'s `docof` as
  an input.)
- `text` of a node longer than 256 bytes that is not token-like has no row; a test on it is 0042 §3's file scan.
- `syntax_error(d, l, msg)` for Python is CPython 3.13's `ast.parse` verdict (`cpython::syntax`, through
  `facts::vtab::syntax::python_syntax`, Ruff's parser `ruff_python_parser` 0.0.10, the last on the pinned
  toolchain), not tree-sitter's; compile-time errors CPython's compiler raises after parsing (`return` outside a
  function) are not checked. `cpython_syntax_error` is the same verdict under the converter's name. A document with
  one is withheld in `core.rr`'s shape (no facts but `doc`, its path and `syntax_error`), and `.on syntax_error
  report CODE [at line 1]` and `.on input_error report CODE` report (2026-11-07); T's shape keeps every document;
  the tape's `syntax_error(d)` is tree-sitter's ERROR/MISSING.
- `static_value` is `value`: file-local constants are not followed.
- `tsconfig*`, `package_export`, `git_*`, `signed_list` and `list_len` are not read. `input_error` has its
  `missing` rows (a `.input` file that is absent, decision 0054); the other kinds wait on reading files (O76).
- `earlier` compares `visit_index`, CPython's `ast.NodeVisitor` order, made for Python documents only
  (`facts::vtab::syntax::tree`); other languages have no `visit_index` row, so no `earlier` row.
**Closure:** each made as `core.rr` says, or `core.rr`'s description changed.


## O81. A program that derives a relation `core.rr` makes an input: closed (decision 0047)

- Decision 0046 made `docof` a `core.rr` input, and `sql::select::input_columns` names its columns `n, d`.
  T's programs (`typing/programs/workload*.rr`, `calls`) still derive their own `docof(n, d) :- node(d, n, …)`.
  `plan::sql_col` names columns by `input_columns` for any relation of that name, so the SQL reads `t4.d` from
  a derived table whose columns are `c0, c1`: "names no column in scope".
- Nine `logic::plan::tests` are ignored with this item (they fail the same way on final-rerail-shape-rr-align).
**Closed by 0047:** both. The planner names a column by `input_columns` only for an input
(`plan::Model::col`); the reader refuses a file that declares or derives a `core.rr` relation; T's programs
read `kind` and `docof` as inputs. The nine tests run again.
Typing-support first closed it by renaming (`plan::private_derived_inputs`: a derived relation under a base
relation's name planned as `<rel>_rr_derived`); the merge into final-rerail-shape keeps 0047's mechanism, not both.

## O82. Metal runs abort in a debug test build

Two tests that dispatch to the device abort the test process inside `metal-0.33.0` (`lib.rs:82`): Rust's
debug-build check of `slice::from_raw_parts`' precondition (pointer aligned and non-null) fails, and it is a
non-unwinding panic. They are `logic::plan::tests::t_programs_with_the_transducers_nearest_derive_t_outputs`,
ignored with this item, and `typing::file_local::tests::the_device_finds_the_cpus_nearest`. Because the panic
cannot unwind, it ends every test that shares the process, so a filtered run over `typing::` must skip the
device tests. It was not checked whether this predates O81's change; nothing in that change touches device
buffers. **Closure:** find the empty or unaligned buffer passed to `metal`'s slice constructor (likely a
zero-length contents pointer), and both tests pass in a debug build.

## O83. Metal `transducer::packed` suites fail when run together; `explain` fails now and then: closed (2026-11-06)

Found (2026-11-06). Of the 750 `transducer::packed` tests with `--features metal-tests`, about 440 failed in
one run. There were five causes:

1. **Poisoned locks.** `METAL_EXECUTION_LOCK` and `METAL_ALLOCATION_LOCK` guard no data, but one panicking
   test poisoned them, and every later test failed with `BackendFailure` (443 failures down to 43).
   Fixed: both locks recover from poisoning (`PoisonError::into_inner`).
2. **Tests written for the old 600-launch limit.** The launch budget is per thread and per evaluation
   (`thread_local! EVALUATION`), so it is not carried from test to test, as this item first guessed.
   Copied tests wrote the limit as the literal 600 (or 599), and three launch-guard tests as 11 x 9 = 99.
   Fixed: they use `KERNEL_LAUNCH_LIMIT`. The source scan for bypassing encoders skips `*_tests.rs`
   test modules. 7 tests pass again.
3. **G's grid executor over the bounds since list step 13.** 35 tests of the grid executor (superseded,
   decision 0028, O33) need more than 200 launches per evaluation, or more than the 256 MiB device pool
   (was 2 GB). 22 of them pass when the limit is set back to 600. They are ignored with this item, and they
   go with the executor at list step 31.
4. **A real bug: the label sweep did not compile.** `metal_label_sweep.metal`'s `SWEEP_NODE(local)` declared
   `uint local=(local)` inside `for(uint local...)`, a redefinition and a read of itself (since b995b972e,
   11-04). The sweep runs only under `RERAIL_HOST_SWEEPS`, but `MetalBackground`, which napi's `"metal"` uses,
   compiles it before the JIT. So the background compile always failed, and `"metal"` ran the interpreter
   and never swapped to the compiled kernel. Fixed by renaming the variable. The replay gives the same
   findings.
5. **A process-wide device pool.** `METAL_BUFFER_BYTES` (256 MiB) is shared by every thread, which is right
   for production, but parallel tests then fill it for each other. About 16 tests fail only in a batch
   (`BackendCapacity`, or a peak-bytes assertion). With `--test-threads=1` all of them pass.
   Fixed (user decision: a resource lock on memory, so that small tests still run together):
   `metal_test_pool.rs`. In a test build each thread has its own pool ledger and peak, and the driver's
   process-wide figure is left out, so a test sees the pool as it would alone. A weighted lock over bytes
   bounds what all tests hold at once at 1 GiB (the RAM budget). A reservation that would pass it waits for
   others to release, for at most 30 s, then goes ahead: the bound is on memory, never on results.
   Production is unchanged.
- The admission spike's oracle snapshot (`execute_roots_from` as at f5de8998d) is ignored. The interpreter
  has since gained set mode, lazy cursors and `Reach` pruning.
- `explain` (`logic::profile::tests`): a concurrent corpus run's `profile::reset()` raced it. Profiled runs
  and the profile tests now hold `profile::one_run()`. `logic::` passed twice at load 50 or more.

**Closed:** `transducer::packed` passes in one parallel run: 689 passed, 0 failed, 63 ignored (the grid
executor's, and the admission spike), 104 s at load 40-73.

## O84. The Pulling-it-all-together merge (41f1c88e5): what was checked, and P's tests that need P's addon

**Native code.** Every native file P changed since the merge base (`5cf4246e`) was checked hunk by
hunk against this tree. All but 59 hunks are here. 35 of the 59 are in `matcher/anchors*`: P's
version of that module is `matcher::p_anchors`, identical to P's but for this branch's edits, and
W's `matcher::anchors` stays. The other 24 are one choice, O18's and O20's: P's relational clause
and SQL anchor engine are production in P and test-only here (`CLAUSE_KINDS` and
`ClauseKind::Relational` under `cfg(test)`, `TapeDoc`'s P accessors `#[cfg(test)]`, W's
`select`/store-restore path in `workspace/evaluate.rs`, W's profile fields kept). None was lost.

**TS and scripts.** Taken from P as they were (the schema's `filePath`, `pythonFString`,
`calls.graph`, `query.regex.lines`, `layout.line`, which this native compiles; the `data-layer`
pack in the catalog, whose `rules` list is empty). Checked against this branch's addon (a
`--profile profiling` build) and against P's own (`Pulling-it-all-together/native/*.node`, built
after P's last native commit):
- Tests that compile a `relational` clause fail here with "unknown field `relational`" and pass on
  P's addon. `vitest.config.ts` leaves them out unless `RERAIL_RELATIONAL_TESTS=1`.
- `test/conformance/data-layer/ports.test.ts` fails the same way on both addons (batch 01: 1 false
  positive and 2 false negatives; batch 02: 1 and 1; batch 02 probes: 6 and 0; batch 04: 6 and 2).
  It is P's open parity, and is left out the same way.
- `execution-owner.test.ts` calls `native.CodeRuleDslExecution`, which neither addon has.
- Not the merge's: `audit-cli.test.ts` has 1 failure before and after it.
  `checkout-code-rule-router.test.ts` had 17 failures before and has 1 after it (a 5 s timeout
  under load). The `gate/data-layer/test/conformance` copies fail to load before and after it
  (imports relative to `gate/`).

To close: when O18 is decided, either the relational clause becomes production and these run by
default, or the tests move with P's stage.

## O85. Guests (decision 0055): what is not built yet

Decision 0055's guests are built for `.py` and `.rs` corpus runs. Decision 0058 built hiding, kind names across
grammars, normalized strings and the tape operations (`@parse`, `@splice`, `@unparse`). These parts are not built:

- **Hosts the corpus runner does not collect** (Markdown, HTML, Vue, Blade), and the matcher path's
  Vue and Blade guests as these documents (0055 step 1).
- **Global embeds** (an `embed` reading another chunk's facts).
- **The memo and guests.** A memoised file's rows include its guests' rows. Guest keys depend only
  on the host's path, place and grammar, so they are stable. No test yet checks a memo hit against
  a fresh run on a file with guests. Value documents are O96.
- **Cost on the 9,010-file repository** (guests made, distinct texts, parse CPU): not measured yet.
- **The shared tape cache races:** O95.

To close: each part is built or decided against in a decision record.
## O88. `calls.rr` against Jedi over a real corpus

Decision 0047's condition for moving `calls:` is the runtime oracle, which `calls.rr` meets. A Jedi comparison over
a real corpus (the Python pack's django, `fd91518f`) is a broader accuracy check beside it, not a gate.
`calls::python_tests::benchmark_python_calls_corpus` scores only the native resolver. The spike scored `calls.rdl`
(99.18% of 33,309 calls, t9 §7.4), but its scorer was not copied.

**Closure:** a Jedi 0.19.2 recording over django (`fixtures/python/calls/generate_corpus.py`), a scorer of
`calls.rr` over the same files through `corpus::run`, and the run recorded in t20.

**Closed 2026-11-06:** run, and recorded in t20. 99.2% of 20,364 calls both decide agree; 142 differ, mostly
through decorators. Making the 7,286 calls the program leaves open, which Jedi decides, is the accuracy work
left.

## O89. The type relations' first draft: what they leave out

`calls.rr`'s type relations (decision 0047, point 3; t20) answer at the positions whose value the program models
(`vpos`), from its points-to values. Left out, each giving `type_open`, never a wrong class:
- **Positions:** any expression no rule of the program reads the value of (an operand, a subscript, a
  comprehension's element).
- **Declared types:** parameter, return and variable annotations are unread. A parameter's type is open.
- **Values:** functions, bound methods and `super()` objects get no class. A class with a metaclass the workspace
  defines gets none either.
- **Generics:** names only, no arguments: `list`, not `list[int]`. Terms are interned per node by the corpus
  runner (`x.node_type`) as unions of names.
- **Exceptions and other builtin hierarchies:** only `bool <: int` and `<: object`.
- **Possible:** past what is listed, `possible_open`.

**Closure:** annotations read (gap 2), positions widened to every expression a rule asks about (by demand), and
generic arguments carried as terms.

**Annotations read (2026-11-07, `5cbb86a96`).** Parameters, returns and class attributes now declare what they hold,
trusted as a type checker trusts them:
- a workspace class C means an instance of C or of a workspace subclass, the closed world `self` already assumes;
- a builtin class means its value, and `None` means None;
- `A | B`, `Optional[A]` and `Union[A, B]` are their members;
- a generic is its class, so `LazyResult[T]` is `LazyResult`;
- `type[C]` means C's class objects, and `Self` means the method's class.

A closed declaration closes the parameter, the function's calls and the attribute. A decorator declared
`(fn: F) -> F`, with F a type variable, keeps the decorated function. Still open, as a lower bound:
- **Quoted annotations** (`-> "LazySeq[int]"`). The facts held no parse of a string's contents; since
  decision 0055 `parsed(s, "annotation", r)` has rows, and `calls.rr` does not read it yet. The data layer
  has 146 quoted returns against about 118,000 annotated ones.
- **External classes** (`p: Path`). They get no type name, so a pathlib receiver is unknown rather than "not
  lazy". This needs the classes' supertypes, from stdlib summaries (t20 gap 6).
- Type variables, `Any`, `Protocol`s, generic arguments, and variables annotated in a function or module body.

## O94. `calls.rr` over the data layer's l1–l20 packages: cost and convergence

On the data layer's l1–l20 packages (489 files, debug build), `calls.rr` takes 101.6 s of CPU with a peak RSS
of 835 MB. Its rounds do not converge by k = 4: hop rows 498,860 → … → 753,630, so the pending round runs. On 84
of those files it takes 15.3 s, 13.0 s before annotations were read. The Jedi score was therefore taken on the 84
files (t20). LEG-083's own scope (`impl/`, `data_layer/` and every `l<n>` package) is several times larger.

**Profiled** (`RERAIL_PROFILE`, 2026-11-07; spans now cover the round bookkeeping, `8224f2c8a`, and each round's
hop rows and changed documents, `6940b8a49`). 489 files: 86.4 s user, 777 MB; local 27 s, cross 52 s.
On the 84 files (13.9 s user, load about 4): local 3.4 s, cross 8.7 s, of which the six rounds are 8.1 s at
about 1.35 s each. Every round re-runs the whole cross part over every document, but few documents change:

| round | 0 | 1 | 2 | 3 | 4 | pending |
|---|---|---|---|---|---|---|
| documents whose hop rows changed (of 83) | 82 | 65 | 43 | 26 | 10 | 30 |
| `pv` rows | 9,426 | 15,191 | 18,098 | 19,947 | 20,146 | 20,155 |

The rest of the cross part: copying and measuring hop rows 0.9 s, digest 0.35 s, sorting the previous rows
0.2 s, loading the done inputs 0.45 s. In the local part, column statistics are 1.45 s of 3.4 s (`node`,
constants, `child`, the three `rr_nearest_*`); they now take one sort per column, and are still exact.

**Since then** (2026-11-07, `88cda9248` … `629b15b33`):
- Two declared-type rule shapes multiplied out: 14M of 28M join rows on typeshed's closure. They were declared
  returns joined to subclasses before calls, and module TypeVars per function.
- `found` now covers only the names that lookups read; the corpus runner does not apply `.demand`. Declared
  parameter and attribute classes are expanded to subclasses only where read.
- The rounds sort once, keep their base inputs loaded, and re-measure only hop relations that moved.
- On the 84 files the outputs are unchanged. The rounds converge after 7 runs: rounds 4 and 5 change 12, then 3,
  documents.

**Per-document cutoff: design.** Every recursive read across documents already goes through `__prev`: a hop is
keyed by its first node column, `qv`'s included. A round over the dirty documents D gives the full round's rows
if:
1. Every cone rule whose head has a node column carries `docof(h, d), rr_dirty(d)`.
2. Reads that are not hops see the other documents' last rows. These are non-recursive atoms of cone relations,
   and atoms in rules whose head has no node column (`need_name`). They read `X__all = X ∪ X__kept`.
3. A relation without a node column that changed in the round sends the round back to all documents.
4. D for round r + 1 comes from a touch program: each across rule's input literals over the rows its `__prev` atom
   gained or lost, to `docof(head, d)`.
5. Rounds 0, 1 and the final round run in full.

On the 84 files that saves about a quarter of the rounds (about 1.5 s of 5.8 s), and it lets k rise until the
rounds converge for little cost.

**Closure:** the cutoff above, then k raised so the rounds converge; then the full l1–l20 Jedi score and the
LEG-083 files.

## O90. Cross-file costs `calls.rr` works around

Profiling `calls.rr` on django (t20, "Cost of `calls.rr` on django") found engine costs. The program avoids them by
how its rules are written; another program would meet them again. Measured on django utils + db (172 files),
debug build, load about 7.

1. **Hop rules fell off the native engine. Fixed** (`bcffa4c26`). `cross_rounds::bound` adds `@doc($a) = @doc($b)`
   or `!=` to every hop rule, and `join_spec` had no native step for a call term other than `@cat`. The native set
   engine declined the whole component ("`@doc($v) != @doc($s)` has no native step"), and its probes ran as SQL
   through the vtabs, where costs 2 and 3 apply. A `deq`/`dne` step now compares the two nodes' documents
   (`a >> 32`). The component is native, with composite indexes and live fanout. Cross-file CPU went from
   25.3 to 18.1 s, SQLite statements from 6,600 to 605, and peak RSS from 418 to 350 MB, with the same outputs.
   The earlier `cross_rounds` change (a join through same-document atoms is not a hop) removed most of these
   tests from the start.
2. **SQL over a vtab looks up one column, priced at one row per value. Fixed** (`f443cc6f8`, `2a69daa08`,
   `210a2d82b`). `best_index` (`facts::vtab`) used one equality constraint, the one with the smallest
   `estimated_matches`. The in-memory source (`setfix::Stored`: the cross part's inputs, `__prev`, `__done`)
   answered 1 for every column, so a tie picked whichever constraint SQLite offered first. Option 4 (native `min`/`max` and
   text functions) took SQL off `calls.rr`'s hot path. SQL stays a product surface, though: other applications
   query rerail's relations through these vtabs, and the SQL executor (`--modes ""`) must stay usable. So the
   index shape is fixed for both tiers:
   - **Inputs (vtabs; immutable, so an index costs no insert).** Every usable `=` goes into one filter. `Stored`
     answers from a lexicographic order over those columns. The order is built on first use, holds 4 bytes a row
     and serves every later search whose columns are its prefix. Other sources probe the most selective column
     and check the rest before SQLite sees a row. Estimates come from distinct counts (`Stored`); for several
     columns, the default treats them as independent, bounded by the most selective. A range is taken only
     when it beats the estimate for all the `=` together.
   - **Derived tables (SQL executor; mutable, `WITHOUT ROWID`).** Each secondary index is a full copy of the row,
     written on every insert. So the table's primary key serves every search on a key prefix. The other searches
     share lexicographic orders, one order per chain of nested column sets: a minimum chain cover, as in
     Soufflé's index selection (Subotić et al., VLDB 2018). A search either extends an order whose columns are a
     subset of its own, or starts a new one.
   - **Semi-naive rounds.** The new rows come from an anti-join on the key (`NOT EXISTS`, one primary-key probe per
     candidate), not `EXCEPT`, which scanned the whole relation every round.

   On slice-utils (49 files), SQL executor, debug build, with the same outputs:

   | Change | Measure | Before | After |
   |---|---|---|---|
   | Chain-cover orders on derived tables | index statements | 99 | 28 |
   | Composite vtab filter | VM steps | 148.5M | 50.5M |
   | Composite vtab filter | CPU (load ~146) | 7.9 s | 5.5 s |
   | Anti-join for new rows | VM steps | 50.5M | 43.2M |
   | Anti-join for new rows | full-scan steps | 3.87M | 3.02M |
   | Anti-join for new rows | CPU | 5.4 s | 5.4 s (noise) |

   The worst statement before the fix was `iattr`'s probe of `field` by the constant name rather than the bound
   child: 65M VM steps across 6 runs. With the fix it drops out of the top statements. The native engine on the same
   slice was 1.94 s. The rest of the SQL executor's gap is the `cand` temp tables and their union steps.
3. **A kind test in a recursive step. Gone with 1; the cause was 2.** The cost blamed on `kind(n, "assignment")`
   came from a lookup of `field` by its constant `f`, not the bound `c`, per the plans' index numbers. It was the
   1-per-column tie of 2. With 1 those rules run as native joins, and with 2 the SQL path probes `c`.
4. **Probes start from seed estimates in every round. Fixed** (`a1d580128`, `a450fc0f2`; about 3%, near noise,
   since the native engine's replans already correct the first waves). Recursive relations are priced at their seed rules'
   estimated rows, and derived columns have no distinct counts. The probes are replanned from live fanout after
   the first waves, which are the waves with the largest delta. Plans and statistics are not kept between rounds:
   the native component replans in every round (`replans=6` over 6 loops). On the SQL path, the bad `pv`-before-`retval`
   order ran in the first 2 waves of each of the 6 rounds (12 runs, 169M VM steps).
   **Solution:** price round r's recursive relations by round r − 1's measured rows and distinct counts (the
   `__prev` totals are already measured), and keep each probe's replanned order for the next round.
5. **Every round reruns the whole cross-file part. Fixed** (`a450fc0f2`): `cross_rounds::cone_split`. Same 13
   outputs; utils + template 2.67 → 2.24 s, utils + db 17.4 → 15.4 s. The gain is below the estimate, because
   `call_target` and the relations next to it are read by hop relations and stay in the cone. `run_rounds` runs the bounded program in full each
   round. On 172 files, the strata no hop relation reads (the type relations, `call_target`, the outputs) take
   about 6.3 of 18.1 s over 6 rounds.
   **Solution:** split the bounded program at the hops' upstream cone. Rounds run only the cone. After the last
   round, the rest runs once over the cone's held relations, bound as inputs as the cross part binds the exports.
   This saves about 5 s (29%) here.

**Closure:** 1–5 done; left: `calls.rr`'s rewrites (`alook`, `pass`, `fv`, `fret`) checked against the
plain rules on the same slices.

## O91. Bridged typing answers are recomputed every run

A rule that reads `typing_<program>__<output>` runs the program in full before the rule (decision 0056). The rows
are a derivation of the program's digest and the run's sources (its manifest, 0053), but they are not recorded as
one. An unchanged corpus therefore reruns the typing program's rounds.

**Closure:** the rows are kept as a derivation (03 §14, `content_store::derivation`), valid while the program and
the files its rounds read are unchanged, and a run with a valid derivation reads them instead.

## O92. The device's nearest is off the corpus path

Since decision 0057, the corpus runner's kind-set axes are made per looked-up node on the host (`within::Axis`).
The file-local FIX on Metal (pointer jumping, `typing::file_local::nearest` with `Binding::Metal`) now runs only for
typing's file-local part. A join that scans an axis makes every node's rows on the host, one climb each.

**Closure:** a scan of an axis over a large batch is measured on both paths, and the device solves the whole set
where it is faster (the rows then filled from its answer), or the device path is recorded as typing's only.

## O93. The data-layer pack's rules do not run on the full corpus

These are the 68 checks in `gate/data-layer/packs/data-layer/candidates/rr` (Data-layer-rule-expressivity), measured
with `RERAIL_HAND_DIR=<that dir> RERAIL_MEASURE_HAND=only`. They need the data-layer engine's `core.rr` relations,
merged at ca930d974. Of the 68 run one at a time on s50:

- 50 run;
- 7 take over 40 s: conditional-architecture, connection-service-architecture, data-layer-facade-purity,
  hnsw-architecture, lazy-adapter-ownership, matching-architecture, versioned-lifecycle-legacy-surfaces;
- eager-resolve takes over 45 s once its directory inputs are read;
- the rest fail (below).

The 49 that run together:

| corpus | CPU | instructions | peak RSS | violations |
|---|---|---|---|---|
| s50 | 17.2 s | 205e9 | 1.57 GB | 606 |
| s200 | 100.5 s | 1,064e9 | 5.19 GB | 2,878 |

Converted rules on s200 take about 2 s. On s200, the costs are:

- **`gis_before`, 40.7 s of 100.** It is `gis_before(i, n) :- kind(n, "call"), earlier(i, n)`, every node before every
  call of its file, which is quadratic in a file's size. Its readers bind both arguments. A filter (each rule
  functional lookups and tests of its head) should be inlined, not built. A draft inliner (`/tmp/ai-trash/
  simplify-inline-filters.rs`) was correct, but it moved base lookups into rules where the runner had not
  measured their statistics or held their columns (`no statistics for docof.n`, `syntax_error has no column l`).
  The runner must measure what the simplified program reads.
- **`cc_line`, 14.9 s.** It runs in the cross-file part, in SQLite, because `@prefix(…) = 0` has no native step. It
  crosses the 15 selected documents with every trimmed line (504k) before looking up `split_line` by document.
  `split_line`'s statistics are not measured, so the lookup is estimated at 2M rows.
- **Memory.** It grows with which rules are combined, not with their number: 12 rules take 417 MB, 22 take 1.27 GB.
  A single rule's run is 220–290 MB; the test process adds about 650 MB.

Failures, by kind:

- **A span stored as a symbol** (bounded-composed-graph-paths, schema-architecture, bootstrap-engine-invariants):
  `match` over a span gives span captures (`core.rr`). A column declared `symbol` then holds a span, and
  `cat()` stops with `no symbol <id>`. The reader should refuse the program and name the column.
- **`no such table: rr_ctx<k>`** (cached-pure-lazy-allowlist).
- **`rule for x(…) reads nothing`**, a constant-head rule left with no body (connection-structural-boundaries,
  post-op-side-effects, pure-lazy-runtime-purity).
- **An aggregate in recursion** (lazy-architecture).
- **Revision and stamped-artifact readers**, not written (plan-composition-architecture, test-budget).
- **The `$O[:-$K]` slice pattern**, not lowered (versioned-table-names).

**State on 2026-11-07** (t10 §2k). The costs above are fixed: `gis_before` is inlined, `cc_line`'s computed view is
inlined, text tests are native, and relations are released after their last reader. The 49 rules on s200 take 39.5 s
and 1.56 GB. Thirteen more rules now run, 62 in all:

- the 7 that took over 40 s take 0.6–1.6 s each, and eager-resolve 1.0 s;
- bounded-composed-graph-paths, cached-pure-lazy-allowlist, connection-structural-boundaries,
  post-op-side-effects and pure-lazy-runtime-purity no longer fail (the merges fixed their failures);
- eager-resolve was slow only in the pack: demand split a rule whose kept atoms only the demanded read joined, a
  cross product (`pre_1199`). Demand now refuses that split.

The 62 rules together on s50: 19.2 s CPU, 303.8e9 instructions, 637 violations (each rule's count equals its count
alone), and a peak of 1.43 GB, **over the 1 GB budget**.

**Later on 2026-11-07** (t10 §2l). schema-architecture and bootstrap-engine-invariants run too: 64 rules.

| corpus | CPU | instructions | peak (the run's) | violations |
|---|---|---|---|---|
| s50 | 15–17 s | 302e9 | 913 MB (995 MB profiled) | 637 |
| s200 | 43.0 s | | **2.39 GB** | 2,960 |

s50 is within the 1 GB budget; s200 is not. On s200 the footprint is about 1.5 GB at the end of each chunk's
file-local run, and binding the cross-file part's inputs adds 800 MB (the base relations held for it, and the
exports read back from `x`).

Left out, by cause (none of them is a quick fix):

- **lazy-architecture**: was refused as an unbounded value cycle (`la_cls` climbs `ast_parent`, whose decorator
  rule moves to a later node in preorder). Runs since 2026-11-07: `front::cycles` also measures depth (t12). Its
  speed and memory are O116.
- **plan-composition-architecture, test-budget**: they read CI artifacts (`artifact="plan_composition_probes"`,
  `artifact="test_collection"`). Stamped-artifact readers are not written. A grep reader does not replace the
  probes (t11, O118); test-budget can count tests statically instead (t13, O119).
- **versioned-table-names**: runs since 2026-11-07 (O103).

2026-11-07: `.scope` was not applied at all before then, so the pack's counts above included findings in test
files and outside each policy's directories. With it, the 66 running rules give 32 findings on s50 and 58 on s200
(637 and 2,960 before). Peak RSS on s200 is 1,463 MB, 1,163 MB without lazy-architecture.

**Closure:** the pack's rules run on the full corpus, within the budgets, with every rule left out named.

## O95. The shared tape cache races

`core::build_cached` writes tapes to one SQLite store in the temp directory, and every run on the machine shares it.
Under parallel tests, a write sometimes fails with `FOREIGN KEY constraint failed`. Guests write many small, equal
texts, so they hit it most. A guest whose cache write fails is parsed without the cache, and the run counts it
(`guests.cache_failed`). Primary files use the same call and can fail the same way. The likely cause is a collector
removing a source between `put_source` and `put_tape`; this is not investigated.

The memo shares the store, and so the race. `value_documents_are_memoised_with_the_files_that_asked` passes alone.
In a parallel `logic::rr::` run it sometimes fails, in one of two ways. Its first run's write fails on the same
foreign key. Or its second run finds no memo rows, because another test's background collector deleted them as a
dead plan's rows between the two runs. A test cannot take a store of its own, because `RERAIL_TAPE_CACHE` is read
process-wide.

**Closure:** the write is made atomic (source and tape in one transaction, or a collector that cannot remove a
source a writer holds), with a test that runs parallel writers against one store; or the cache is made per run.
A run can also name its store, so a test can hold one of its own.

**Closed (2026-11-07).** A run's sources, tapes and memo rows become live only when its manifest is recorded, at
the end of the run. Until then, collectors of other runs could take them from under it. There were four such
races; each now has a fix in the store:

- **A run's sources and tapes.** A run holds them (`ContentStore::hold_sources`). Each source the run's handle
  stores or finds by hash is pinned under the run's owner and lease. So is each tape it stores. The pin is taken in
  the same transaction as the write or lookup, so no pass can come between. A run that dies leaves its pins to
  lapse with their lease (6 h). A cold insert gets its pin within its existing write. A source found by hash takes
  one write. A stat-index hit takes none.
- **A tape of a collected source.** `put_source_and_tape` stores or finds the source and stores its tape in one
  transaction. `core::build_cached_source` returns the source's id as it is after that write.
- **The run's memo plan.** The plan digest is the same across processes, so an earlier, evicted manifest could
  leave it dead, its rows being deleted while this run writes them. The run holds the plan
  (`hold_corpus_plan`), which makes it live; a decision keeps a held plan live.
- **A stale decision.** `keep_corpus_runs` read its manifests outside the transaction that applied the decision,
  so a plan recorded in between was marked dead. Now the decision is read, made and applied in one transaction.

`put_facts_memo` still writes nothing for a source that is gone, as a last guard (that file is evaluated again next
time). The runner's `HeldPlan` takes both holds when a run is memoised, and releases them when the run ends, after
its manifest. Tests:

- `a_source_collected_before_its_tape_is_stored_again_with_it`;
- `a_held_source_survives_collection_until_released`;
- `a_held_plan_stays_live_until_its_run_releases_it`;
- `parallel_writers_and_a_collector_share_one_store`.

`value_documents_are_memoised_with_the_files_that_asked` passed in 12 of 12 parallel `logic::rr::` runs (it had
failed in 3 of 3 before). The guests' fallback (`guests.cache_failed`) stays, for a store that refuses a write for
another reason.

## O96. Chunks that read value documents are not memoised

Decision 0058's value documents (`@parse`, `@splice`) belong to no file. A chunk whose outputs hold a row that
locates one cannot attribute it, so the chunk is not memoised (`memo_unattributed`). Attributing a value to the
files whose rows asked for it needs the demand's provenance.

**Closure:** value rows are attributed to the files that asked for them, with a test of a memo hit against a fresh
run; or a measurement shows unmemoised value chunks cost too little to matter.

**Closed (2026-11-07):** a value's rows are attributed to every file of its chunk that asked for it, and the split
places a value's tape with the atom that gave its text (0058 §4). Test: `value_documents_are_memoised_with_the_files_that_asked`.

## O97. A combined run's programs and their guest bounds

Each program of a combined run is bounded by its own `.guests` setting (decision 0058 §1), but all programs share
one batch. No test runs a program with `.guests off` beside one without.

**Closure:** a combined-run test where the two programs see different guests.

**Closed (2026-11-07):** `embed` and `embed_value` are shared across a combined run's programs, so one batch makes
the guests. Test: `combined_programs_see_guests_by_their_own_policy`.

## O98. Kind sets across grammars

`nearest`, `inside`, `has` and `within_stop` take kind sets resolved in the run's grammar. A scoped SQL node's
`nearest(n, a, {"select_statement"})` names no SQL kind. The scalar kind and field constants are qualified (0058
§2); the sets are not.

**Closure:** a kind set over a scoped variable is resolved in its grammar, with a test over a SQL guest.

**Closed (2026-11-07):** derived relations and kind sets read over a scoped variable are specialized to its grammar
(0058 §2). Test: `kind_sets_over_a_guest_are_its_grammars`.

## O99. Guests and hiding at scale

Hiding (decision 0058) is a range test on columns the join already reads. Its cost, and the cost of the guests
and values a policy makes, are not measured on the 9,010-file repository. Global embeds and the matcher path's
guests are still not these documents (O85).

**Closure:** a full-corpus run with guests on and off, in CPU time, with the guests made and the distinct texts
parsed.


## O100. Normalized strings as native operations, then as transducers

Decision 0058 §3's normalizer (`facts::vtab::syntax::normalized`) is a hand-written host scanner. Only the guest
maker calls it. A rule reaches its text only through `str(n, v)`, and its source map not at all. Typing needs both:
a quoted annotation's text, a decoded key, or SQL text whose nodes need places in the file. Each must run per chunk
in native code, never in SQL and never per node per byte (see "String matching is chunked").

Two steps:

1. **A native operation.** A normalized string is an operation the engine runs natively, like `@parse`. It is a
   relation of (literal, text, map) made per chunk, read by `str`, by guests and by rules. The language's decoder
   and the map stay one, so `str` and a guest can never parse different texts.
2. **A transducer.** Once the parser-generators work lands, the escape scanner and decoder become a side program
   of the transducer `Program`, as `value_programs` are (`docs/parser-generators/12` §3). They then run on the
   transducer's CPU and GPU paths, over the literal's bytes on the tape, and write the text and map as tape rows.
   They are not a separate module. Each language's escapes are the program's tables, not code.

**Closure:** rules read a literal's normalized text and map through a native operation, with a test against
`str`. After parser-generators: the operation is a transducer side program, with the hand scanner's tests passing
against it.

## O101. A demand-lowered `.rr` program does not print back as itself

`rr::parse` lowers `parsed(s, g, r)` (and `@parse`, `@splice`) into a demand: `rr_embed_req` and its context
relations `rr_ctx<k>`. `print` writes those rules out, and the `parsed` atom stays in the rule's body. Read back,
the printed program is lowered again, and gets a second context (`rr_ctx2`) and demand rule. The `.rdl` reader
lowers no demand. `rdl_programs_translate_to_the_same_program` therefore skips `typing/programs/calls.rdl`, which
reads `parsed` since its quoted annotations (C1). A run whose program text is printed, then read again, may do
the extra demand's work twice.

**Closure:** lowering a demand is idempotent: a printed program carries its demands as already made, and reading
it back adds none. The skip is then removed. Or `.rdl` programs are removed (step 31), and the round-trip test
stays for `.rr`.

**Closed (2026-11-07).** The reader notes which demand relations a text declares itself (`rr_embed_req`,
`rr_parse_req`, `rr_splice_req`, `rr_dir_req`, `rr_file_req`). A printed program declares the demands it was lowered
with, so reading it back makes none of them again. A constant world request (`file_line("setup.cfg", …)`) is lowered
as a fact, the form a bodiless clause reads back as. The `.rdl` comparison drops the demands the `.rdl` reader does
not make (`without_requests`, which now covers the value demands too), and the skip of `calls.rdl` is removed.
Test: `a_program_with_demands_prints_back_as_itself`.

## O102. `product_programs_derive_t_outputs` reads a table its fixture lacks

Since 50a57d0e7, `calls.rr` reads `host_node`, and the test stops with `no such table: f.host_node`. The fixture's
facts database is built without the guest relations. Not caused by the quality branch; seen on 2026-11-07.

**Closure:** the fixture holds every relation the product programs read, and the test passes.

**Closed (2026-11-07).** The test adds the guests and, since Typing's B6 and B10, `name`, `str`, `value` and
`op_at` (which `calls.rr` reads through `python/conditions.rr`), empty, as it adds `strprefix`. T's facts predate
them; with them empty no `sys.version_info` or `sys.platform` test is decided and every branch is taken, as T
assumed. The test passes (merge of Typing-support-final-shape, 56a615905).

## O103. AST pattern literals (`n ~ `pattern``) have no executor

`Lit::Pattern` parses, and `Literal::Ast` exists in the logic, but nothing lowers it and the plan has no step for it
("no executor … yet"). versioned-table-names is the pack's one rule that needs it. The matcher should run once per
pattern, over the chunk, and give an input relation of its matches and captures, which the rule then joins.

**Closure:** a pattern literal plans and runs, and versioned-table-names runs in the pack.

Closed (2026-11-07): ``n ~ `pattern` `` lowers to `ast_match(p, n, g, c)` atoms (capture "" is n; each `$X` binds
X; a negated pattern binds nothing). The batch compiles each pattern once per grammar with ast-grep, matches it over
each document, and maps matches to projected nodes by kind and byte range (`core_rr::CoreRows::patterns`). Test:
`rr::tests::pattern_literals_match_and_bind_their_metavariables`. versioned-table-names gives 0 findings on the
1,000-file slice; the Python linter reports the full tree clean. `$$$X` captures nothing yet.

## O104. Two guards keep a run's store objects from collection; one should go

O95's fix pins what a run holds. The run's plan is pinned (`hold_corpus_plan`), and so is each source and tape it
stores or finds (`hold_sources`), under a lease that expires. Rerail-rule-expressivity's fix adds a second guard, a
shared file lock (`<db>.collection.lock`). Runs hold the lock while they read, and a collector takes it exclusively
with `try_collect`, deferring if it cannot. Both guards are in place since the merge (31d3d15dc).

The lock is coarse. While any run holds it, no collection happens at all, so a store in constant use (parallel
runs, a long watch session) may never collect, and disk grows past the 10 GB budget. It also says nothing about
which objects a run needs. The pins are exact, and they survive a crashed run until their lease expires. But they
cost a row per held object, written in the transaction that stores the object.

**Closure:** one guard is chosen in a decision citing O95's races
(`a_source_collected_before_its_tape_is_stored_again_with_it`, `parallel_writers_and_a_collector_share_one_store`,
`a_held_plan_stays_live_until_its_run_releases_it`, `a_held_source_survives_collection_until_released`). The other
is removed, and those tests still pass. If the lock stays, collection under constant use is shown to make progress.

## O105. A `.rr` file is a corpus document only in a run whose program names `rr`

Decision 0061 §6 says a `.rr` file in a corpus run is a document like any other, so a run over a repository checks
its rules with its other files. A corpus run reads one language, though (`corpus_language`): Python or Rust, by
`Typing::rust` or by which has more files. The build adds `rr` to that choice. A program with `.language rr` reads
the repository's `.rr` files (`packs/rr/rr-input-outside-repository.rr`), and every other run reads none.

**Closure:** a corpus run reads every registered language its policies name, each document under its own dialect.
A run with a Python policy and an `rr` policy reports both from one store, and
`a_rule_about_rules_runs_over_rule_files` holds with a Python file's finding beside the `.rr` ones.

**Closed (2026-11-07).** A program's `.language` now chooses the files it reads (`python`, `rust` or `rr`), and
`tools::run_policies` (`rerail-engine run-policies <root> <out> <policy.rr>...`) runs several policies over one
repository, each over its own language's files with one content store, and merges their diagnostics.
`python_and_rr_policies_run_over_one_repository` runs a Python policy and RR101 over one tree and reports both. One
run still reads one language. A program naming two would be a run with documents of mixed dialects, which nothing
needs yet.

## O106. A relation named like a SQL keyword breaks the SQL backend

A program with `.decl values(…)` fails its run with `near "values": syntax error` on `DROP VIEW IF EXISTS
temp.values`. The relation's name is put into SQL unquoted. Found while writing `rule_files_have_text_facts`, which
names its relation `vals` instead.

**Closure:** every relation name the SQL backend writes is quoted, and a program with relations named `values`,
`select` and `order` runs.

**Closed (2026-11-07).** `logic::sql::ident` quotes a relation's name when it is a SQLite keyword or not a plain
identifier, and `sql::table` does the same for each part of a qualified name (`x.values`). Every statement that
names a relation goes through one of them: the plan's views, tables and indexes, the corpus's exports, and setfix's
reads and writes. `relations_named_like_sql_keywords_run` runs `values`, `order`, `select` and `group`, and
`rule_files_have_text_facts` names its relation `values` again.

## O107. The rr parser still builds statements beside its tree

Since decision 0062, lowering reads a rule file's statements off its tape (`rr::from_tape`). The parser still builds
its own statements as it builds the tree, because its control flow reads them. Whether a term becomes an atom
depends on it being a `call`, and a negated comparison is refused. Nothing outside the parity test
`statements_from_the_tape_are_the_parsers` uses them, and they cost a second copy of every term during a parse.

**Closure:** the parser builds only the tree, and decides from node kinds what it decided from statements. The
parity test becomes a comparison of the tape-read statements with a recorded set, and the replay (3988 303 298 0)
and the rr tests are unchanged.

## O108. A corpus run takes two collection leases for one problem

Rerail-rule-expressivity and Typing-support-final-shape each fixed collection deleting a running corpus run's sources
(O95), in different ways. The final shape now has both:
- `ContentStore::hold_collection` (`CollectionLease`) with `HeldPlan` (`hold_corpus_plan`, `hold_sources`);
- `ContentStore::begin_corpus_run` (`CorpusRunLease`, with the store's busy retries).

A run takes both and releases both once its manifest is recorded, before its collection starts (`record_run`). One of
them is enough.

**Closure:** one lease, the other's tests kept against it.

**Closed (2026-11-07):** the shared file lock (`hold_collection`) is the run's one hold. Every collector takes it
exclusively first (`try_collect`: `collect`, `keep_corpus_runs`, `collect_corpus_pass`), and a process that dies
releases it with its descriptors. `begin_corpus_run`, its pin kind and the check in `collect` are gone: the pin row
cost a write transaction per run and, when a run died, held all generation-based collection off for an hour. The
row lease had no tests of its own. `readers_publish_memos_and_roots_before_collection_resumes` checks what both
promised: a reader's unrooted source, tape and memo rows outlive collection until its manifest names them. Removing
the row lease showed that the stored prepared program (decisions 0073 to 0075) had no root and was collected
between runs: the run's manifest now names its sources.

## O109. A run read from memoised base rows derives other rows

Decision 0063 memoises the base rows a file hands the cross-file part (`memo::Entry::base`), each row its file's by
its first column, and a memo hit replays them. Merged with typing's `calls.rr`, a run read entirely from the memo
gave 233 `call_open` rows where the evaluated run and T give 241: eight calls of `glob.py` missing
(`own_ingestion_derives_what_t_facts_do`, the 4096-node run after the 262144-node one). Every file was a hit and no
chunk was refused. With `RERAIL_CORPUS_MEMO=off` the rows agree. The cause is not found: the replayed base rows, or
the statistics the cross-file part plans with (measured once over every file's rows in a memoised run), are the
candidates. Crediting a guest's rows to its host file made no difference.

Until then a run whose cross-file part reads base relations is not memoised, as before 0063: `memoised &&
stored.is_empty()`. The entry's `base` field stays. `a_session_reruns_what_an_edit_changed` (0063's test, whose rule
reads base relations) is ignored with this reason, so a session reruns every file of such a rule.

**Closure:** a memoised run with base rows gives the evaluated run's rows in that test, the hold-back is removed and
the session test runs again.

**Closed (2026-11-08):** with the hold-back lifted, the 4096-node run, read whole from the memo, gives 241 `call_open`
rows and 256 `call_target`, as T and the evaluated run do. Every output file is equal. The guest-hiding fix (O110)
was in when they first agreed, and it is the likely cause: hiding read `tbv`'s `live_asg` from its annotation copy
only. That was not proven by reverting it. One gap remained: a run read whole from the memo wrote no
`rr_embed_req.csv`, a file-local output no file made, which the evaluated run writes empty. Every output now has its
file. The hold-back is removed, and `a_session_reruns_what_an_edit_changed` runs again.

## O110. Four builtin receivers are Unknown after the Typing-support merge

After Typing-support-final-shape (84cba7e50) merged into the final shape, `python_type_relations_agree_with_the_runtime`
finds four receivers of `fixtures/python/calls/project/pkg/typed.py` Unknown: `text = str(12)`, `data = b"ab"`,
`words = [...]` and `table = {...}` (40 exact of 44). It fails with `RERAIL_CORPUS_MEMO=off` too, so the memo is not
the cause. The merged `calls.rr` is that branch's, unchanged. The rr tape reader gives the parser's statements for it
(`statements_from_the_tape_are_the_parsers` now covers the typing and golden programs). The run reads no typeshed:
the world round loads no documents, and `litq`'s classes come from `qv("builtins.…", "cls", c)`. Whether the test
passes on that branch alone was not checked: building a second target would exceed the disk budget. The test is
ignored with this reason.

**Closure:** the four receivers exact again, or listed as departures with the reason, and the test not ignored.

**Closed (2026-11-07):** 44 exact of 44, and the test runs again. The cause was the engine, not the merge: guest
hiding (decision 0058 §2) specialized `tbv`'s `live_asg` to its annotation copy because `tv`'s first column may hold
an annotation's node, and so dropped every host row of it. A variable now needs to be definitely a guest's node to be
specialized; a rule whose variable only may be one is split into the host's rule and a guest clone (0058, amended).

## O111. Python pack rules that fail to run, alone or combined

Found while measuring decision 0073 (2026-11-07). Run one at a time with `rerail-engine run-rr` over
`fixtures/python/calls/project`, a sample of the Python pack failed:

- **A message names no column:** `ruff-c404`, `ruff-c406` and `ruff-perf403` ("message: no column k: v"), and
  `ruff-fly002` ("FLY002_fstring message: no column a").
- ~~**No such table `returns`:** `ruff-pyi006`, `ruff-pyi011`, `ruff-pyi014` and `ruff-pyi046`.~~ 2026-11-08:
  `returns` is read back only when the cross-file part derives it (PYI006 runs; the others not rechecked).
- ~~**Not lowered yet:** a rule including `ruff-ruf001-ambiguous-unicode-character-string.confusables.rr`~~
  (2026-11-08: RUF001 to RUF003 lower and run, decision 0081).
- **Refused:** `ruff-b005`, whose value cycle is unbounded (see above).

Given as one directory to `rerail-engine logic`, the pack does not combine. Leaving out the rules above, and others
that fail combined (`ruff-c408`, `era001`, `fast003`, `int001`, `int002`, `ruf001` to `ruf003`, `ruf024`, `up030`,
`up032`), every bandit rule then failed with "relation kind is read by the rule for B101 and not declared". That
points to how a directory is combined, not to the bandit rules. Not investigated.

**Closure:** every pack rule runs alone, or is listed with its reason, and the pack combines into one program.

**Progress (2026-11-08).** `check-rr` over the 546 files: 539 pass the reader (526 before).

- **Messages:** `{{` and `}}` are braces of a message's text (`rr::lower::findings`), as the converter already
  wrote them (`convert::policy`). The 14 messages with literal braces now double them. In `fast003`,
  `{{{K}}}` is a brace around the parameter's name.
- **Value cycles:** `era001` runs. A recursion only between constant keys that never come back
  (`era_re("EXPR", …) :- era_re("OPD", …)`) is bounded (`cycles::constant_keyed`). So is a head term equal to a
  term a lower stratum's variable is assigned (`era_run`'s `@add($l, 1)` checked against `era_cline`).
- **Combining:** a directory's policies are read as `run-rr` reads one (`tools::program_text`), so their language's
  base relations are declared. The "relation kind … not declared" failure is gone. 60, 120 and 240 rules
  combine and run.

What remains:

- **Listed with their reason:**
  - `ruff-b005`: its index rises until `@slice` gives `""`, which no static bound sees. Rewritten with
    `$j < strlen($v)` it would be a capped count.
  - `ruff-fast003`, `ruff-perf403` and `ruff-ruf001` to `ruf003` (with ruf001's `confusables.rr`): `match_all`
    over a node, O80.
- **The full pack (539 rules) does not prepare within 2 minutes.** Preparing was superlinear. Three hot spots are
  fixed:
  - `lower` rebuilt every column's domain map per rule (52.8 s to 0.13 s at 240 rules);
  - `strings::vocabulary_columns` scanned every rule per declaration;
  - guest hiding's `carry` re-analysed every rule each round, now a worklist.

  At 240 rules the run takes 23.6 s user, against 91.4 s before. At 539, `hiding::hide` is still about 58 s of the
  68 s that combining takes (sampled): its specialization loop runs `carry` from scratch each round and clones
  every rule. Peak RSS is 2.0 GB at 240 rules, over the 1 GB budget.

**Progress (2026-11-08, decision 0084's dump).** The 58 s was not the loop's size. One program,
`ruff-perf401`, never ended: a copy whose head holds a constant at its seeded column (`unset`, -1) was found again
each round and counted as a change, 8,532 rounds in 40 s at a steady 1,311 rules. Fixed in `hiding::hide`
(`a_copy_with_a_constant_at_its_seeded_column_ends`); perf401 alone now takes 0.39 s. `check-rr` now also types
the pruned program (`types-failed`), as a run does when it plans. Six rules still fail, alone as in the pack:

- **Refused at reading:** `ruff-b005` (an unbounded value cycle) and `ruff-perf403` (`match_all` over any
  expression, O122).
- **Types:** `ruff-pyi062` (`lit_key`: `$t is a Name and a Kind`) and `ruff-ruf028` (`stmt_level`: `$k is a
  Kind and a Name`).
- **A span where text is compared (O120):** `ruff-pgh004` (`noqa_all_tail`: `R = ""` over a capture of a comment)
  and `ruff-up009` (a fact of `utf8`, `"utf-8"` where a span goes).

Without those six, the pack (538 programs) combines in 29 s and then stops in the file-local split (O135).

**Closure (narrowed):** the six rules run, or are listed with their reason; the combined pack's cost is O135's.

**More failures (2026-11-08, over the Ruff fixtures).** The calls project does not exercise these four, so they
were not seen before. Each fails alone, and also with libraries pasted in (`RERAIL_LIBRARIES=copied`), so 0089
did not cause them:

- `ruff-pyi034`: unplaced literals, `$q = @cat(…)`.
- `ruff-pyi036`: `$form` unbound.
- `ruff-sim222` and `ruff-sim223`: unplaced literals, `$rr_t135 = @add($k, 1)` in `rr_ctx134`.
- `slot001`, combined with the programs it shares relations with, fails the same way as sim222 in both modes. It
  is probably the same cause.

## O112. The shared tape cache holds memo rows whose length does not match (closed 2026-11-07)

Found while testing decision 0073 (2026-11-07). `python_split_lines_are_lossless_and_document_local` failed with
"corrupt content store: facts memo length does not match metadata". The default tape cache
(`$TMPDIR/rerail-tape-cache.sqlite`, which every build and session on the machine shares) held 18 of 4,867
`facts_results` rows whose zstd frame decodes to one byte fewer than `raw_length`. All are per-file memo rows (fact
schemas 3 and 4) of a `b.py`. No prepared-program entry (schema 1) is among them. In one writer, `raw_length` and the
frame come from the same entry (`put_facts_memo`), so another writer of the shared file is the likely source. That
is not confirmed.

`facts_memo` decodes every content-pure row of a source and schema, so one bad row fails every lookup on that source,
and the run fails instead of missing. The file was not moved (DB files are not moved). Tests for 0073 ran against a
fresh store (`RERAIL_TAPE_CACHE`).

**Closure:** the writer found, or a row that does not decode treated as a miss (and rewritten), with a test.

*Likely writer (2026-11-07, investigate-bend-vs-datalog):* an early version of decision 0076's corpus test
(`cross_file_rows_are_memoised_per_import_component`) ran an `UPDATE` on `facts_results` for every row with
`path = 'b.py'` in the shared store, whatever its plan (17 rows then), to plant a bad row. The test now changes only
its own plan's newest row and restores it.

**Closed (2026-11-07):** a row that does not decode is a miss. `facts_memo` skips a result row whose frame does
not decode to its length, and a completion row whose counts do not, so only that row's plans are evaluated again.
`put_facts_memo` overwrites a result row, and a completion row it cannot merge into is replaced instead of failing
the write (`a_memo_row_that_does_not_decode_is_a_miss_and_is_replaced`). The writer of the bad rows was not found.

## O113. `calls.rr` with `.pragma "cross-memo"` fails in world's final round: no statistics for `docof` (closed 2026-11-07)

Found trying decision 0076 on the typing programs (2026-11-07). With the pragma appended (even naming no relation
`calls.rr` derives), `python_calls_program_agrees_with_the_runtime` fails in the final round with "no statistics for
docof.n" (`plan.rs`, an input a rule reads that is not a batch source and was not measured).

- Fixed on the way: `cross_memo::add` merged everything `components.rr` brought with `core.rr`. `calls.rr` declares
  its own base relations (`docof(n: node, d)`, not `core.rr`'s `loc`), and the merged rules and inputs failed with
  "table docof may not be dropped". `add` now takes only the `rr_` rules and what they read that the program lacks.
- Not investigated: which part reads `docof` unmeasured. The likely one is the component sub-program, if a
  `components.rr` rule that reads `docof` lands in the cross-file part (`rr_cx_imports`' recursive package rule has
  no node, so the split may not place it per file). The toy corpus test reads `core.rr`'s `docof` and passes.
- Also seen: the 84-file Jedi benchmark (`dl-small` with typeshed) ran past 10 minutes of wall time at load 50–120
  without finishing its final round, so the time it would take is unknown.

*Closed:* `add` gives inputs and facts only to relations the program does not declare (calls.rr's `docof` over
nodes is a rule over `node` after lowering; re-adding `core.rr`'s input left it unmeasured), and a pragma naming no
derived relation binds no `rr_cross_doc`. The profile names the relations the static contract allows: 122 of
calls.rr's 234 cross-file relations, once run constants (`py_version`) and projections that keep a node (`kind`,
`docof`) are read freely. Kept out: `isqmod`, `qknown`, `qopen` (sets over every document, keyed by name), `tsub`,
`tsup_open` and `need_name` (demand).

## O114. The cross memo is correct on calls.rr but does not pay: what typing's rows depend on is global

Measured 2026-11-07 (decision 0076 §7). Over the 84-file Jedi corpus, replaying 133 components (1.55 M rows) makes
the cross part 3.6 s slower than deriving it (19.8 s against 16.2 s), and peak RSS reaches 1,038 MB.
`RERAIL_CROSS_MEMO=verify` passes. The declarable relations (122 of 234) cost about 3.6 s to derive. The expensive
fixpoint reads lookups over every document by name (`isqmod`, `qknown`, `qopen`, `tsub`, `tsup_open`, the demand
`need_name`), so it cannot be declared.

Two steps would change that, in order:
1. **The rules.** Make those lookups closure-local: ask "is q a module" of the modules the closure imports
   (`rr_cx_imports` joined with `rr_cx_module`), not of every document; derive `tsub` per class from its own bases
   (0065 §2 already moves `sub` to the workspace side). These are calls.rr's, the Typing branch's to change.
2. **The engine.** Replay in place, not by copy: a memoised relation's stored rows are read where rules probe them
   (0065 §2, "read by index as rules probe them"), and a guarded rule is cut to the missing components' documents
   before its joins, not filtered after them.

**Closure:** with both, a warm replaying run's cross part costs less than a derived one on this corpus, under 1 GB.

**2026-11-08: met, with `calls_cx.rr` (decision 0076 §8).**
- Step 1 is done in a parallel program, which `make_cx.py` writes from `calls.rr`. 280 of 283 cross-file relations
  are declarable.
- Verify over an edited corpus found four more kinds of importer-shaped rows, and they are fixed: generics, star
  imports, instance attributes, and the benchmark's spans. After the fixes, all 82 found components matched.
- Step 2 is done in part. The guard has a native step, and a run with every component found runs only the copies.
- A replay's cross part is 7.8 s against 35.7 s for `calls.rr` derived, at 837 MB, with Jedi agreement 6,299 in
  every run.

Still open:
- ~~Reading stored rows in place~~: done (0076 §9). The rows are in `cross_rows`, read by one query of key ranges, and only the
  relations still read are loaded. A warm run's cross part is 2.3 s and the whole run is 13.2 s.
- ~~Cutting a guarded rule before its joins~~: done in part (0076 §9). Each same-document node is guarded. The
  edited corpus still costs more with the memo than without it (42.2 s against 29.9 s): the 41 components that
  miss hold most of the work, and storing them costs 6.9 s. The next step is early cutoff, a component key made from
  what the cross part reads of each file, not from its bytes.
- Whether `calls.rr` itself becomes `calls_cx.rr`. That is Typing's to decide: the closed world shrinks to the
  reader's closure, and the agreement is the same on both oracles.
- Finding importer-shaped rows statically, so the check is not only `verify` over an edited corpus.


## O115. A prepared program read back from the store fails its run: "no such table: pre_85"

Found merging final-rerail-shape into investigate-bend-vs-datalog (2026-11-08). On a fresh store,
`prepared::tests::a_prepared_program_reads_back_from_its_facts_and_its_tape` passes once and fails the second time,
when the program is read back from the store (decision 0074, rules as facts), with "no such table: pre_85" (a demand
split's relation, `demand.rs`):

```text
RERAIL_TAPE_CACHE=/tmp/fresh.sqlite <test binary> --exact …a_prepared_program_reads_back_from_its_facts_and_its_tape   # ok
RERAIL_TAPE_CACHE=/tmp/fresh.sqlite <test binary> --exact …a_prepared_program_reads_back_from_its_facts_and_its_tape   # fails
```

The test's round trip (front program, lowered program, file-local split) compares equal, so what differs is
outside what it compares. In parallel test runs the in-memory cache (16 programs) is evicted, runs read their
program back, and other corpus tests fail the same way (`cross_file_rows_are_memoised_per_import_component`,
`a_corpus_runs_memoised_rows_bind_by_name_from_their_frames`); with `--test-threads=1` they pass. Not investigated
further: the code is final-shape's (0073, 0074).

**Closure:** a program read back from the store runs as the one prepared, with a test that runs twice on one store.
## O116. lazy-architecture's speed and memory

**What was wrong with the shape.** The rule was 52 ms a file on 1,000 files (52.5 s CPU) and did not finish the
full tree in 600 s. The cost was not one slow join. Six things together made the engine do work the Python script
never does:

1. *Most of the program ran cross-file.* 438 of 700 rules ran once over all files, holding 621 MB of base
   relations on 1,000 files. The causes were equal variables that carried no location, constant tables, joins
   through symbols only, and string "ask" relations. Fixed: equalities carry locations, constant relations are
   derived in both parts.
2. *Every string annotation was parsed as a guest*, 36k on 1,000 files, though no rule the outputs need reads
   one. Fixed: guests are made only when a needed rule reads them.
3. *Single literals fell back to SQLite.* Concatenation, constant sets and split heads had no native step.
   Fixed (`e6c4fcd04`).
4. *Every file was read.* The script reads its scope, about a third of the tree. Fixed: the run reads the
   documents the policies' `.scope` selects (O117).
5. *Two plans scanned what they should look up.* LA_018 tested each source line against 55 surface names, one
   at a time. Fixed: a test of a constant table's values first tests the whole table as one set
   (`strings::implied_set_tests`). `la_scan` read 28M containment pairs to find each function's 467k nodes,
   because `within`'s fanout by ancestor was weighted by subtree size (about 1,000). Fixed: priced by mean
   subtree size, as `nearest`'s descent is.
6. *The cross part held whole-corpus node attributes* (`line`, `docof`, `visit_index`, `node`: 673 MB on the
   full tree) for about ten thousand lookups. A file-local relation with a row per node (`la_leaf_target`) was
   exported whole because the one rule that read it also read a cross-file relation. Fixed: a join of
   file-local relations alone is pushed into the chunks, and a chunk keeps base attribute rows only for the
   nodes its exports hold (`file_local::semijoin_bases`).

One rule bug showed up on the full tree: `frozenset(x for x in y)` read as `frozenset()` gave 11 LA-017
findings. Fixed in `la_arg`.

**Now.** Full data-layer tree (3,117 files in scope of 9,024): 0 findings, as the script. The run took 43 s CPU
and 881G instructions, against 151 s CPU and over 2,770G instructions before. The engine's peak footprint is
829 MB, against 3.6 GB before; `time -l` reports 1.36 GB maximum RSS. CPU figures were taken at load average 45.
The script takes 5.4 s CPU on the same tree. The engine is still about 8x slower: 34 s of the 43 s is the
file-local part, spread over strata of a few hundred ms each (`la_ask_*` callee resolution, `la_vis`,
`la_base_name`, `ast_parent`).

The whole pack on s200 (65 rules, 172 files in scope) runs in 34.8 s CPU at a 1,257 MB peak footprint, over
the 1 GB budget. Three rules had passed 3 GB on 50 files (match contexts that were cross products) and one failed
to run (an empty demand split); both are fixed (`77762bf31`).

**Closure:** the full tree runs within the 1 GB budget by the OS's measure, at no more than a few times the
script's CPU, with its findings equal to the script's; the s200 pack runs within 1 GB.

**Pack memory, after the merge (2026-11-08).** The merge took the s200 pack from 1,257 MB to 2.6–3.0 GB. The cause
was not the memo: the prepared 66-rule program is 775k fact rows (32 MB of `.rr`), stored as one document and read
back off one tape for its check (decision 0074), and that parse peaked at about 1.5 GB in the stats phase. A
stored program is now documents of at most 20,000 rows, each its own tape (`9298b00aa`): 1.56 GB peak, the same 61
findings, and a warm run reads the parts back. Then (`42c30725a`):

- *Semi-joins per relation.* `semijoin_bases` was all-or-nothing, so one cross-file walk in the pack held every
  attribute whole. It is now a taint fixpoint per attribute. It also found that `tape(n, par, …)` is no attribute:
  `par` is a parent node typed as a number, and the `rr_up` walks climb it.
- *Options are constants.* `option`/`option_item` rows are the same in every chunk, so a relation built from them is
  file-local (`bootstrap-engine-invariants`: 143 to 100 cross-file rules); the memo key folds in the options.
- *Chunks sized to the run's budget.* A chunk adds at most three quarters of what is left of 1 GB
  (`RERAIL_RUN_MB`), not a fixed 384 MB.

Now: 61 findings, 861G instructions, 1,530 MB peak (`time -l`: 1.71 GB), load average 65–106. Where it goes, measured
by live heap at each chunk: 374 MB before the first chunk (the prepared program, its plans and automata), then about
50 MB more with each chunk, kept for the cross part. At the end that is 15.2M base values (`node`, `docof`, `line`,
`text`, `child`, `field`, about 120 MB raw) and 1.5M per-value statistics (`Totals.vals`, about 60 MB). A chunk adds
200–490 MB at its peak (4.4–7.7 KB a node). The base rows stay whole because cross-file rules reach nodes no export
holds: `authentication-plan-purity` walks `field(x, f, v)` from a cross-file relation's nodes (so `node` and `text`
stay whole), the pack's eleventh policy walks `child` (`docof`, `visit_index`), and a diagnostic decodes a tainted
finding (`line`). Before options were constants, `bootstrap-engine-invariants` also enumerated every node of its
documents cross-file (`bei_node(n, d) :- docof(n, d), bei_doc(d)`); it no longer does.

**Next:** hold those base rows on disk (`x`, as the exports are), or keep, for a walk from held nodes, the rows of
their children too; and bound the per-value statistics (a sketch, not a map).

## O117. `.scope` forms the run does not read (closed 2026-11-08)

The run now reads only the documents the policies' `.scope` globs (`dirs`, `include`, `exclude`, `tests false`)
select, as the scripts' discovery does. On 1,000 files that is 348. Still not read: `report dirs`, `text`,
`embedded`, and `option("…")` entries, whose values come from policy options. In a pack that combines policies,
documents are the union of their scopes. Each policy's facts are not yet limited to its own scope.

**Closure:** each `.scope` form is applied or refused by name, and in a combined pack each policy reads only its
scope's facts.

**The forms, done (2026-11-08, `58ae86df3`).**

- `text "…"`: a file with no grammar of the run that matches a text glob, and that a scope selects, is read as
  text only (`core::text_only`): `doc`, `source` and `src_line`, no nodes. Before this the corpus walked only
  `.py`, `.rs` and `.rr` files, so `bounded-composed-graph-paths`' BCGP011 documentation check never saw its
  `.rst` files. On the full tree, 1,070 documents are read as text.
- `dirs | include | exclude option("…")`: kept by name (`front::OPTION_GLOB`) until the run's options are known,
  then resolved both for the documents read and for the findings filter (through `option_item`). An option the
  run does not set adds nothing. `matching-architecture`'s `--application-root` now reads its roots.
- `report dirs "…"`: findings outside these directories are dropped; facts still come from the whole scope.
- `embedded false | true`: `.guests off | on`.

Test: `scope_takes_option_dirs_text_documents_and_report_dirs`.

Reading the text documents also exposed a rule cost: `bounded-composed-graph-paths` counted the lines between
every decorator and every later def. That passed 3 GB on a quarter of the full tree, with or without the text
documents. Its line rules now read Python files only, and a decorator's run of `@` lines is walked. Full tree:
0 findings, as the script; 10 s CPU, 535 MB footprint (`time -l`: 1.1 GB). The script takes 0.08 s.

**Each policy's own facts: measured, not yet guaranteed by structure.** On s200, the 66-rule pack's findings are
exactly the union of each rule's run alone (61 and 61, none on either side only). A policy's findings outside
its scope are dropped (`rr_scope_drop`). Its file-local relations, though, are made over every document the pack
reads, so a cross-file rule can join rows from documents outside its own scope. Filtering each policy's exports
by its scope would make this exact. One case blocks the simple version of that filter: `merge_identical` merges
equal relations across policies, and a merged relation has two owners with different scopes. So the merge would
have to keep policies with different scopes apart. A doc-level guard (each policy's `doc` atoms read a
`doc` restricted to its scope) is cheap, but does not cover node-level rows.

**Closure (what remains):** a policy's exports, and its reads of `doc`, are restricted to its scope, with
`merge_identical` keeping policies of different scopes apart; the pack's findings still equal the union of the
rules' runs alone.

**Each policy's own facts, by structure (2026-11-08, decision 0091, `096d34181`, `cd59cb78b` and the commit after
them).** Library relations are canonical (one copy for the run, made over every document read; with the libraries held once (0089) the library is its own part), and each
policy's rules read base and shared rows only of its scope's documents, through `!rr_scope_not(@doc(n))` guards.
`.global` shares a policy's own relation when every copy is alike. The structure is tested
(`library_relations_are_shared_and_each_policy_reads_its_own_scope`, which fails without the guards). On s200 the
pack's findings are still the union of the rules' runs alone (61). Cost: +6.6% instructions (893.0G against
837.9G without guards), peak 1,319 against 1,350 MB. The 6.6% is the planning of about 800 scope strata in every
chunk; giving `rr_scope_not` as base rows of the batch would remove it. O117 is closed.

## O118. A grep reader for files that are not documents

t11 specifies `.input r(IO=grep, glob=…, re=…)`: one walk and one multi-pattern scan per file for all grep inputs,
rows memoised by content. It is for lock files, CI YAML and `.md` tables. It does not replace
plan-composition-architecture's runtime probes, which hash objects the builder makes as it runs.

**Closure:** decided (t11 §10), then built or dropped.

## O119. test-budget from a static model of pytest's collection

t13 counts tests from the trees: files, `Test*` classes and their bases across files, parametrize lists
(constants resolved across files), fixtures with `params=`. A prototype gives 13,610 against pytest's 14,449 on
the 9,010-file tree (−5.8%), and −5.9% on main. Missing in the engine: a `product` aggregate, a TOML reader for
pytest's settings, and findings in one scope with names resolved across the whole workspace.

**Closure:** the model is written in `.rr`, checked against the per-file counts, and test-budget reports it.

## O120. A regex group over a span binds a span where rules compare text (closed 2026-11-08)

Since `5e026dcfa` (merged from the final-shape branch), `match("…(?<x>…)", sp)` over a span `sp` binds `x` to the
group's span (`rx_span`'s `m`), not its text. `bounded-composed-graph-paths` compares such an `x` with strings
(`bcgp_block(p, "PathPattern", h, _)`), so its program fails to lower: `the string "PathPattern" stands where an
integer goes`. The rule is skipped in the s200 pack run.

**Closure:** a group bound over a span reads as its text where the rule uses it as text, or the rule says so, and
the pack runs the rule again.

**Closed.** The engine is right: `core.rr` says a capture over a node or span is a span. The rule was wrong.
Each capture in `bounded-composed-graph-paths.rr` now names a span (`(?<xs>…)`) and reads its text with
`text(xs, x)`. On s50 the rule alone gives 26 findings at 77 MB, most of them BCGP001 (a required file is not in
the 50-file subset). Back in the s200 pack: 66 rules, 61 findings, 52.7 s CPU (load average 36–90), 2,652 MB
peak, with no lowering error.

## O121. A fact or empty-body rule whose node column is `unset` has no row

`r(-1).`, and a rule whose body is only `unset(e)`, lower to a row with the node value -1 (`rr::lower::unset`). In
a corpus run of a file-local relation that row is lost. `ruff-fast003` reads "a parameter with no default" this
way (`fast_param`), so on Ruff's `FAST003.py` it finds all 31 expected findings and 33 false ones (decision 0081).
One attempted fix, keeping the row in the file-local split, did not change the result and was reverted.

**Closure:** the row is kept, a test runs `r(-1).` through a corpus run, and FAST003 has no extra findings.


## O122. `match_all` over nodes: not chunked, and refused over any expression

Decision 0081 scans each distinct text of the nodes a pattern runs over, once per pattern. That is a scan per
container, not the chunked pass over the file that string matching should be: nested kinds scan a byte more than
once, and no measurement on the 9,010-file repository exists yet. A column bound by any expression (`perf403`:
`store(t, e)`, `e` any node) has no kind, so the lowering still refuses it.

**Closure:** containers are matched in one chunked pass over the file's bytes, with matches kept per container,
measured at full size; and a column of any expression is scanned, or the rule binds its kinds.


## O125. Delete-and-rederive on the points-to fixpoint (decision 0083, spike 1)

0083 §2 maintains the stored fixpoint by deltas. Delete-and-rederive can over-delete in a large recursive
component, and `calls.rr`'s `pv`/`bv` stratum is one: 51 relations, most of the cross-file cost.

**Method:** apply real edits (the last 20 commits touching the 84 files, and the `exceptions.py` comment) as
deltas to today's fixpoint, through More-ergonomic-rule-syntax's `maintain.rs`.

**Closure:** measured rows over-deleted, rows re-derived and CPU against a cold run, results equal to a full run.
A one-function edit costs under 5% of a cold run, or the fallback (support counts per row, or maintenance per
component of definitions) is measured and chosen.

**Measured, in part** (2026-11-08). The spike is the ignored test `spike_o125_session_maintains_calls`, run over the
84 files without typeshed. It runs `calls.rr` in a session, makes one edit at a time, and compares each run with a
full run. The edits are three hand-made ones (a comment, a function added at the end of a module, a `return` changed),
not yet the last 20 commits.

- **The results are right.** Every edit's outputs equal a full run's (`call_target`, `call_open`, `type_of`,
  `possible_type`).
- **Most strata are maintained.** 180–200 of 218 strata are maintained from deltas, 19–21 of them recursive. Three are
  derived again because they hold an aggregate.
- **The deltas are tiny.** An edit over-deletes 3–88 rows and inserts 3–88, out of 1.14 M rows the maintained
  relations hold.
- **The cost is not.** It follows the fixpoint's size, not the delta's (CPU on the main thread; machine load 6–15):

  | Edit | Strata run | Session: cross part | Full run: cross part |
  |---|---|---|---|
  | comment in `exceptions.py` | 183 of 218 | 11.4 s | 4.5 s |
  | function added | 203 of 218 | 17.7 s | 4.2 s |
  | `return` changed | 203 of 218 | 16.4 s | 4.5 s |

  The file-local part does shrink (7.5–8.0 s to 1.0–1.7 s: one file evaluated), but the cross part costs 2.5–4x a
  cold derivation.
- **Where the time goes.** About 7.7 s is the strata themselves. The points-to component alone goes from 1.7 s cold
  to 3.6 s maintained (its insert rules plus its over-delete rules: 1,697 and 1,147 rules against 464). Over 4 s is
  outside any stratum, and each item is linear in the stored fixpoint:
  - the survivor rules (`h :- h__old, !h__del`) copy every held row;
  - every kept relation and input is bound as a source, measured and digested;
  - every derived relation's rows are copied out for the next run;
  - the rewritten strata (3–4x the rules) are planned again.
- **Why so many strata run.** A comment adds a tree-sitter node, so `node` changes and 183 strata read it. A
  function added changes 41 inputs.

**What it took to run at all** (in this branch, not More-ergonomic's):
- `maintain.rs` takes a head's computed term (`@dot`) by naming it with a variable the body binds, and copies
  `nearest` literals as it copies comparisons;
- the session keeps one cross part per program key, since a run's passes are different programs and took each
  other's place;
- the `r__old` sources carry their constants' counts. Without them a measured column prices every constant at no
  rows, and the over-delete plans began with a cross product of `kind__old(_, identifier)` (the run did not end in
  400 s);
- the demand split's nullary `pre_` relation gets the unit column every nullary relation has.

**Closed** (2026-11-08). Over the last 20 commits touching the files, and once with typeshed. The test is
`spike_o125_replay` (77 files at the base; each step applies one commit's files); the replay and the work split
are made by `scripts/profiling/o125/` (`replay.sh`, `work.py`, `defdelta.py`). CPU is the cross part on the main
thread; machine load 6–116, so the times are rough.

The commit replay, one commit per step:

| Step | Files, lines | Nodes renumbered (preorder / in changed definitions) | Input delta rows | Over-deleted / truly deleted | Session cross part | Full run cross part |
|---|---|---|---|---|---|---|
| 01 | 2, +176 −1 | 14,462 / 2,473 | – | – | timed out (118 s) | 4.5–7.4 s |
| 02 | 1, +5 −4 | 17,522 / 1,364 | 928,786 | 257,659 / 204,956 | 35.0 s | |
| 03 | 3, +75 −33 | 29,501 / 1,830 | 1,607,427 | 434,252 / 352,760 | 40.9 s | |
| 05 | 14, +479 −82 | 46,054 / 9,146 | 2,423,652 | | 48.2 s, 2 rows wrong | |
| 08 | 6, +493 −9 | 28,392 / 6,679 | 1,497,030 | | 42.1 s | |
| 12 | 9, +695 −360 | 35,124 / 13,162 | 1,813,844 | | 51.0 s | |
| 14 | 1, +2 −32 | 4,492 / 290 | 250,094 | | 38.1 s | |
| 17 | 2, +31 −9 | 425 / 394 | 17,784 | 4,141 / 3,225 | 32.4 s | |

The other twelve steps fall in the same band: 0.5–2.1 M input delta rows, 33–50 s.

- **Right, with one departure.** 18 of the 19 steps that finished equal a full run. Step 05 has 2 `call_open` rows
  wrong, and typeshed has 63 (O137). One wildcard bug was found and fixed: an insert rule dropped the negation it
  was derived from (`maintain.rs`, test `a_session_keeps_a_negation_that_still_fails`).
- **Over-deletion is mild.** Over-deleted rows are 1.23–1.35x the rows truly deleted. Support counts per row would
  save at most that, so they are not the fallback.
- **The input delta follows the ids, not the edit.** A 9-line commit (step 02) changes 929 k input rows, because
  every node after the edit is renumbered (`doc << 32 | preorder`). Step 17, which edits near the end of its files,
  changes 18 k.
- **Where the work goes** (native join work from `RERAIL_PROFILE`):

  | Run | Cold | Session | Deltas (anti-joins) | Survivor copy | Over-delete | Insert | Rederive |
  |---|---|---|---|---|---|---|---|
  | step 17 | 8.59 M | 8.90 M | 6.98 M | 1.43 M | 0.29 M | 0.11 M | 0.04 M |
  | step 02 | 7.55 M | 21.2 M | 6.69 M | 1.34 M | 3.84 M | 6.84 M | 2.46 M |
  | step 17, typeshed | 71.3 M | 111.8 M | 11.7 M | 3.3 M | 4.2 M | 11.2 M | 9.9 M |

  The typeshed session also runs 71.6 M of plain re-derivation (the strata it does not maintain, and the verify).
  On step 17 the maintenance proper (over-delete, insert, rederive) is 5% of cold. The rest is linear in the stored
  fixpoint: computing deltas as anti-joins of whole relations, and copying survivors. The CPU on top of native work
  is the overhead already named: planning 3–4x the rules, binding, measuring and copying.
- **Typeshed** (step 17). Cold: 85.5 s CPU, 51.2 s of it the cross part. 71 of 76 strata maintained, 8 recursive.
  An 18 k-row input delta over-deletes 93 k rows and inserts 93 k, against 3.25 M held: typeshed's fixpoint fans
  a delta out 5x more than the 84 files alone. The maintenance proper is 35% of cold. Peak RSS 1.64 GB, over the
  1 GB budget.
- **The cut alone saves nothing.** Maintenance per component, the cut that re-derives each changed component
  whole, runs 210–214 of 218 strata with preorder ids: it costs about what a full run costs (5.1–7.3 s).

**Chosen route.** Delete-and-rederive stays, with three changes, in this order:
1. per-definition node ids (0083 §3). On these commits they cut the rows renumbered 6–30x (step 02: 17,522 to
   1,364); they are what makes a delta follow the edit;
2. maintenance in place: deltas handed over by the store (the parser knows what it changed), not anti-joins of
   whole relations; no survivor copy; plans of the rewritten strata kept;
3. typeshed's fan-out measured again after 1 and 2. If an edit's maintenance still passes 5% of cold there, the
   next lever is a dependency cut between library and repository facts (library rows held fixed), not counting.

The 5% target is met on step 17 by the maintenance proper, and missed by everything around it. It is not met
with typeshed.

## O126. How big a base delta is per definition (decision 0083, spike 2)

0083 §2–3 make the definition the unit of change, with node ids `(definition, index)`.

**Method:** replay the data layer's history (a few hundred commits). For each commit, count files changed,
definitions whose digest changed, and base rows changed under `(document, preorder)` ids against
`(definition, index)` ids. Count the cases a symbol key must cover: redefinitions, conditional definitions,
`TYPE_CHECKING` branches, overloads, properties.

**Closure:** the distributions recorded in 0083, and the symbol key's form decided.

**Measured** (2026-11-08) on `nera-data-layer-gruel-main` (13,088 `.py` files, 218,046 definitions), last 224 non-merge
commits, with Python's `ast` standing in for the tape (`ast` has fewer nodes than tree-sitter; the ratios are what
count). The tool is a dev script, not engine code.
- **The commits are bulk.**
  - 109 of the 224 change more than 40 `.py` files and were only counted.
  - The other 115 change a median of 17 files (p90 34), and 120 definitions each (p90 277).
- **Base rows rewritten per commit** (medians, p90 in brackets):

  | Id scheme | Rows rewritten |
  |---|---|
  | `(definition, index)` | 18,605 (44,920) |
  | `(document, preorder)` | 64,575 (160,774) |
  | whole files (today's per-file memo) | 130,521 (319,704) |

  Per commit, the definition scheme is a median 15% of the files' rows (p90 37%) and 29% of the preorder scheme's.
- **Commits that change one file** (6 of them), the size of an edit:
  - definition: 246 rows;
  - preorder: 3,571 rows;
  - file: 6,064 rows.

  The definition scheme is 14x smaller than preorder and 25x smaller than the file.
- **What a key by qualified name must disambiguate**, at HEAD:

  | Case | Definitions | Share |
  |---|---|---|
  | conditional (inside `if`, `try`, loops) | 2,741 | 1.26% |
  | a name defined more than once in one scope | 1,010 | 0.46% |
  | `@overload` | 722 | 0.33% |
  | under `TYPE_CHECKING` | 40 | 0.02% |
  | property accessors sharing a name | 26 | 0.01% |

  A key of (qualified name, ordinal among the scope's definitions of that name) covers all of these. Its ordinal
  shifts only when a same-named definition is inserted before another, which is under 0.5% of definitions.

Not yet measured: key churn across commits (renames and moves), and rows under tree-sitter's tapes rather than `ast`.

**0094 §2's encoding, sized** (2026-11-08, `scripts/profiling/o126/encoding.py`). Same repository and `ast` proxy.
Granules as §2 sets them: the innermost function or method, a class header, and each module-level statement (an
import by the names it binds, any other by its digest and `#k`). A slot is a hash of the symbol over 14 bits. The run
covers 136 commits of the last 300 that change at most 40 files; two chunks of the run hit the 2-minute cap.

- **Rows renumbered per commit**, in the encoding against preorder: a median 3.8x fewer (p25 2.4x, p75 6.0x).
  - On the 7 one-file commits: 225 rows against 2,620 (11.6x).
  - The encoding renumbers a median 13.7% of the changed files' nodes; preorder renumbers 53%.
- **How clashes are settled hardly matters for rows.**
  - Settling by symbol order (§2) and probing in source order (today's flag) differ by 818 rows of 2.48 M.
  - Granules moved by a clash: 21 by symbol order and 35 in source order, over all 136 commits.
  - §2's rule is about stability (a granule never moves because of an unrelated insert), not about cost.
- **Module-level statements as granules save little.** Keeping them as one root granule, as the flag does today,
  renumbers a median 6% more rows per commit (12% in total, over 25 commits).
- **Field widths**, over the 13,088 files at HEAD:
  - granules per file: p50 17, p99 221, max 3,073;
  - granules that share a slot with another: 1,873 at 14 bits, 493 at 16, 7,559 at 12;
  - the largest granule is 21,388 `ast` nodes. Tree-sitter's nodes plus tokens are a few times that, under
    18 bits' 262,144.

  The flag's widths (14-bit slot, 18-bit index) fit. A file with more granules than slots keeps preorder ids
  (`DefIds::new`).

**Readers that assumed preorder ids** (2026-11-08). The flag's encoding can only go in once nothing reads a node
id's low bits as a preorder index or a subtree as a contiguous id range:
- `core_rr.rs` (part starts) and both guest paths (`facts/vtab/syntax/guests.rs`, `plan/corpus/guests.rs`) now map
  the id to its local index (`local_of`, `syntax::Numbering`) before indexing.
- `front/cycles.rs` treats `<` and `>` between two node variables under the flag as ordering by value
  (`Later`/`Earlier`), not as moves down or up the tree.
- `native/extension.rs`'s tape (dense, preorder vectors, and `down` as `n+1..=n+size`) is read only by
  `run_sql_native` over T's own per-file facts, which are preorder by construction. Every engine path drops
  `nearest` and `kseed`, so the tape never sees an engine id. It is unchanged.

## O127. Layout invariance of today's engine (decision 0083, spike 3)

**Method:** move definitions between modules of the 84 files, with imports rewritten by a script. Run with and
without typeshed, and compare findings and work (CPU, rows per stratum).

**Closure:** the departure measured, and the layout-invariance test (0083 §0) in the suite as a guard.

## O128. Search against eager propagation, counted (decision 0083, spike 4)

**Method:** from today's relations, on the 84 files and on 500 data-layer files:
- distinct `(f, targs)` keys against `gpar`/`gbind` rows;
- distinct `(class, attribute)` lookups against `found` rows and the `sub` closure.

**Closure:** the ratios recorded, and each question in 0083 §4's table confirmed as a search or moved to eager.

**Measured, in part** (2026-11-08). 84 files with typeshed, `calls.rr` plus counting rules, through
`RERAIL_PYTHON_CALLS_OUT`. Run at 166.6 s wall, which was this hour's exception to the run cap.
- **Generics are small here.**
  - `gpar`: 1,667 rows over 165 calls, and 426 distinct (class, variable, annotation node) keys, about 3.9 rows per key.
  - `gbind`: 824 rows; `garg`: 2,474; `ghcls`: 1,409; `fbind`: 227.
- **Attribute lookup:** `found` has 5,090 rows for 2,034 asked keys (`need_attr`), 2.5x.
- **Subclasses:** `sub` has 628 rows, of which 396 serve the 286 classes some reader asks about, 1.6x.
- **What dominates is `pv`.** On workspace nodes it grows from 24,475 rows without typeshed to 302,897 with it, and
  181,293 of those point at a library value.
- **Gap:** the benchmark writes no row located in a library document. So the library side (lookups on typeshed's
  classes, its own `pv`) is not counted yet. Next: count by `RERAIL_PROFILE`'s rows per rule, or write the hidden
  rows for counting.

So far, searching for generics, attribute lookup and subclasses saves 1.6–4x rows on this corpus. The bulk is the
value relation itself, which P1 and P2 shrink only if library values become declared facts.

## O129. Structure and declared types over the real library (decision 0083, spike 5)

**Method:** a first `struct.rr` (imports, bindings, class headers, written signatures) over the data layer's venv
and typeshed.

**Closure:** measured documents loaded, waves for the import frontier to settle, rows, CPU, RSS, and the share
that is the same for another workspace on the same interpreter. The library's P1 and P2 run once per machine and
are reused.

## O130. One analysis that really crosses files (decision 0083, spike 6)

**Method:** `param_in(f, i)` (parameter types from callers) and "never called", over the data layer's 9,010
files: full, then after one edit.

**Closure:** both cost what their delta reaches, under 1 GB, with no mechanism special to them.

## O131. The stored fixpoint read in place under 1 GB (decision 0083, spike 7)

**Closure:** measured sizes of P3's relations on 9,010 files, and the cost of maintaining a delta by index through
the vtabs without loading relations whole. The store layout (one table per relation, keyed indexes) decided.

## O132. Freshness of library state outside Git (decision 0083, spike 8)

**Method:** stat-check what P1 read in a real venv, parse `RECORD` and `pyvenv.cfg`, and hand-edit one
site-packages file.

**Closure:** an unchanged venv is verified in milliseconds, and the edit reaches P1 as a content delta.
## O133. `calls.rr`'s cross-file fixpoint grew threefold across the final-rerail-shape merge

Measured 2026-11-08 on the 84-file Jedi corpus with the memo off. Before the merge (`da0a634c0`), the fixpoint
stratum was 53 relations and 8.1 s of native joins. After it (`61dc0cd80` onward), it is 74 relations and 20.9 s,
about three times the join work. The Jedi agreement is the same, 6,299. `calls.rr` did not change. The merge
brought final-shape's guest hiding (O110) and lowering changes, which put more guest-annotation copies
(`rr_in_annotation_*`) and `base` into that stratum. Not investigated further: the code is final-shape's.

**Closure:** the cause is named, and the fixpoint's cost is back near 8 s or the growth is shown to be needed.

**2026-11-08: the cause is named. The growth is O110's correctness fix, not a planner regression.**
- Before O110, guest hiding read a reader's atom from the guest copy only (`rr_in_annotation_0_base`, …). A
  derived column that may hold host nodes lost those rows. O110 splits such a rule into a host rule and a guest
  clone, so the host relations are now derived too.
- That puts `base`, `fcall`, `fgp`, `fret`, `pcall`, `rret`, `wbind` and `wcall` into the fixpoint. `base` is
  recursive with `pv` through the guest clone of `sub`: `pv :- cret, rr_in_annotation_0_sub` and
  `rr_in_annotation_0_sub :- sub, base`, where `base :- basex, pv(e, "cls", b)`. Join work rose from 16.2 M to
  52.2 M steps. The largest new costs are the host and guest copies of one join (`ifn` over `icall` and over
  `rr_in_annotation_0_icall`), each run over more waves.
- The demand split choosing `ownvar` and `mro_dropped` (decision 0048) also changed: `demand_ownvar` is computed
  from readers inside the fixpoint. Refusing a split that would make a demand relation recursive cut the stratum
  from 74 to 58 relations but not its time (23.4 s, against 22.6 s, under load). It was not kept.
- Whether every host copy is needed is final-shape's to judge (hiding.rs). A copy whose rows are all guest-only
  could still be specialised. `RERAIL_DUMP_CROSS=<file>` writes the cross-file rules for that analysis.


## O134. A run read wholly from the per-file memo planned its cross-file part with no tree counts

Found 2026-11-08, once the per-file memo stored on the Jedi corpus. `calls.rr` read all 267 files from the memo
and never finished (killed after 20 minutes). The cause: the planner's tree counts (kinds, edges, documents and
constant postings) come only from evaluated chunks, so every `kind(n, "identifier")` looked empty, and `rpvar`'s
cross-file join began with `kind($pr, 1025) , kind($r, 1025)`, a cross product. `RDL_WAVE_TRACE` now names each
probe's component, head and planned joins, and `RERAIL_DUMP_STATS` prints what the cross part plans with. Those found it.

**Fixed:**
- Each chunk mix's measured counts are stored under the file-local program, named in the run's manifest.
- Memo hits, grouped by file type, add their mix's counts scaled by their nodes (falling back to their types',
  then the last measured mix).
- On the fixture, a memo read plans with 52 kinds over 1,493 nodes, as the evaluated run does. On Jedi, every warm
  and edited run finished, with answers equal to the evaluated runs.

**Closure:** met. Still open: storing exact per-file counts in the memo entry, in place of a mix's scaled ones.

## O135. The Python pack does not finish within the run cap: its libraries are copied into every program

Measured 2026-11-08 with decision 0084's profile and dump, on 538 of the pack's programs (O111's six left out),
over `fixtures/python/calls/project`, load 36 to 75. The run was stopped at 115 s, 116.4 s user CPU, before it
evaluated anything:

| stage | CPU | allocated | what it works on |
|---|---|---|---|
| parse each program | 9.4 s | 24.2 GB | 538 printed programs, each with its `#include`d libraries inlined |
| `hiding::hide`, per program | 19.5 s | 21.2 GB | 353,878 rules: 658 per program, of which a handful are the rule's own |
| merge the programs | 0.3 s | 0.3 GB | every library relation renamed `p<i>_…`, kept once per program |
| prune, simplify, demand | 20.6 s | 12.6 GB | `simplify::merge_identical` folds the 538 copies back into one |
| lower the program | 0.2 s | 0.2 GB | |
| `file_local::push_local_joins` (the split) | over 50 s, stopped | | no span of its own; `split_explained` |

Peak footprint 3.2 GB, against the 1 GB budget. At 50 programs the run takes 5.6 s, at 100 15.1 s: the passes
before `simplify`'s merge grow with the copies, and the split grows faster than the rules.

**Design:** decision 0089 (proposed 2026-11-08): libraries as modules held once, requests per distinct key, a
file's memo rows per unit; staged, each stage measured here.

**Progress (2026-11-08, 0089 stage 1).** `#include` now names a module, and each library is lowered once per run
under its own names. A program that adds rules to a library relation gets its own copy of that relation's cone.
`RERAIL_LIBRARIES=copied` keeps the old pasted path for comparison.

The run covered 534 programs over the Ruff fixtures (472 `.py` files), at load 14 to 24, CPU user+sys. Left out
are O111's six and the four single-rule failures listed under O111. On the calls project the pack finds nothing,
so it can't be compared there.

- **Whole pack, shared:**
  - finished, rc 0;
  - 76.7 s CPU, peak RSS 1.14 GB;
  - 6,184 findings.
  - The library's 2,343 rules are held once. 187,983 relations come from the library instead of being copied.
    Only 159 cone relations are copied.
- **Spans:**
  - parse each program: 5 s;
  - prune, simplify, demand: 0.54 s (was 20.6 s);
  - locate: 1.55 s;
  - split: 1 s;
  - `push_local_joins`: 11 s, now the largest.
- **Shared against copied, pack in four quarters of 134 programs:** findings equal in every quarter. The quarters
  add up to the whole run's 6,184.

  | quarter | findings | shared CPU / RSS | copied CPU / RSS |
  |---|---|---|---|
  | 1 | 3,776 | 10.2 s / 573 MB | 32.6 s / 1.53 GB |
  | 2 | 943 | 11.7 s / 431 MB | 23.3 s / 813 MB |
  | 3 | 387 | 19.5 s / 547 MB | 64.2 s / 1.49 GB |
  | 4 | 1,078 | 37.5 s / 676 MB | 65.5 s / 1.29 GB |

What remains against the closure:

- The peak, 1.14 GB, is over the 1 GB budget.
- `push_local_joins` is not yet measured as linear in the rules.
- Hiding still runs over the whole combined program once. Stage 2 makes it per module.

**Progress (2026-11-08, 0089 stage 2).** Equal requests now share one cone. A program that asks nothing is read
with its libraries' declarations and kind sets only.

- **Requests are keyed.** A request is held as a unit: the library relations of its cone, plus the program's own
  relations they read. Its key is the digest of the unit's rules and declarations, with variables as written, so
  equal keys mean equal questions. Programs with equal keys read one copy, `q<k>_…`.
  - `RERAIL_REQUESTS=copied` keeps one copy per program, for comparison.
  - A unit that reads a repository file the policy names stays per program.
  - Test: `libraries_are_held_once_and_equal_requests_share_one_cone`.
- **What this saves on this pack: little.** 29 of 534 programs ask, with 26 distinct requests, so 3 copies are
  saved. The cones are 159 relations in all. The largest share is 6 copies, for `cmp_same` and its cone.
- **Parsing is halved.** 505 of the 534 programs ask nothing. Each of them is read with its libraries' statements
  except their clauses. The exception is kind sets (unary relations), which lowering reads statically
  (`rr::parse_own`). Leaving the kind sets out lost 22 findings in quarter 2 (`nearest(…, fscope)`).
  - "parse each program": 7.84 s CPU and 17.3 GB allocated, now 3.3 s and 7.3 GB.
  - "parse the program": 10.1 s, now 5.1 s.
- **Hiding runs once.** It covers the 12,850 rules of the combined program, in 1.67 s. It was 19.5 s per program.
  Programs read under different bounds (`.guests off`) still take the copied path. The pack has none, and holding
  a library once per bound is left until one appears.
- **Whole pack, keyed requests, declarations-only parse:**
  - rc 0, 6,184 findings, equal to the union of the four quarters;
  - 100.1 s CPU at load 35 to 82, with the profile on;
  - peak footprint 1,076 MB (max RSS 1.57 GB, which counts mapped pages).
  - The quarters match stage 1: 3,776, 943, 387 and 1,078 findings.
- **What is left:**
  - `push_local_joins`, 13.2 s, now the largest preparation span;
  - the footprint peak, 1,076 MB, against the 1 GB budget. The largest rises are evaluation (`local/run`, 365 MB)
    and "stats" (355 MB).

**Before 0089 stage 3 (2026-11-08): what a per-unit memo could save.** This uses the profile of the same whole-pack
run. Adding one rule changes the file-local digest, so every file is evaluated again. Of that work:

| span | CPU | under stage 3 |
|---|---|---|
| `local/run` | 34.2 s | |
| … library strata (unprefixed names) | 3.8 s | read from the library unit's rows |
| … lowering helpers (`fl…`, `rr_in_annotation…`, not a program's) | 3.0 s | mostly the library's, so mostly read |
| … programs' own and requests' strata | 6.8 s | made again |
| … the rest (per chunk: opening, loading inputs, indexes) | about 20 s | made again |
| `local/guests` | 18.1 s | unchanged (tape cache, already content-keyed) |
| preparing (`stats`; `push_local_joins` is 13.2 s of it) | 24.3 s | unchanged: the prepared program is keyed by the whole pack |

So per-unit rows would save at most 4 to 7 s of the roughly 100 s a pack edit costs. The larger costs are three:

- the per-chunk overhead of `local/run`;
- `push_local_joins`;
- preparing the whole program again for one rule.

Decision 0094 places stage 3 after phase 0, which keys the prepared program by the module graph.

**Measured again after the day's merges (2026-11-08, trunk at `32b16e4fe`, with O141's fix).** Whole pack, 534
programs over the Ruff fixtures, no profile, load 27 to 56:

- rc 0, 6,184 findings, equal to before the merges;
- 64.6 s CPU (stage 1 alone: 76.7 s), 821G instructions;
- peak footprint 1,124 MB, still over the 1 GB budget.

Before the fix, the merged trunk took 161.6 s with the profile on (100.1 s before the merges). Quarter 1 went
from 11.7 s before the merges to 19.6 s, and back to 10.6 s with the fix.

**Closure:** a library is one program unit, parsed, bounded (`hiding`) and merged once, however many pack rules
include it, so the combined program is the libraries once plus each rule's own; the file-local split measured
and linear in the rules; the whole pack runs within the cap and the memory budget, measured.

## O136. `calls_cx` with both memos off grew past 10 GB (fixed)

**Seen (2026-11-08, t22).** A cold run of `calls_cx` over the 84 files ran with `RERAIL_CORPUS_MEMO=off` and
`RERAIL_CROSS_MEMO=off`. It reached 10.5 GB RSS and took 416 s of CPU on the main thread, 385 s of it in the cross
part. The answers were still right (6,299 agree).

**Cause.** Demand (decision 0048) cut the positive read `uinst(t, c)` out of the rule
`bv(f, x, "inst", s) :- pann(f, x, t), pread(f, x), uinst(t, c), anc(s, c), docof(f, d9), docof(s, e9), rr_cx_reach(d9, e9)`.
- `uinst` is the only link between `t` and `c`. What was left in `pre_<i>` joined only through documents: 4,719
  `anc` rows, against every annotation of every document they reach. That gave 47M rows in and 24M out.
- Four rules shared that body, and each made its own copy of it (`pre_566`, `pre_590`, `pre_796`, `pre_819`). Each
  copy allocated 6.8 GB.
- The reader then looked up only 9,319 `(t, c)` keys in it.
- `disconnected`, the guard against exactly this product, counted a shared document as a join.
- With the cross memo on, the run is planned per component, and the split does not arise.

**Fix.** In `demand::disconnected`, a document variable does not connect atoms. Those are column 1 of `docof` and
the columns of `rr_cx_*`. The test is `atoms_joined_only_by_their_documents_are_disconnected`.

| both memos off | CPU main thread | cross part | peak RSS | agree |
|---|---|---|---|---|
| before | 415.9 s | 384.8 s | 10,468 MB | 6,299 |
| after | 87.3 s | 51.4 s | 846 MB | 6,299 |

**Left.**
- Identical `pre_<i>` bodies could share one relation. It does not matter once the product is refused.
- The memo-on prime's cross part takes 81.7 s, against 51.4 s here. That 30 s is the cost of splitting by import
  component (0076).

## O137. Two departures of delete-and-rederive from a full run (spike O125)

Two runs of `spike_o125_replay` keep rows a full run does not have. Both are in `calls.rr`'s points-to component,
and both in a `rr_in_annotation_0_*` relation.

- **Step 05** (commit 28d80c48d45, 14 files). `call_open` is 2 rows off. `RERAIL_CROSS_VERIFY` traces it to one row
  of `rr_in_annotation_0_base`, `(4372681134333496151, 4372681134333495048)`. It is in `__old` and missing from
  `__del`, although its over-delete rule
  `rr_in_annotation_0_base__del(c, b) :- fl695_ext_base_up__old(c, e), pv__del(e, 1451, b), !special_base__old(e)`
  has matching rows (`pv__del(…6154, 1451, …5048)`, `ext_base_up__old(c, …6154)`). Turning the demand split off
  does not change it. The suspicion is the evaluation of the rewritten rule (delta probe or replanning), not the
  rewrite.

  It persists on the tree with every branch merged (survivors in place, scalar sharing, exports in memory;
  2026-11-08). One session over steps 01–20 matches a full run until step 05. From step 05 on, `call_open` has 2
  extra rows (`l2055_lazy_result/tests/test_lazy_result_bugs.py:1067`), unchanged through step 20. Survivors in
  place is not the cause, because the departure is older than that spike. On the merged tree, the row is
  `(c, b) = (…6151, …5048)` with `e = …6154`, and the rule now reads `fl684_ext_base_up__old(c, e),
  pv__del(e, 158, b), !special_base__old(e)`. Every input row is present: `ext_base_up__old(c, e)`,
  `pv__del(e, 158, b)` and no `special_base__old(e)`. The profile shows the rule ran with `pv__del` as its delta:
  40 delta rows in, 8 out, and the row is not among them. `base(c, b)` itself is in `base__del`. So the fault is
  in the semi-naive evaluation of the over-delete stratum (174 relations). The step to take next is to replay
  step 05 with that stratum evaluated naively. If the row comes back, the delta bookkeeping is wrong; if not, the
  probe is.
- **Typeshed, step 17.** `rr_in_annotation_0_gfn` has 60 rows extra and 3 missing. The extra rows are in `__del`
  and in `__ins` but not in `__old`: the insert rules derive them from rows that a full run does not have.

**Repro:** make the replay with `scripts/profiling/o125/replay.sh <data layer repo> <replay dir>
scripts/profiling/o125/files.txt`, then run `O125_REPLAY=<replay dir> O125_STEPS=4-5 RERAIL_CROSS_VERIFY=1 O125_TRACE=<row ids>`, test
`spike_o125_replay`; add `RERAIL_PYTHON_TYPESHED=1` and `O125_STEPS=16-17` for the second.

**Closure:** both runs equal a full run, with the cause found and a test that fails without the fix.

## O138. Two data-layer checks read run artifacts, and no artifact or revision reader exists

Found 2026-11-08, over the 75 data-layer candidate checks on 133 files of the data layer. `plan-composition-architecture`
and `test-budget` fail before evaluation: `spc_fingerprint: revision and stamped-artifact readers are not implemented`
(`corpus/inputs.rs`, `populate_with`). They read `artifact="plan_composition_probes"` and `artifact="test_collection"`.
No candidate check uses `at="base"` yet.

The design is written (`docs/rule-syntax/lib/core.rr`, run artifacts; `data-layer/RUN-FACTS.md` §2–§5;
`FINDINGS-LAYER.md` §2). What is missing:

1. `.artifact "name" inputs globs` kept as a program setting, checked for one declaration per name.
2. `artifact="name"`: the file `name.json` in the facts directory (`--facts DIR`, default `.rerail/facts/`), its
   envelope (`format`, `artifact`, `inputs`, `status`, `data`) checked, and `path=` applied to `data` with every
   `IO=json` option.
3. Freshness: SHA-256 over sorted `path NUL blob-id LF` of the files the walk sees under the globs. The blob ids are
   the content hashes the store already has (decision 0092 §2's leaves). A different digest gives no rows.
4. `input_error("artifact:NAME", kind, detail)` with `missing`, `stale`, `failed` or `malformed`.
5. Withholding: rules that read the artifact through `!` or an aggregate give no findings when it has no rows, and
   their places are Unknown (FINDINGS-LAYER §2). Until the findings layer has Unknown, the run reports the
   withheld relations, so a missing artifact is never a silent pass.
6. `rerail facts stamp DIR --out DIR`, so a test can make a fresh artifact.
7. `at="base"`: the file as it is at `git_rev("base", …)`, read from the object store.

**Closure:** both checks lower and run on the sample with no artifact (static findings, withheld rules reported) and
with a stamped one (their artifact findings), and a stale stamp gives `input_error(…, "stale", …)`.

**Built 2026-11-08 (items 1–6):** `corpus/inputs.rs` reads `artifact=` from `--facts DIR` (default
`<root>/.rerail/facts`), checks the envelope and the digest (git blob ids over the files the corpus walk sees), and
gives one `input_error("artifact:NAME", kind, …)` per artifact. `withhold` removes, to a fixpoint, every rule that
reads an incomplete relation through `!` or an aggregate, and the run prints each finding relation that may be
missing rows, with how many of its rules read the artifact (`unknown: budget_exceeded (artifact:test_collection
missing; 2 of its 3 rules)`). `rerail-engine facts-stamp RAW --out DIR --root REPO POLICY…` stamps. On the 133-file
sample both checks now run, in 0.2 s and 1.0 s of CPU: without artifacts, with a failed producer, and with a fresh
artifact (no Unknown). Tests: `corpus::inputs` (12).

**Built 2026-11-08 (item 7, `--require-facts`):** `at="base"` reads a `filename=` input from the object store, through
`gix`, at `logic --base REV` (a commit id or a reference name), under the corpus root's prefix in the repository.
Without a base, or with a path the base lacks, the input gives `input_error("PATH@base", kind, …)` and its readers
under `!` are withheld, as an artifact's are: a burn-down comparison never reports every entry as new.
`--require-facts` fails the run on an artifact that is missing, stale or malformed, and on a failed one unless a rule
reads `input_error` (RUN-FACTS §5). Tests: `corpus::inputs` (16).

**Still open:** the merge base. `--base` names the revision; computing the merge base of a change and giving
`git_rev`, `git_has` and `git_commit` as facts is the git provider's (0094's order of work, last). Unknown *places*
(FINDINGS-LAYER §2's second evaluation), which needs the findings layer.

## O139. `RERAIL_PROFILE` adds 5–8 s of CPU to every run

Measured 2026-11-08, load 47–110: `data-layer-compat-aliases` over 133 files takes 0.17–0.25 s of user CPU, and 5.0–8.3
s with `RERAIL_PROFILE` set. Over six checks the run's phases account for the same time either way. The rest, 8.5–9 s
per check at that load, lies outside every phase and span: the profile's own report. Profiled timings of the data-layer
checks taken before this was found are inflated by it.

**Closure:** the report's cost found and made proportional to the program, and the profile states its own CPU.

**Found and fixed 2026-11-08.** The cost was the roofline's calibration (`profile/roofline.rs`): before writing
each report, about ten micro-benchmarks of 1–64 M iterations, five times over, 7.25 s in a debug build at load 33.
The report now measures only the kinds of work the run charged, and keeps the roofs per engine binary (its path, size
and modification time) for a day in `<temp>/rerail-roofs/`. The report says where they came from and how long it
took ("read from the roof cache in 0.00 s"). data-layer-compat-aliases: 0.25 s with the profile, the same as without;
the first profiled run of a new binary still pays the calibration once. Test:
`the_roof_cache_reads_back_what_it_wrote`. Profiled timings taken before this are inflated by 5–9 s each.

## O140. Engine rules that make per-file facts look global

Found 2026-11-08 by `RERAIL_SPLIT_DUMP` (decision 0093 §4). After a30c360ee fixed the demand context, the data-layer
checks are file-local or same-unit in 42 checks by their own rules, but in 17 with the library's and lowering's;
unions of per-file rows rise from 1 check to 19. Two causes remain:

1. **Located rows projected to a set** (`x(c) :- x_at(_, c)`): lowering drops the document column, and readers that
   bind the document read the union.
2. **An equality between two per-file pieces' carriers placed in the cross-file part** (`deco_word`'s `$e0 = $d`).
   `file_local::push_local_joins` should treat the equality as one document, as the classifier does.

And the findings assembly (`rr_diagnostic`, `violation`), which drops the document in cross-file rules; it is the
findings layer's stages 2–3.

**Closure:** the classification rerun with both fixed shows no check global through lowering alone, with findings
equal on the 65 checks that finish both ways.

**2026-11-08, worked:**

- **Item 2 is not a leak.** In the 39 checks with `x(c) :- x_at(_, c)`, none of the projections' 144 readers binds
  the projected row's document; the 7 that bind some document read the projection by a value. A projection is a union
  of per-file rows, a unit fact (0092 §4), and there is nothing to rewrite (0093 §4, corrected).
- **Item 3 is fixed.** `file_local::push_with` closes each candidate's variables under the rule's `a = b` equalities,
  both when it groups the atoms and when it counts their independent node groups. `deco_word`'s four cross-file rules
  are now pushed into the chunks (bootstrap-engine-invariants, graph-lazy-sql-plan, schema-bootstrap-contracts).
  Test: `an_equality_between_two_pieces_joins_them_in_the_chunks`. Over the 25 checks with variable equalities in
  their cross part, findings are equal with and without the grouping half of the fix, and CPU is within noise.
- **Found while checking: a nested aggregate lost the rule's variables.** `count : { bei_identity(x), count : {
  def_walk(n, f), bei_ref(n, x) } = 0 } > 0` made a helper of x alone, so bootstrap-engine-invariants failed to plan
  after the final-rerail-shape merge ("unplaced literals … in rr_agg72($x)"). The helper now has every variable the
  rule shares with its nested aggregates, bound by the context. Test:
  `a_nested_aggregate_keeps_the_rules_variables_it_reads`.
- The findings assembly (`rr_diagnostic`, `violation`) is a union too; it moves when the findings layer is built.

## O141. The 66-rule pack costs 70% more instructions after the merges of 2026-11-08

On s50 (43 files), with the scope guards off, the pack took 380.8G instructions at `329e0b5d1` and 644.5G at
`9be591511` (load 33–77). The guards' own share is unchanged (681.1G on, +5.7%), and the 32 violations are the same.
So the rise is in what the merges brought (trunk's 0081–0089, phases, Typing-support, O136's demand rule, the SQL
work), not in 0091.

**Repro:** `packguard.sh s50 240 off` (the measuring harness: `measure_converted_rules` over the data-layer
candidates) on each merge of the day.

**Closure:** the merge that adds the cost is found by bisecting the day's merges, and the cost is explained or
removed.

**Closed (2026-11-08).** The cause was found by sampling, not bisecting.

- **Where.** `sql_kernel::component` ran `ProgramAnalysis::analyze` over the whole program for every stratum it
  ran, in every chunk. That analysis is the logical key and scalar analysis from `0edfc6227`, which came in through
  the Rerail-rule-expressivity merge (`9be591511`).
- **What it cost.** Each stratum's own time rose from about 1 ms to 0.45 s, with 233 MB allocated, outside every
  named span. On the Python pack that was hundreds of strata.
- **The fix.** The planner's model already made the analysis once per plan. It is now held behind an `Arc` and
  shared. It is made again only when the demand split replaces the program (`plan::run`). `emit` shares it too,
  where it was deep-copied.

s50, 43 files, the same 32 violations:

| guards | before the merges (`329e0b5d1`) | after (`9be591511`) | with the fix |
|---|---|---|---|
| off | 380.8G | 644.5G | 336.3G |
| on | 390.5G | 681.1G | 343.3G |

The fix leaves the pack slightly cheaper than before the merges. The analysis, planner, corpus and SQL executor
tests pass, and so does the replay gate (`converted_rules_give_the_recorded_findings`).

## O142. `calls_cx`'s process peak RSS is above the run's sampled peak and the 1 GB budget

**Seen (2026-11-08, t24).** On the 84 files with typeshed and both memos on, the run's report gives a peak RSS of
762 MB for a cold run and 355 MB for a warm one. `/usr/bin/time -l` gives 1.46 GB and 1.07 GB for the same
processes. An edit's run gives 712 MB and 1.43 GB. Phase 0's whole-run cutoff is not the cause: the warm figure
is with it off.

**Open.** The report samples RSS at phase ends, so a transient peak inside a phase is missed. Find the phase, and
whether the per-phase figures the budget is checked against should be the process peak.

**Closure:** the process peak within the budget, or the phase that exceeds it named, with a figure the report gives.

## O143. The Python pack's annotation stratum read every annotation guest once per binding

**Status:** closed (2026-11-08).

**What.** On the Python pack (534 programs), the `ann_input/ann_strip/type_app` stratum cost 39 s of about 89 s of
CPU. The cause was not a collision of guest ids. In every chunk, each `ann_strip` row has its own `t`: the rows equal
their distinct first columns, about 480 to 800 per chunk. The cause was join order.

- **Stale postings.** The first chunk of a mix measures the program's constants' postings before the guest rounds
  make any guest. So `parsed($s, "annotation", $r)` was priced at no rows.
- **Stale observations.** An atom that shares no bound variable kept its fanout observed in one wave or chunk. That
  number is the relation's size at that moment, not a property of the join.
- **The result.** Between chunks the replanner moved `parsed` straight after the delta, as a filter. Every delta row
  then read every annotation guest: 10,857 bindings gave 5.4M rows for RUF013 alone.

**The fix** (`5e335eb4d`):

- the guest postings are measured after the guest rounds;
- an atom that shares no bound variable is priced from its relation, with at least one row per binding.

The run went from about 89 s to 51.6 s of CPU, with identical findings.

**The rest of the run, measured the same day.** The work below brought the pack to 8.7 s warm and 61.3 s cold (CPU,
user + sys, load 46 to 74), with identical findings:

| What | Before | After | Commit |
|---|---|---|---|
| Strata that fell back to SQLite (native steps for calls, arithmetic, `strlen`, computed comparisons, string joins) | 168 strata, 7.4 s | 5 strata, 0.06 s | `152096654` |
| Exports to the cross-file part, held in memory and read in place instead of copied through `x` | 5.6 s | — | `49035cc90` |
| A collection lock taken per guest parse; one locate per row instead of per column | 2.6 s (sampled) | — | `92b5bbe2c`, `e88cdd20d` |
| The per-file memo, which kept no rows on a cold run (text-test rows of other chunks' strings) | 0 of 486 files | 486 of 486 | `6ae30c48a` |
| The combined program, kept in the store by the module graph (decision 0094 §6) | 6.4 s warm | 0.24 s | `6ae30c48a` |
| The prepared program's split, kept with it | 0.57 s warm | 0.11 s | `24d05da80` |

What a warm run still spends (8.7 s):

- the cross-file part, 4.5 s: joins 1.75 s and engine open 0.78 s. The 1,473 small strata take 0.63 s in all,
  about 0.4 ms each.
- the memo replay, 1.6 s. 0.6 s of it measures the replayed exports again (`Totals::add_rows`).
- the workers' memo lookups, 0.9 s, mostly zstd and decoding.
- the prepared program's lookup, 0.56 s.

A warm run with nothing changed still derives the whole cross-file part again. That is 0090's maintenance (an
empty delta), not a fixed cost.

## O144. A relation the request cone derives is written empty as an output

**Seen (2026-11-08, t25).** `bootstrap.rr` first declared `p0_resolved` as an output. It is in the cone of `load`
requests (a load is asked from it), and its output table was written empty, though the run had the rows. The cross
path holds back cone relations that are outputs and expects them to be read from their table
(`corpus.rs`, `held.extend(cone.reads … !outputs_of(r))`); the table is not filled.

**Worked around.** `bootstrap.rr` outputs copies (`p0_include`, `p0_missing`, `p0_absent`) that no request reads.

**Closure:** a cone relation declared `.output` is written with its rows, and `bootstrap.rr` drops the copies.

## O145. A per-file relation with no node column is lost for documents a `load` brings in

**Seen (2026-11-08, t25).** A loaded document's rows of the per-file relations the cone reads are found by its
node range (`corpus.rs`, the `rr_load_new` join on the relation's first location column). A per-file relation
whose columns are all symbols and numbers (`p0_names(f, at, name)`) has no location, so a loaded module's
`#include`s did not reach the cone: phase 0 stopped after the first level.

**Worked around.** `p0_names` and `p0_declares_doc` carry the node `i` as their first column.

**Closure:** the per-file rows of a loaded document are found by the document they were derived from (the chunk
builder knows it), so a relation needs no node column; or the engine refuses such a relation with a message.

## O146. Tests against the shared default tape cache wait on its atoms lease

**Seen (2026-11-08, t25).** `$TMPDIR/rerail-tape-cache.sqlite` (425 MB) is shared with other sessions. Each phase-0
run takes the atoms lease, and while another process holds it a test waits (`LEASE_WAIT`, 60 s); a run of
`logic::rr::` took minutes against seconds on an isolated store (`RERAIL_TAPE_CACHE=<own path>`). Phase-0 runs take
no corpus session (a session held across the nested run deadlocked against the lease).

**Closure:** the lease is held only across the atoms write, not the run; or tests default to a per-process store.

**Closed (2026-11-11): decision 0118.** A test's store is its own: a copy of a template made once per build, removed
when its test ends. No test shares a store, or its atoms lease, with another test or another process.

## O147. Each phase-0 run costs about 0.25 s, paid once per seed set

**Seen (2026-11-08, t25).** A run of `bootstrap.rr` over one text with includes costs about 0.25 s CPU when nothing
is kept. Callers that resolve programs one at a time pay it each time: `rule_syntax_files_pass_check_rr` took about
1,300 such runs until it asked for all its files in one (`phase0::graphs`). The pack loader and the corpus run batch
already. A process keeps each seed's graph while its modules' stats hold.

**Closure:** every caller that resolves more than one program batches its seeds, or a run's fixed cost (planning
`bootstrap.rr`, opening the store) is below 50 ms.

## O-artifact-entries. An artifact is one digest, so one edit makes all of it stale

Decision 0095 §2. `test_collection`'s inputs are `**/*.py`: after any edit the artifact is stale everywhere, and
locally it is never fresh again. **Closure:** the v2 envelope with entries, each with its own digest and status;
`facts-stamp` writing entries; a stale entry withholding and making Unknown only its rows' places; a test where one
edited key file stales one entry.

## O-artifact-digest-p0. Artifact digests walk and hash the tree beside phase 0

Decision 0095 §3. `inputs_digest` walks the root and hashes every matching file on every run and in every cutoff
check, and it hashes git blob ids where phase 0 keys content by sha256. **Closure:** digests over phase 0's
content sources, no read of an unchanged file on a warm run (counted by the observer), and stamp and run agreeing.

## O-artifact-store. One stamp per artifact, lost on a branch switch

Decision 0095 §4. **Closure:** stamped entries in the store by `(artifact, key, digest)`, absorbed from the facts
directory; a revert finds its old stamp; least recently used entries removed within the disk budget.

## O-artifact-imports. An entry's inputs are globs, not its imports

Decision 0095 §2. **Closure:** an entry's digest as a join over its import closure, once phase 0's module graph is
a phase output (t24, "not built").

## O-test-collect-rules. `test-budget` counts tests with rules, not pytest's artifact

Decision 0095 §1, t13 §6–§7. The spike's model and draft rules are in `notes/t13/`.

**Built (2026-11-08).** The `product` aggregate (`de8b50a9f`); `python/pytest-collect.rr` (`8c68e8d82`), the
model's levels 0–6 as rules, with lengths evaluated on demand (`pc_want`) and bounded by a number domain, not by the
tree; `test-budget` counting with it, the artifact optional, `collect_disabled` reported as `collection_disabled`.
**Measured (2026-11-08, pinned corpus, against `notes/t13/truth-full.json`).**

| Scope | Files exact | Total (pytest 14,449; model 13,610) | CPU |
|---|---:|---:|---:|
| `conform/` and the root's files (1,418 files) | 1,120 of 1,143 | 13,515 | 327 s user + sys, load 12–17 |
| the same plus `data_layer_testing/`, on the five directories that differed (394 files) | 143 of 145 (the model: 145) | | 63 s |

With `conform/` alone, all eleven files where the rules and the model differ read a name from outside it: a case
list (`READ_PERMISSION_SCENARIO_KEYS`), a decorator that replaces a class with a function (`requires_capabilities`),
or an enum. `data_layer_testing/` holds the first two; `test-budget` reads it. The two files left read enums of
`data_layer` itself (`DelegationCapability`, `BranchTipMaterialization`), which the scope leaves open.

**Measured (2026-11-09), `test-budget`'s scope** (`conform/`, `data_layer_testing/`, the root's files; 1,601
files): 1,127 of 1,143 exact, total 13,585; 262 s user + 4.6 s sys, load 90–130. The four files where the rules and
the model differ read an enum defined outside the scope (`MatchProducerFamily` in `l1470_matching_contracts/`,
`BranchTipMaterialization` and two more in `data_layer/`); each is reported open and counted as 1.

**Beyond the model (2026-11-09).** Lists a function builds by `append`/`extend`/`+=` under `for` loops, helpers
with several returns (the largest), function-local names, and `range` bounds given as `len(x)`.
`test_speculative_publication_scenarios.py` (pytest 66, model 13) and `test_branches.py` (18, model 8) are now
exact. The same scope run with them: 1,129 of 1,143 exact, total 13,648; 208 s user + 4.2 s sys, load 20–60; the
only files below the model are the four enum files above. A list built by introspection (`test_phase1_case_matrix.py`, 819) has no length; it stays in
`collect_disabled`.

**The enums need only their documents (2026-11-09).** A scoped run of the four files, plus each enum's re-export
chain (`data_layer/matching.py` → `l5250_matching/__init__.py` → `l1470_matching_contracts/graph_producer_models.py`;
`data_layer/versioned_schema.py` → `l7000_versioned_schema/compatibility.py` → `…/versioned_schema.py`;
`l1500_llm/delegation_consent.py`), counts all four exactly (22, 12, 25, 40): no rule change.

**Status:** closed except for the scope: an enum outside `test-budget`'s scope is open. Through the `values` phase
(O-names-cost) the four enum files count exactly on a 212-file root; over the whole workspace that waits on the
phase's cold cost. Expected over the whole
workspace: 1,133 of 1,143, once names over the whole workspace fit the budget (O-names-cost). Known difference from
the model: a nested Test class reached through two parents counts once (the owner is a node, not a path).

## O-names-cost. Name resolution costs about 75 ms of CPU per file

**Seen (2026-11-08).** `python/pytest-collect.rr` over the 9,010-file data layer ran past 9 min of CPU and 1.8 GB,
and was stopped. On the 133-file sample, `resolve_export` alone costs 15 s of the run's 28 s (debug build), and
nearly all of that is `qn`, which reads `look`: the LEGB walk of every name reference (`type_checking_block` reads
`qn`, and `import_time` negates it). Every check that reads `sym_of` pays this.

The run over `conform/` (1,418 files) also peaked at 3.8 GB RSS, in the parse workers and the cross-file part,
against the 1 GB budget.

**Progress (2026-11-09).** `python/pytest-collect.rr` no longer reads `sym_of`: its own lookup (`pc_sym`) is asked
only for the expressions the count needs (`pc_symq`), at module level, through `live_def`, `live_asg`, `ibind` and
the from-import chain, with no LEGB walk. The 133-file sample went from 28 s to 10.4 s. Rewriting the lookup over
`parsed_in` guest parses was slower (68 s, and past 2 min) and was dropped. The library-wide `sym_of` is still
whole-relation: `export_query(q) :- qn(_, q)` in `imports.rr` and `resolve_export`'s identity rule ask for every
name.

Memory, on the 1,601-file run of `test-budget`'s scope: the engine reports a peak RSS of 3,770 MB in the parse and
batch workers and the cross part, and 1,184 MB in the local part, against the 1 GB budget. Not yet looked into.
(`/usr/bin/time -l`'s "peak memory footprint", 1,245,496, is the same in every run, the smallest included: it is
the wrapper's, in bytes, not the engine's.)

**Spikes (2026-11-09) toward reading the enums' documents.**
- A, the whole workspace (no scope): stopped at 20 min, 527 s user + 11 s sys at load 108–150, unfinished.
- B, loads along import chains (`load` inside the fixpoint): the cone of requests may read per-file exports only
  ("reads base relation node"), so the node-level demand cannot drive loads. A per-file, symbol-keyed demand
  (the names a document's decorators import, chained over `ifrom`) loaded 76 documents for the four enum files
  and counted all four exactly; each loaded document ran the whole per-file part (22.7 s over 285 files).
- `struct` (P1) costs about 3 ms per file on the 133-file sample against 72–80 ms for `pytest-collect`; over the
  whole workspace it did not finish in 115 s at load 80.
- Direction (to decide): module- and class-level values as written in P1, evaluated across files after P2
  (`len`, `int`, enum members, by symbol); `pytest-collect` reads them for imported names, and runs on the
  checked files only.
- `values` prototyped (`cabb761ad`; typing/programs/values.rr, exports `len(q, n)`): exact on the fixture in both
  scopes. Over the whole workspace (2026-11-09, the four enum files checked, struct and values over all 9,010
  documents) it did not finish in 1,500 s: 1,803 s user + 33 s sys, load 11–33 at the start and about 20 at the
  end. The values phase's store was still being written when it stopped, so the four counts were not read.
- Profiled (2026-11-09) on samples of the workspace's directories (149, 601 and 2,063 files; 10.4 MB at 601,
  33.6 MB at 2,063, against 137.9 MB in all), each phase alone (`struct` reused from the run before through the
  phase record), CPU user + sys at load 8–23. Two rules grew with the square of the workspace, both fixed:
  - `v_name` of an assigned name joined from `struct__binds` (every assignment in the workspace: 15,820 rows at
    601 files) to the chunk's wanted names, 1.57 M pairs for 239 matches; the planner priced `v_nq` at 0.35 rows.
    Now the name and its module come first (`v_idn`), and the binds are probed on both. `v_dom`, the whole
    workspace's symbols remade in every chunk, is gone: an attribute's symbol is checked against struct's three
    exports directly.
  - In `python/lengths`: `len_num` (0..100000) counted up by a recursion of 100,001 steps in every chunk, now one
    join of digits; `len_plain`'s count made a view of every child edge (295 k rows for 116 lists), now only the
    asked lists'; parentheses went through `python/ast`'s `unparen`, made for every node, now one `child` step as
    asked. 149 files: 7.7 s to 5.2 s. Fixture and outputs unchanged (325 rows at 149 files, 1,573 at 601).
- After the fixes both phases are linear: from 601 to 2,063 files `struct` took 27.8 s to 122 s and `values`
  19.8 s to 66 s, every span growing alike (parsing too). Over the whole workspace that is about 500 s and 270 s.
  Half of `values` (34.6 of 66 s) is parsing and projecting its documents again, as every phase does; then
  `len_num` (0.24 s a chunk, now the domain's join), `posarg` (every call's arguments) and `v_asg`. Peak RSS 578 MB.
  The tape is memoized across phases and runs (the tape cache): with it on, three phases over 596 files read
  1,788 documents and parsed 591 (1.75 s). What each phase repeats is the tape's restore (0.7 ms a document), its
  facts (2 ms) and the batch's projection, the largest; the projection's string table (SipHash) and three
  per-node `wants` lookups took a fifth to a quarter of it, fixed (`bc4c60651`). Every measurement above and the
  1,500 s run had the tape cache off (`RERAIL_TAPE_CACHE=off`).
  `values` keeps reading every document: a phase's exports are keyed by its own inputs, so any reader reuses them;
  reading only what one reader's documents import would key them by the reader.
- Whole workspace, tape cache on, cold (2026-11-09 15:03, load 93–138): `struct` ended and was recorded at 13
  minutes (218,783 granules; tapes for 8,986 files); `values` had not ended at 1,800 s (1,327 s CPU). A warm rerun
  reuses both.
- The values phase as `test-budget` would read it, on a root of 212 files (2026-11-09, load 31–35): the four enum
  files, their conftests, each enum's re-export chain (19 files), and `data_layer_testing/` and the root's files
  (`pcE.rr`'s scope). `struct` and `values` run over all 212; `pytest-collect` reads `values__len`. All four exact
  (22, 12, 25, 40), none open. Cold 25.0 s CPU (318 MB peak RSS); phases reused 13.9 s; with the per-file memo on,
  19.4 s then 5.6 s. `test_fts_versioned_isolation.py`'s 12 needs `data_layer_testing`: its `TestFTSBranchIsolation`
  is replaced by a function (`@requires_capabilities`, `pc_replaced`); without the decorator's document the class
  counts 21 more (its fixture's 7 scenarios, now read through `values`, times three tests).
- What a cold phase spends, measured on samples (237, 591 and 2,063 files), and fixed (2026-11-09; outputs
  compared each time: struct's four exports on 237 files, `values`' 1,379 rows on 591, the fixture's counts):
  - The per-file memo's lookup decoded every plan ever run on the file (zstd), 0.86 ms a lookup; it now reads the
    run's row alone, 0.2 ms (`17c00cc9c`).
  - `py_dead_at` (python/conditions) joined `py_dead` with every child edge in the cross part (4.3 M rows at 2,063
    files, 6.2 s); its descent is now per file below guarded branches only (`8e3f8ee4b`): struct 14.35 s to 11.84 s
    on 237 files.
  - `pdef` joined `nearest` in the cross part, which then held and indexed every node's nearest scope (352 k rows
    and two 150 ms index builds at 237 files, 4.7 s and 2.4 s of the 2,063-file cross part); the lookup is now per
    file (`pdef_at`, `34f45e49c`): struct's cross part 0.63 s to 0.33 s on 237 files.
  - `len_num`, 100,001 numbers derived in every chunk to bound a length, is gone: the bound is `t >= 0, t <= 100000`,
    which the cycle check now takes as finite (`34a2dc514`). `values`' rules on 591 files: 8.86 s to 6.58 s.
  - What a cold phase still spends on 591 files (load 120–150): `struct` 26 s CPU (rules 7.2 s, projection 8.1 s,
    tape restore and facts 3.3 s, parse 1.9 s); `values` 23.5 s (rules 6.6 s, projection 4.1 s, tape and facts
    4.2 s). Linear in the documents, so over the whole workspace about 400 s and 350 s of CPU cold. A warm run
    reuses each phase from its record.
- Whole workspace, cold, 4 threads (2026-11-09 22:13, `44d7dac3f`, load 35–88): `struct` then `values` over 9,024
  files finished: 861 s wall, 966 s CPU (899.6 user + 66.1 sys), 36,643 `lens` rows. Peak RSS 3,142 MB, three times
  the budget. Two causes found, not yet fixed:
  - Memory was above 1 GB from the start, so the chunk sizer shrank every chunk to its least (8,192 nodes): 2,028
    chunks, against 4 for 237 files. Every per-chunk cost is paid 2,028 times.
  - `v_enum` (`values.rr:77`) and its count (`rr_agg63`) read struct's exports alone, no document, yet run in the
    per-file part, so each chunk derives them again over all of struct's exports: 116.6 M `struct__member` rows for
    one answer, 148 s and 35 s. Relations made from facts alone are left to the split (`constants::foldable` skips
    `constant_relations`), which derives them per chunk; cheap for small facts, not for a phase's exports.
  - The rest: per-file rules 477 s in all, projection 149 s, tape and facts 137 s, cross part 120 s.
- `v_enum` and `rr_agg63`, constants of struct's exports, are derived once before the chunks and bound beside the
  exports (`run_constants` in `corpus.rs`), no longer in every chunk: on 237 files each runs once in a few ms, and
  `lens` is as it was (361 rows). Not yet measured over the workspace.
- A main program over phase exports alone (`defs(q, k) :- struct__def(q, k)`) no longer builds documents
  (`58a5243a1`): 480 builds to 243 on 237 files for `struct`, 726 to 489 for `values`.
- Spike (2026-11-09, `RERAIL_SPIKE_PROJECTION`, `syntax/stored_spike.rs`): each document's projected relations
  kept on disk per (document, relation), so a later phase or run reads them instead of rebuilding them. On 591
  files, `struct` and `values` (1,197 builds, load 102–120): a build now costs 9.5 ms a document (tape restore
  0.75, facts 2.1, projection 5.8, text tests 0.85); reading back every relation the phases read, views included,
  2.0 ms (SQLite, zstd and varint decode, the relations remade). 12 MB on disk for the 591 files (about 20 KB a
  document), against the tape cache's 52 MB; writing is 0.5 ms a document. Open: both phases read the per-node
  axes (`rr_nearest_scope`, `rr_inside_*`), computed from each document's tree as asked, so a phase that reads
  them still needs the facts unless the axes run over stored tree columns; joins over stored relations against
  views over facts were not compared.

**Worked around.** `test-budget` reads `conform/`, `data_layer_testing/` and the root's files only. A case list or enum
imported from outside is then open, counted as 1.

**Closure:** the walk computed only for the references a rule asks about (demand), or `qn` without the walk for
names no import binds; a run of `sym_of` over the data layer within the run and memory budgets.

## O-git-tracked. `git_tracked` is declared and never made

**Status:** closed (2026-11-08, `3cb7d0d0a`). `@git_tracked(path)` is a functor over the index phase 0's Git
provider reads (`strings/git.rs`), read again when the index file's digest moves; the test stages an add and reads
it again. Decision 0095 §6.

## O-git-merge-base. `git_rev` is declared and never made; `--base` is the caller's

**Status:** closed (2026-11-08, `3cb7d0d0a`). `git_rev("head", …)` and `git_rev("base", …)`, the merge base of
`HEAD` and `--base` by gix's `merge_base`, are made when a rule reads `git_rev`; `at="base"` reads at the merge base;
the whole-run cutoff records both as a read (`git_rev`, with the index as `git_index`) and checks them again. A bad
`--base` gives `input_error("git_rev", "unreadable", …)`. Decision 0095 §6.


## O-stored-patterns. A stored prepared program lost its AST patterns, and its memo kept the result (fixed)

**Seen (2026-11-08)** as `pattern_literals_match_and_bind_their_metavariables` failing on a warm shared tape cache
and passing on an empty one. Every finding was `Plain`, none `Cut`: the run had no `ast_match` rows.

- `program_facts` writes a program's `rx_spans` and not its `ast_patterns`. A prepared program read back from the store
  (`prepared::lookup`) therefore asks the batch for no pattern matches.
- A run that reuses the prepared program and finds no file-local memo row makes the document's facts without
  `ast_match`, and memoises that result. `program_digest`, the memo's key, did not include the patterns, so the result
  matched the key of a correct run, and later runs read it.
- To reproduce on an empty cache: run the test, delete `facts_results` rows with `fact_schema = 5`, run it again.

**Fix:** `ast_pattern(p, s)` in the program facts, read back into `ast_patterns`
(`ast_patterns_read_back_as_written`). `program_digest` now also hashes `rx_spans` and `ast_patterns`.

**Still open:** `Program::answers` has no relation in the program facts either. Check whether a stored prepared
program needs it. **Closure:** every `Program` field a run reads after `prepared::lookup` either round-trips through
the program facts or is derived from what does, and a test compares a read-back program against the original.

## O-replay-intermittent. The replay gate gave 3988 248 243 once

**Seen (2026-11-08).** `converted_rules_give_the_recorded_findings` gave `3988 248 243 0` once and `3988 303 298 0`
on the next run, with the same code. Suspect: the shared default tape cache (O146), which other sessions write.
After the merges of 2026-11-08 (`0078ba091`, with the collection rules), 2026-11-09 00:15: `3988 303 298 0`, 55.6 s
CPU (49.9 user + 5.7 sys), 56 s wall, load 15–20.
**Closure:** the cause found, or twenty runs on an isolated store all `3988 303 298 0`.

## O-metal-fact-rows. Metal tests facts the host found

**Status:** open (2026-11-09, RER-131; deferred by the user). On Metal, a clause's facts are found on the host before
any pass (`fact_rows` in `transducer/packed/compile/matcher.rs`'s `metal`: `Fact::found` per tape, the `tokens` facts'
regexes over each token text included), and the device only tests the rows it is handed (`Pass::Fact`, the pattern
kernel's `FACT` case). That is host evaluation on the Metal path, which is not allowed: what Metal's kernels answer,
the device computes. It is the one entry `metal_host_guard_tests::REMAINING` still lists.
**Why it matters:** the host's facts run single-file on the CPU before the command buffer, in the Metal run's critical
path; and a fact Metal cannot find is a fact Metal does not own.
**Closure:** facts found by kernels (their text tests by chunked DFA, as string matching is everywhere else), `fact_rows`
gone from `metal`, and the entry removed from `REMAINING`; S7's keys equal on all four backends, full and named-only.

## O-metal-root-programs. After Metal labels a batch, the wave runs the root programs on the host

**Status:** open (2026-11-09, RER-131; deferred by the user). `LoweredWave::labelled_metal` hands the wave the labels
as views of its tapes, and `demand(batch)` then runs each clause's root program on the host for its results
(captures, spans), which a label keeps only as bits. The root walks Metal does run (`root_records_*`, `RootsFound`)
cover a program that is a micro walk of at most 32 frames; any other, a walk that overflowed, or records past the
buffer, the host walks too. The guard does not see this yet: it is in the wave, not the Metal entry points.
**Why it matters:** same rule as O-metal-fact-rows, and the host's root runs are the rest of the CPU a Metal audit
spends after the device.
**Closure:** every root program's results made on the device (captures and spans in the root records, every depth
by a device stack, overflow as a rerun on the device), `demand` on Metal reading them only; the wave's Metal path
added to `metal_host_guard_tests`' scan.

## O-parse-single-splat. A list of one `*` element loses its `list_splat` node

**Seen (2026-11-09).** In `A = [*range(4)]` the list's one child is a `call` whose text is `*range(4)`: no
`list_splat` node. `[1, *range(2, 5)]` has its `list_splat`. `python/lengths` then counts `[*range(4)]` as one
element. Not yet traced (parser or tape); the `values` fixture uses `[0, *range(4)]`.

**Closure:** `[*x]` and `(*x,)` keep their `list_splat` node, with a test.

## O-factorize-nothing. The factorized-store test's program hands no export on as its parts

**Seen (2026-11-10).** `an_export_handed_on_as_its_parts_finds_the_same_and_a_changed_choice_misses_the_memo`
fails on every run. With factorizing on, `factorize_exports` hands on no part (`factorized 0`). The run's
file-local part then has the same plan as the run with `factorize: off`, so it finds that run's two memo entries.
This does not depend on how the memo is stored: the memo misses only when the program differs. In the lowered
program, `uses` is not file-local, because it reads `within`, which the split no longer counts as file-local.
No rule reads `encl` any more. `bound` and `defined` are refused because no rule of theirs factors
(`plan 0 of 1`). The test is ignored with this item.

**Closure:** find which change made `within` (and so `uses`) cross-file, and whether that change is right.
Then give the test a program whose export factorizes, or restore the split, and stop ignoring the test.

## O-suite-five. Five engine tests fail in files the merges of 2026-11-10 did not touch

**Seen (2026-11-10).** These five tests fail in the full engine suite after the merge of
`More-ergonomic-rule-syntax` at 96d4720ab, and fail on their own too:

| Test | Failure |
|---|---|
| `logic::simplify::tests::an_existence_count_is_a_negation` | one existence count rewritten, not two |
| `sql_executor::tests::scaling::walks_with_negative_and_aggregate_seed_and_output_match_fresh_runs` | refreshed strata: 0, not 2 |
| `sql_executor::tests::recovery::original_cpython_programs_use_private_projections_and_bounded_join_work` | 1755 against 1841 |
| `transducer::packed::backend_parity_tests::the_verifier_and_the_metal_interpreter_share_one_opcode_set` | tier 2's words lack 83 |
| `transducer::packed::compile_matcher_tests::the_gate_catches_a_widened_walk` | only 2 widened clauses differ |

Neither the merge nor this branch's memo and frames work changes these tests' files or what they run. Five other
failures in that run need the pack fixtures, which come from S3 (`fixtures:pull`) and are not in this worktree:
`python_shortcuts_change_no_pack_result_f811`, `…_f821`, `…_f841`, `reproduces_sqlfluff_rf01_and_al05` and
`vue::view::tests::corpus`.

**Closure:** each of the five passes on the hub, or its owner fixes it or ignores it with a reason.

**Closed (2026-11-10).** After the merge of `More-ergonomic-rule-syntax` at b9e20bfe9, the full suite gave 3,064
passed and 6 failed. Four of the five pass now. The fifth, `the_gate_catches_a_widened_walk`, failed in the suite
and passes on its own, so the suite's load made it fail. The pack fixtures came from `236fc98d2~1` with
`git archive`, outside the index. With them, the other five failures pass.

## O148. Measured key domains price joins apart from T

Commit 2ab61d7b4 ("Plan delta joins from measured key domains before expansion") changes the estimate on purpose:
a binding's distinct values are capped by the rows its constant filters leave, and containment applies to every
relation, not only to the component being solved. That moves T's recorded plans under T's numbering:

```text
// workload plan 16: estimated work 417 (T) → 412 (port)
bound_in($f, $x)  kind($f, "function_definition")  []  412
// calls plan 54: a different anchor
classmeth($f)  builtin_ref($e, "classmethod")   (T: dexpr($f, $e))
```

About 20 lines of `workload`, `workload_demand` and `calls` differ. `keeps`'s copy-pricing test flips with the same
estimate: the copy is now priced above the reader, so it is no longer inlined.

The three tests are ignored with this item: `t_programs_plan_and_run_like_t`,
`t_programs_over_atoms_plan_and_run_like_t` and `observed_selectivities_price_a_copy_that_never_ran`.

**Closure:** decide whether the port keeps T's estimates where T's plans are the test (and the new estimate only
where T has no plan), or takes the new estimate and records new goldens. Then stop ignoring the tests.

**Closed (2026-11-11): the new estimate, with goldens of plans measured to do no more work.** The user's call was
the new estimate, on condition that the new plans are shown to be better, not just recorded. Measured with
`o148_plans_and_their_work` (each statement's SQLite VM steps, deterministic) on a build at 2ab61d7b4's parent,
whose plans are T's goldens exactly, against the hub:

| Program | VM steps, T's plans | VM steps, the new plans | |
|---|---|---|---|
| `workload` | 55.75 M | 55.37 M | 0.993x |
| `workload_demand` | 5.68 M | 5.46 M | 0.962x |
| `calls` | 10.91 M | 10.38 M | 0.951x |

Every output equals T's in both. By head, the plans that changed order do no more work: `classmeth` 82 → 35,
`pv` 527 k → 514 k, `attr_open` 21.6 k → 20.2 k, `vopen` 1.71 M → 1.25 M, `returns` 1.07 M → 0.70 M, `lookup`
(demand) 1.72 M → 1.51 M; the largest increase is `decl` +0.7% (3,070 → 3,093). On O125 steps 1–8 (the native engine,
the estimate switched alone): join work 0.94x on the full runs and 0.999x on the session steps, instructions retired
within ±1% across three rounds (the user CPU moved with the machine's load, 36–79). `keeps`'s copy-pricing test
flipped because the new estimate gets its case right from the start: by the test's own selectivities the copy is
about 66 k rows against 18 k stored, and `ctx` now stays stored without needing the observed side (the test says
so; `the_side_that_ran_is_believed` keeps `weigh`'s correction covered). The three tests run again; the goldens
are the new plans; `scripts/profiling/o148/compare.py` compares two builds' measurements.

## O-wave-b602-nosec. The wave misses B602 on a line whose `# nosec` names another code

**Seen (2026-11-10).** `the_findings_are_ws_violations` had not run on any worktree for a while: the bandit and
ruff fixtures were missing since the pack test data moved to S3. With them restored (from `236fc98d2~1`, checked
against each tool's `test-data.lock` by files and bytes), the wave finds 16 keys and W finds 17. The one only W
finds is `bandit-b602-subprocess-popen-with-shell-equals-true` on `f006.py` line 10,
`subprocess.Popen('/bin/ls *', shell=True) # type: ... # nosec B607 # noqa: E501 ; pylint: disable=line-too-long`.
Bandit's `nosec B607` suppresses only B607, so B602 stands. The same run lowers 13 more clauses than the last
blessed `wave.rir` (B403–B413 imports and five ruff rules), and 140 families changed. Not yet traced: whether the
wave drops B602 for the comment, or the clause's lowering misses this call. The test is ignored with this item.

**Closure:** the wave reports B602 on that line, or the difference is shown to be W's, and the test runs again.

## O-p0-stats-flake. Phase 0 sometimes finds no statistics for `p0_file_t_shape` under parallel tests

Under the test harness's parallel threads, `corpus::base_tests` fails intermittently with `phase 0: no
statistics for p0_file_t_shape.c0`. Each time it is a different test (`fstrings_are_ws_python_fstring_trees`,
`match_group_joined_on_its_match_column`, `a_lines_regex_is_src_line_text_and_match`, and others). It happened in
8 of 20 runs on the pre-merge hub engine, with or without strands (decision 0113), and never with
`--test-threads=1`. The failing model is built in `make_plan` from the caller's given statistics, which have
neither the relation's columns nor a lazy entry. The run comes through `cone_rounds` on a pool thread
(`maintained::run`). So something process-wide is suspected, shared between concurrent runs: a store keyed by
connection address or by path (`""` for in-memory databases). Not yet traced.

**Closure:** the shared state is found and keyed per run, and `base_tests` passes 20 of 20 in parallel.

**Update (2026-11-11), decision 0118.** The same error comes up in `logic::plan::tests::` (for example
`bounded_value_recursion_and_negated_conjunctions_run`): 4 of 10 runs on one store per process, 0 of 20 on a store
per test. So the shared state is the shared store. A test's run read the statistics, plans and choices other tests'
runs had kept there. `base_tests` is compiled only by `rerail-device-metal`'s test binary, which does not build
(O-device-metal-tests), so its 20 runs are still to do.

## O-wave-test-shared. `closures_of_one_level_launch_in_one_wave` fails beside other tests

Run with the rest of `logic::plan::tests` on the harness's parallel threads, this test sometimes sees its level's
three closures go out as more than one wave of three launches. Alone, or with `--test-threads=1`, it passes, and
the run has no requests, so decision 0114's suspension on them is not on its path. The device wave
(`lanes::wave`) is process-wide, so a concurrent test's launches, or its flush, are suspected of splitting this
test's wave.

**Closure:** a wave belongs to its run (or the test counts only its own launches), and the test passes 20 of 20
beside the others.

## O-cone-libraries. Since phase 1 loads the libraries, the import closure is near all pairs of 4,249 documents

c90b3e39b makes the libraries an import resolves to into documents of every later phase. components.rr seeds
`rr_cx_reach(d, d)` from every document (`rr_cx_own`), and typeshed's stubs import each other in cycles. So
on the 54-file Python corpus, `rr_cx_reach` holds 6,347,637 rows over 4,249 documents (2026-11-11, hub 03125d2d1).
Before the merge it was about 4 k. Through it:
- `rr_cx_sees__sup0`'s seed and the statistics of `rr_cx_reach` cost about 7 s and 4.3 s a run.
- The kept cross-file part holds one relation of 6.7 M rows (147 MB of values, 34 MB of levels, 41 MB of sets and
  indexes).
- Read back from the frames, the part is 268 MB, past the 256 MB budget (`RERAIL_CROSS_RETAIN_MB`). A warm run with
  no input changed then keeps 70 relations and derives 215 of 481 strata again, run after run. A cold run had kept
  364 relations in 145.7 MB.

The closure is the same as its condensation: `d` reaches `e` iff `d`'s component reaches `e`'s
(`rr_cx_component`). A reach over components, with `rr_cx_reach` and `rr_cx_sees` asked as memberships through
it, holds the component DAG's closure instead of the documents' pairs.

**Closure:** a warm run of the 54-file P3 corpus derives no stratum again with no input changed; cold CPU is back
near 66.6 s (t32); findings unchanged.

**Status (2026-11-11): mostly closed by decision 0119** (the reach index). `rr_cx_reach` is 208 KB of bitmaps,
not 6.3 M rows. A warm run with nothing changed derives 2 of 480 strata again: the index's own two, rebuilt in
0.1 s and never kept. It keeps 587 relations in 29.1 MB, and findings agree (4,815). Still open: the cold-CPU
check against 66.6 s, measured with the per-file memo warm (0119 **Measured**).

## O-logic-tests-link. rerail-logic's test binary checks but does not link: the grammars are the engine's

`cargo test -p rerail-logic --lib --no-run` compiles (01db3167f) but fails to link: `_tree_sitter_python`,
`_tree_sitter_javascript`, `_tree_sitter_typescript`, `_tree_sitter_tsx`, `_tree_sitter_sql` and
`_tree_sitter_vue` are undefined. The language registry (rerail-lang's build.rs) declares them, but only the
engine's build.rs compiles the vendored grammars. So the corpus tests that stay in logic (memo, retain's frame
tests, kept_strata, phases, reuse, cache_tests, prepared) run nowhere.

**Closure:** rerail-lang's build script compiles the vendored grammars it declares (the engine's then does not),
and logic's test binary links and runs.

## O-constants-strict. Under `RERAIL_SQL_STRICT=1` a corpus run stops at `constant_rows`'s set-up

`corpus/constants.rs:214` creates `constant_rows` on the run's connection, a write outside the waves that
decision 0104 refuses. So the P3 benchmark aborts in strict mode before its first wave (hub 03125d2d1).

**Closure:** the table is made by the store's schema or written through the store writer; the 54-file P3 run
passes in strict mode.

**Closed (2026-11-11).** Four things stood in the way, each fixed:
- The constants are a run record of phase 0's observer (`constants:<key>`), read with its records before the run and
  written behind it by its writer. The `.phases` file is gone.
- The keeper's files changed by key (a run numbers its documents afresh), not only by content, are found before the
  first wave. One asked and not found is held as not found, so the waves do not ask SQLite for it again.
- `@cat` inside a computed head term is a native `Cat` step (`struct.rr`'s `sym(f, cat(@dot(q, x), cat("#", …)))`).
- `components` runs on the native engine over the relations it holds. The constants' own run outputs the constants
  alone, not `rr_cx_component`.

The 54-file P3 run passes strict, 4,815 agree.
**Closed (2026-11-11): decision 0118.** With a store per test, the test passed beside the rest of
`logic::plan::tests::` in 20 of 20 runs; with one store per process it failed 2 of 10. The split wave came from the
shared store, not the device's lanes: a test's run took plans and choices that other tests' runs had kept there.

## O-split-logic-tests. Six tests of `rerail-logic` read crates above it, and are compiled nowhere

After the crate split, `rerail-logic`'s test target did not build: its tests named the engine's modules (the
`prefilter` test helpers, `tools::flatten`, `sql_executor`, `waves::policies`, `model::FileInput`, the device's
`transducer::packed` rows). The helpers had a home below (`fixtures::temp`, `logic::settings::program_text`,
`ContentStore::open`), and those calls now use it. The corpus runner's tests that parse files (the cone, the shared
relations, the cross-file frames) need the languages, which the crates above register: they moved to
`corpus/tests.rs`, which the engine's facade of `logic::plan::corpus` compiles. The ones that parse nothing stay in
logic. Five tests need a crate above, so they are gated out (`#[cfg(any())]`, marked `O-split-logic-tests`) until
the split hosts them in the engine:

| Test | Reads |
|---|---|
| `corpus::matchers::tests::any_arms_binding_one_name_lower` | `waves::policies` |
| `corpus::matchers::tests::a_matcher_relaxed_where_it_only_widens` | `waves::policies` |
| `corpus::matchers::tests::a_nested_file_path_is_the_documents` | `tools::run_policies` |
| `native_demand::tests::retained_semantic_results_support_sql_outputs_and_isolate_targets_and_runs` | `model::FileInput` |
| `native::setfix::lab::tests` (the relational lab, ignored) | `transducer::packed` (Metal rows) |

`test-hosts.tsv` already names `logic::native::setfix::lab::tests` as the engine's, but the generator does not move
an inline test module, and the engine mounts none for it.

**Found on the hub (2026-11-11), after the More-ergonomic and Cratification merges.** `rerail-logic`'s test
target built only together with the engine: nothing turned on its own `test-hooks` (16 errors: `tempfile`,
`fixtures`, `drop_unread_core`). It now has the engine's self dev-dependency (`rerail-logic = { path = ".",
features = ["test-hooks"] }`, the same in `rerail-device-metal`), and `rerail-lang`'s `test-hooks` turns on
`rerail-store`'s and `rerail-compiler`'s, which no crate's list named. Built alone, 269 pass and 41 fail;
33 once `corpus::tests` went to the engine (More-ergonomic `b2bfa28d6`).
Every failing test reads what the engine registers as it loads, through a
registry that no static read shows, so `hosts.py` leaves them in `rerail-logic`:

- the Rr language pack (`no language registered for Rr`, from `rr::stored_statements`);
- then the language crates' `.rr` libraries (`#include "python/modules.rr": no such library`) and the Python pack;
- the `to_precision` function (`TO_PRECISION`, set by the ECMA crate).

**Hosted in the engine (2026-11-11).** A test that reads through a load-time registry reads the crate that
registers it, so these went to the engine; copying `install_languages` into `rerail-logic` would have been a second
loader. Each module's facade in the engine (`native/src/code_rule_dsl/mod.rs`, `waves.rs`) mounts the test file,
and what the tests read became `pub`:

- whole modules: `liveness::tests`, `corpus::{cache_tests, base_tests}`, `phase0::tests`, `phases::tests`,
  `prepared::{tests, identity_tests}`, `reuse::tests`, `session::tests`, `profile::report::tests`,
  `typing::phase_tests`, `waves::owner::tests`, `residency::slabs::tests` and
  `metal_program_backend::tape_tests` (inline ones first moved to files);
- the one test of a module that needs a language, in `engine_tests.rs` beside the module: `compose`,
  `native_demand`, `strings` (`to_precision`), `metal_backend` (the launch budget) and `typing::file_local`.

Their rule paths are the engine's manifest's (`CARGO_MANIFEST_DIR` is `native/`). The device tests stay in
`rerail-device-metal`, which now registers Metal as its own test binary loads (`device_metal::test_load`), as the
engine does. The launch-guard test cut each file at `#[cfg(test)]\nmod `, which the split's `pub mod` test modules
no longer match, so it read its own list of forbidden calls; it cuts at either. Results: `rerail-logic` 262 pass,
0 fail; the 83 hosted tests pass in the engine; `rerail-device-metal` 20 pass, 2 fail (below).

**Still open:** `setfix::device::tests::{text_functions_on_the_device_are_the_cpus,
text_tests_on_the_device_are_the_cpus}` ran nowhere on the hub until now. In the cases with a 60 × 2000 cross step
the device reports fewer bindings at that step than the CPU (79,153 against 120,000); every final head count is the
CPU's. It is Native-and-GPU-strata's to read: either the device drops bindings it should not, or it counts them
after a filter the CPU applies one step later.

**Closure:** each runs in the engine's test binary (or in a crate that has what it reads), and the gates go.

## O-test-store-cost. A test's own store costs 5–21% more instructions per suite

**Seen (2026-11-11), decision 0118.** Against one store per process: `logic::rr::` 740 G instructions against 668 G,
and 784 MB against 686 MB; `logic::plan::tests::` 179 G against 148 G, and 329 MB against 240 MB;
`logic::plan::corpus::tests::` 194 G against 185 G. That is about 0.65 G per test, whatever the template holds. A
template of the core program alone costs as much, and needs less memory (669 MB for `logic::rr::`). The cost is a
test's first run on a new store: it opens the store, makes its observer store, and finds the process's caches of the
store cold (by its path: `setfix`'s `SYMS`, `STORED`, `INPUTS`; the frames read before the first wave). That warmth
is the sharing between tests that the decision removes.

**Closure:** each first-run step on a new store is measured, and any that is not proportional to what the run reads
is cut. The memory per concurrent test is judged against the 1 GB budget. Which template, the library's or the core
program's, is decided by the user.

## O-device-metal-tests. `rerail-device-metal`'s test target does not build

**Seen (2026-11-11).** `cargo test -p rerail-device-metal --lib --features test-hooks` fails in `rerail-transducer`:
`content_store::positional_search` and the store's bulk search methods are not there under that build (11 errors).
Without `test-hooks` its own facade fails (`fixtures`, `p_table`, `drop_unread_core`). `corpus::base_tests` is
compiled only there.

**Closure:** the target builds, and `base_tests` runs (O-p0-stats-flake's 20 runs).

## O-phase0-store-state. Phase 0's include graph depends on what the store already holds

Each test now starts from a copy of a store template that already holds the
library's facts (decision 0118). In such a store, two runs of phase 0 on the
same seed disagree:

```text
seed: the text of python/conditions.rr (its #include lines)
first run   (graph):  python/env.rr includes core.rr at -1 (the implicit include)
second run  (derive): python/env.rr has no includes; the absent candidates differ
module digests: equal
```

On an empty store the two runs agree, so the derivation depends on the facts
left in the store by earlier runs. That is maintenance of phase 0's own
relations, not the observer: the module digests match.

`bootstrap_resolves_as_the_builtin_reader_does` is ignored until this is fixed.
It passes with `RERAIL_TAPE_CACHE` pointing at an empty store. Next step: find
whether the first or the second run is wrong (compare both with a run on an
empty store), then fix the rule or the maintenance that loses the edge.

## O-replan-churn. Every edit replans about 3,350 rules whose kept order "grew over 4x" (closed)

Measured on the P3 bench (54 files of `dl-sub40`, `calls_p3`, strict) on 2026-11-12,
over two edits, each run in a new process:

```text
first edit:  planned (first sight) x1190; replanned, work per node grew over 4x, by over 10000 rows x3330
second edit: planned (first sight) x186;  replanned, work per node grew over 4x, by over 10000 rows x3350
```

The plans are kept across processes (0116 step 2), but almost the same 3,350 are
searched again on every edit. That is 0.87 s of join-order search and 0.77 s of
strata planning in the second edit (profiled). A kept order priced at one scale
and replanned at another, run after run, is churn, not a size that moved. Next
step: print one such rule's kept and new work per node (`RERAIL_PLAN_CACHE_WHY`),
and find whether the replan's result is kept under the key the next run reads.

**Closed (2026-11-12).** The cause was not work growth. It came in three parts.

1. **Distributions compared as they were.** `RERAIL_PLAN_CACHE_WHY` printed "distributions near false" for each
   replan. A rule planned for a chunk (22,146 nodes) and for the cross part (61,997) shares one kept plan (0116
   keys the scale by powers of 4), and its distinct values were compared as they were. So each scale replanned
   what the other had kept. Now `near_distributions` takes an entry as near either as it was (a column of few
   values) or in proportion to the nodes (a column of node ids).
2. **Small relations whose distinct values differ by part.** The 935 replans left were rules whose re-priced work
   was 1–600 rows (`rr_embed_req`, `pre_832`, `qann`, `callable_ann`). A search can save at most that work. So a
   re-priced kept order whose whole work is under `REPLAN_ROWS` is kept, whatever its distributions.
3. **First sights at a scale not stored.** An edit's chunk is a bucket or two from the scale its plans were stored
   at, so `RunPlans::find` found none and searched from nothing (638 per edit). It now takes the plan stored at
   the nearest scale. That plan is re-priced before it is kept, so this is exact as before.

The profile's replan line now names its cause (driver not live, distributions moved, or work grew), where it said
"work per node grew" for all of them.

Measured on the third edit of a build (profiled; 2,987 agree each run):

| Per edit | Before | Distributions in proportion | All three |
|---|---|---|---|
| Replanned | 3,350 | 935 | 18 (distributions moved, work 11,600–62,000) |
| First sight | 186 | 638 | 0 |
| Kept, re-priced | 228 | 2,710 | 3,468 |
| Join-order search | 0.87 s | 0.77 s | 0.25 s |
| Re-price + strata + replan spans | | 0.33 s | 0.23 s |

## O-live-replan. Live replans search again for orders they already have (closed)

Profiled on the P3 bench (third edit of a build, 2,987 agree), after O-replan-churn closed. Planning left per
edit, as own CPU of spans (profiled):

| Where | Calls | CPU |
|---|---|---|
| Live replans between waves (`plan_rule_live`): searches | 1,162 | 0.40 s |
| Their EXPLAIN records | 1,162 | 0.085 s |
| Their re-prices | 1,920 | 0.075 s |
| The replan step's own work | 3,497 | 0.07–0.09 s |
| Plan time: re-prices | 3,744 | 0.11 s |
| Plan time: searches | 211 | 0.02 s |
| Reprobes after an order changed | 1,255 | 0.05 s |

Every live search had one cause: "the distributions it reads moved". Its guard compares distributions as they
are, the fault O-replan-churn fixed at plan time. Most of the searched rules are in one recursive stratum of 99
relations, searched again wave after wave. The costliest is `fgp` (16 literals, about 8 ms a search). Another,
`sym`, flips between two size vectors (distinct values 1, then 3) on alternate waves, so each search overwrites
the one priced entry that the next wave needed.

What the searches found, against the kept order re-priced:

| Re-priced work | Same order | Other order, saving < 100 rows | Saving 100–10,000 rows |
|---|---|---|---|
| Under `REPLAN_ROWS` (1,014 searches, 0.40 s) | 886 | 105 | 23 |
| Over it, distributions moved (83, 0.012 s) | 81 | 0 | 2 |
| One positive literal (55, 0.002 s) | 55 | 0 | 0 |
| Near in proportion to the nodes (10) | 10 | 0 | 0 |

So 89% of the live searches return the order they had.

Opportunities, largest first:
1. **Keep a re-priced live order whose work is under what a search costs.** This is the plan-time rule
   (O-replan-churn, part 2) applied to `plan_rule_live`. A search is about 0.4 ms, roughly 600 rows of join work
   at this bench's 0.64 µs a row, and grows with the literals (`fgp`: 8 ms). Keeping the order when its
   re-priced work is under the search's own cost removes about 1,000 searches (0.40 s plus most of the 0.085 s
   of EXPLAIN). It still searches the 23 that saved 100–10,000 rows. A flat `REPLAN_ROWS` would also skip those
   23, giving up at most about 150,000 rows over the run's waves.
2. **Build a live search's EXPLAIN only when its order is taken.** The caller drops the result when
   `same_order` holds, so 0.085 s of EXPLAIN records are discarded when the order did not change.
3. **Distributions in proportion, in the live guard** (`near_distributions`, as at plan time). This matters
   only for the large relations once item 1 is in: 10 searches here.
4. **Two priced states per live key.** This is for sizes that alternate between waves (`sym`). After item 1 it
   is needed only when the work is large.
5. **No search for a body with one positive literal.** Its order is forced: 55 searches, 0.002 s. Not worth it
   alone.

What is not an opportunity: reprobes (0.05 s with the indexes they open) and the plan-time re-prices (30 µs
each, 0.11 s; they are what stands in for a search). The index builds in `open: joins` (0.36 s) are the joins'
own cost, not replanning's.

**Closed (2026-11-12, decision 0121 §1).** All five are built. A live replan keeps the re-priced order when its work
is under what a search costs (`search_rows`), or when it has one positive atom. Its distributions are compared in
proportion, the state before the last is kept, and a live search's EXPLAIN is made only when its order is taken.
Live searches went from 1,162 (0.40 s) to 558 (0.04 s) per edit. Planning fell from 4.5% to 3.1% of the edit's
CPU, and the join work is the same in each later run (8,766,423). 0121 also makes the cold search a third
cheaper, with the same plans, and sizes greedy-first, classifiers and bulk planning.

## O-cutoff-phases. The whole-run cutoff never applies to a phased program

`RERAIL_REUSE_TRACE` on an unchanged P3 run:

```text
changes: not comparable: declared_ret false, phases ["(\"sources\", …)"]
whole-run cutoff not applied: a consulted file or read changed
whole-run cutoff not applied: not recorded: 54 consulted, other phases ["struct", "types"]
```

`corpus::changes` refuses any program that reads an earlier phase, and the
outermost run is not recorded because it read other phases. So an unchanged run
of `calls_p3` costs 5.5 s of CPU, where a single-phase program costs 0.71 s
(b8819272f). The phases are derived from the same consulted documents and
requests, so a record over the union of every phase's consulted things would be
exact. Next step: record the outermost run with its phases' consulted things,
and check each phase's requests in `changes` as for a phase-free program.

**Closed (2026-11-12).**
- `reuse::record` keeps a run of any phase, with every phase's consulted things, one `doc` entry a document.
- `rule_files` names every phase program a program reads, transitively, with its includes (`phases::phase_files`).
- `changes` compares only the workspace's documents with the file list; a library document (an absolute path) is
  checked by its digest.
- On the P3 bench, an unchanged run took the cutoff: 1.47 s user + 0.17 s sys, against 5.5 + 1.0 s, and 2,987
  agree. An edit of one file was not cut off ("1 documents changed").

## O-kept-inputs-stale. A run in a new process keeps an edited file's old rows (kept inputs, 0117)

`a_session_derives_the_reach_index_again_when_cycles_and_connections_change` fails after the hub merge
(d0f1297e5), on every run, with or without decision 0120's kept statistics, and on an empty store
(`TEST_TAPE_CACHE`):

```text
edit: c.py "from a import z\nfrom d import v\n" -> "from d import v\n"  (break the first cycle)
session run:  CYC, INCYC, SEESE at c.py:1                    (right: c.py has one line)
full run:     CYC, INCYC, SEESE at c.py:1 and at c.py:2      (c.py's old line 2)
```

The full run is the one without a session (`session::full`), which reads the kept part from the
store. With `RERAIL_KEPT_INPUTS=off` the test passes, so the stale rows come from 0117's kept-inputs
route. The whole-run cutoff is not involved: the full runs don't ask for it. `RERAIL_CROSS_VERIFY` finds
"0 relations differ" in the inputs, so the stale rows come in after them, from rows kept by owner
or from the kept part's derived frames. The statistics keeper does count c.py as changed, under both
memo keys. The test passed once right after the merge, under heavy load, so the route may depend on timing
(write-behind, 0114). So does `the_reach_index_is_derived_again_when_cycles_and_connections_change_in_a_new_process`, on the same edit, against
a run with an empty store; it too passes with `RERAIL_KEPT_INPUTS=off`. Both tests are ignored with this item's name. Next step: dump the full run's
`impmod` (or the reach index's edge relation) rows for c.py with and without kept inputs, and find
the owner range that was not marked dead.

**Closed (2026-11-12).** `RERAIL_KEPT_INPUTS_TRACE` showed the cause. A chunk evaluates its unchanged
files with the changed one (e.py with c.py), so this run's rows of an input hold e.py's too. e.py has no old entry,
so `patch_owners` took its kept rows plus this run's: twice. The same sum kept rows a changed file's old entry
did not hold (c.py's old entry had no `line` rows), and those included `line(…, 2)`. Now a real
document's rows come from its own file alone (`memo::document_of`), so for every document this run evaluated,
its rows are this run's. The count arithmetic is kept for value documents, which several files hold. Both tests
pass again.
