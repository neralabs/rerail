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
| O8 | Phase 2 | Tape v9np is not locked | open (conditional) |
| O9 | Phase 2 | Kind histograms are digests only; their rows are not stored | open (Phase 3) |
| O10 | Phase 2 | Core facts of a non-UTF-8 blob, an unknown language or a parse stopped at a limit are `ABSENT` | open |

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

## O8. Tape v9np

05 Phase 2 locks v9np "later (G8, X)", using P `experiments/tape-v8`, and copies it into
`cr/tape/v9np.rs` only when its 4 measurements pass. W is on v8 (v7 plus N's supertype table). Closure:
run the 4 measurements against v8; copy v9np only if they pass.

## O9. Kind histograms are not stored

`#kinds:` derivations record the histogram's digest. The counts themselves are recomputed from the
tape when read. Closure: Phase 3's `file_kind_counts` (the prefilter's access path per file) stores
them, written by the kinds derivation.

## O10. Files without core facts

A path whose blob is not stored, is not UTF-8, has no dialect, or stops the parser at a limit gets
core and kinds outputs of `ABSENT`. That is consistent (an edit that makes it parse rebinds it), but
a rule must treat `ABSENT` as unknown, not as empty. Closure: Phase 8's findings layer reports these
files as Unknown under the loose contract.
