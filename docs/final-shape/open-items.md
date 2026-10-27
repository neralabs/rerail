# Open items ledger

Status: live, started 2026-10-27. Each item says what is open, why it matters, how it will be
closed or validated, and where it stands. Items are added at the phase they are found and are
never deleted: a closed item keeps its row with the commit that closed it.

| Id | Found | Item | Status |
|---|---|---|---|
| O1 | Phase 1 | Crash recovery: pending writes lost in a crash must always be recomputed | **open, validation required** |
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
