# Open items ledger

Status: live, started 2026-10-27. Each item says what is open, why it matters, how it will be
closed or validated, and where it stands. Items are added at the phase they are found and are
never deleted: a closed item keeps its row with the commit that closed it.

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
| O41 | Phase 10 | Bounded cross-file rounds (k = 2) leave calls.rdl short of its fixpoint on the vtab corpus (a third file crossing adds 8 of 5,112 hop rows); the unconverged frontier is reported but not yet marked pending | open |
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
  - the per-document cutoff, since each round still reruns the whole cross-file part.

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
`code {c} in "re"`, `tag` and `suppress` lines; `match` captures, arithmetic, the string functions, Soufflé's
`cat`/`max`/`min`/`to_number`/`to_float`/`%`, floats; `sum` and `join` (and aggregates over terms, inside
terms, and compared with each other); `.input` with parameters; `within`, `within_stop`, `nearest` (a named
ancestor negated too), `inside` and `has` over kind sets and over node-valued sets; `precedes_stop`; negated
conjunctions (a helper relation); `must, may`; `unset`; recursion that makes values bounded by the tree, a
count, a shrinking text or a function between rows (`front::cycles`); `dfs_first` and `dfs_first_memo`, computed natively over the edge relation (`logic::native::dfs_first`), so `data-layer/lazy-architecture.rr` lowers (944 rules). Still refused by name (census of
2026-11-06, clauses alone): `ordered by` (37 clauses: eslint-one-var, the owner-code sketch), `@place` as a
value outside a `.violation`'s head (3 clauses, 12 files), `~` patterns
(1), `mean`, `=> edits`. `UnboundedValueCycle` still stops 15 library clauses (73 files) whose bound is not
one of those (`hex_place`, `norm`/`sci`, `unescape`, `slot_walk` over tokens, `b005_char`, `era_re`).
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

Hand-written rules read relations the engine does not make yet: `parsed` and `string_annotation` (strings
parsed as code) are read as empty, each named in the profile. Inputs read from a file in the repository
(`.input r(IO=json, filename="…")`) are empty when the file is absent; when it exists the run stops,
since reading it is not written. **Closure:** these relations are built natively and the files are read. (`option`, `option_item`, `file` and
`package*` are made now, decision 0046; `parsed` and `string_annotation` are O80.) 2026-11-07: `parsed` is
made (decision 0055); `string_annotation` is still empty. 2026-11-07, after the data-layer engine merge
(ca930d974): repository files are read (`corpus::inputs`: JSON, YAML, JSON Lines, TSV, lines, `dir=` inputs, and
`input_error` rows for missing or malformed files). The revision and stamped-artifact readers are still not written.

## O77. Lowering shortcuts in `.rr`

- A `match` capture is computed per row (`rx_group`, one cached regex per pattern), not as a chunk-at-a-time
  text test. Over a `source` or `src_line` span, captures are spans (`rx_span`, decision 0054); on a node they
  are still symbols (O80).
- `unset(x)` is the id -1. Its text is "" (`text(x, t)` of it lowers to `t = ""`), and `!unset(x)` is
  `x != -1` (decision 0046). Inside an aggregate, `nearest` or `ast` it is still refused.
- `within(n, a)` where nothing else gives `n` lists the nodes below `a` by following `child` down from the
  `a`'s the rule gives (named children only), not by reading the id range `a + 1 ..= a + size`; the native
  path has no range read yet.
- `earlier` and `visit_index` are preorder, not CPython's ast order.
- The value-cycle check (`front::cycles`) accepts the bounds decision 0046 lists; a bound it does not know is
  refused, never run.
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

- `parsed` and `string_annotation` (O76) are still read as empty. A string parsed as code needs its tree grafted
  into the batch's node space, on demand (rrlab answers requests in rounds); not built. 2026-11-07: `parsed`
  is made, as guest documents of the chunk asked for in rounds (decision 0055); `string_annotation` is not.
- `match_all`, and `match` captures as spans, are made over `source` and `src_line` spans (`rx_span`, one scan
  per file and pattern) and over symbols (a native relation): decision 0054. Over a node's own text, `match_all`
  is refused by name and `match` captures are symbols; a scan per node is what 0041/0042 forbid.
  (`docof` of a span is made: the lowering reads `core.rr`'s `docof` as an input.)
- `text` of a node longer than 256 bytes that is not token-like has no row; a test on it is 0042 §3's file scan.
- `syntax_error` for Python is CPython's grammar's verdict, from Ruff's parser (`ruff_python_parser` 0.0.10, the
  last on the pinned toolchain; 2026-11-07), not tree-sitter's; compile-time errors CPython's compiler raises after
  parsing (`return` outside a function) are not checked, and its cost on the full corpus is not measured yet. A
  document with one is withheld in `core.rr`'s shape (no facts but `doc`, its path and `syntax_error`), and `.on syntax_error report CODE
  [at line 1]` and `.on input_error report CODE` report (2026-11-07); T's shape keeps every document.
- `static_value` is `value`: file-local constants are not followed.
- `tsconfig*`, `package_export`, `git_*`, `signed_list` and `list_len` are not read. `input_error` has its
  `missing` rows (a `.input` file that is absent, decision 0054); the other kinds wait on reading files (O76).
- `earlier` and `visit_index` are preorder, not CPython's ast order (as O77).
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

**Closure:** the per-document cutoff (O41) for the rounds: round r + 1 runs only the documents that read a row
that changed in round r, and keeps the others' rows. Statistics sampled rather than exact. Then the full
l1–l20 Jedi score and the LEG-083 files.

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

**Closure:** the pack's rules run on the full corpus, within the budgets, with every rule left out named.

## O95. The shared tape cache races

`core::build_cached` writes tapes to one SQLite store in the temp directory, and every run on the machine shares it.
Under parallel tests, a write sometimes fails with `FOREIGN KEY constraint failed`. Guests write many small, equal
texts, so they hit it most. A guest whose cache write fails is parsed without the cache, and the run counts it
(`guests.cache_failed`). Primary files use the same call and can fail the same way. The likely cause is a collector
removing a source between `put_source` and `put_tape`; this is not investigated.

**Closure:** the write is made atomic (source and tape in one transaction, or a collector that cannot remove a
source a writer holds), with a test that runs parallel writers against one store; or the cache is made per run.

## O96. Chunks that read value documents are not memoised

Decision 0058's value documents (`@parse`, `@splice`) belong to no file. A chunk whose outputs hold a row that
locates one cannot attribute it, so the chunk is not memoised (`memo_unattributed`). Attributing a value to the
files whose rows asked for it needs the demand's provenance.

**Closure:** value rows are attributed to the files that asked for them, with a test of a memo hit against a fresh
run; or a measurement shows unmemoised value chunks cost too little to matter.

## O97. A combined run's programs and their guest bounds

Each program of a combined run is bounded by its own `.guests` setting (decision 0058 §1), but all programs share
one batch. No test runs a program with `.guests off` beside one without.

**Closure:** a combined-run test where the two programs see different guests.

## O98. Kind sets across grammars

`nearest`, `inside`, `has` and `within_stop` take kind sets resolved in the run's grammar. A scoped SQL node's
`nearest(n, a, {"select_statement"})` names no SQL kind. The scalar kind and field constants are qualified (0058
§2); the sets are not.

**Closure:** a kind set over a scoped variable is resolved in its grammar, with a test over a SQL guest.

## O99. Guests and hiding at scale

Hiding (decision 0058) is a range test on columns the join already reads. Its cost, and the cost of the guests
and values a policy makes, are not measured on the 9,010-file repository. Global embeds and the matcher path's
guests are still not these documents (O85).

**Closure:** a full-corpus run with guests on and off, in CPU time, with the guests made and the distinct texts
parsed.

