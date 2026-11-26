# 0686: A delta executor that loads itself from one tape

Status: being rebuilt to the agreed design, milestone by milestone; M0 (the clean slate), B1 (operations on
blocks) and B2 (the program trace) done. Design pages: *Bootstrap
from one tape* (https://claude.ai/artifact/7Sc29B3894aE1PBsbhwbqa, cited B§), *Delta-first engine primitives*
(https://claude.ai/artifact/2pcYo27YJWcPwM3msKjoAH, P§) and *The program bistate and the bootstrap*
(https://claude.ai/artifact/28cUPRyFpEhqCnPbj5gKLT). The rebuild plan: *0686: Rebuild Plan*
(https://claude.ai/artifact/N2rAYMAy7mu46wb31BuAaN), with *0686: Steps, Strata, Roll-up*
(https://claude.ai/artifact/KaDH6yw6hgNjrxPRcqe3pc) and *0686: Agreed and Built*
(https://claude.ai/artifact/QUgNHejfZcXaKPxuizrkLK).

## The user's request (2026-11-25)

"I want to begin creating A BRAND NEW EXECUTOR BASED ON THE FOUR DELTA OPERATORS". It:

- "will have ALL compilation / program modification / lowering logic in rerail";
- "will operate ENTIRELY with the operators we have written, no special Rust logic that reaches inside";
- "will have a tiny amount of hardcoded Rust steps, only enough to bootstrap the rest";
- "be very deliberately built a step at a time".

A new `bootstrap-l0.rerail` is parsed to its tape, and the tape is passed to a bootstrap level 1 operation, "written
as a bistate, whose only job is to extract the loads via ast-match operations and turn them into a program, via a
single production input. It will take one tape, no program, one production, and only facts in which are program
globals". Later levels load the core in phases: "Each phase's results will become a program and facts for the next
phase." At the end, a user `.rerail` program is loaded and all its rules compiled and lowered "into the program input
for the actual engine".

On the design page: "I think we may want more levels, but this is broadly correct." On the first level, after a
round on whether it should be a wave with a constant program read by a smaller Rust pass: one fixed Rust pass that
reads the kernel subset and skips that level ("okay, let's do it that way").

## The design

As agreed on the design pages, which stay as agreed; the points they left open are settled below, in the user's
words.

- **K-form, the one program form.** A program is rows of eight relations, each row's first column its module `m`
  (the document it was read from: the partition column): `k_rule(m, rule, head)`, `k_step(m, rule, k, j, kind,
  rel)` (kind `atom`, `not` or `cmp`; a `cmp`'s rel is its operator; steps run in `(k, j)` order, as read at
  `(k, -1)`), `k_arg(m, rule, k, j, i, term)` (`(-1, -1)`: the head), `k_var(m, t, name)`, `k_str(m, t, s)`,
  `k_num(m, t, n)`, `k_wild(m, t)`, and `k_order(m, k)` (a module's stratum: its file's place in the bootstrap,
  `boot(path, k)`; see F2 below). Rule and term ids are tape node ids, local to their module (L9); they renumber on any edit, and the module is
  the grain; across modules only symbols cross.
- **What the executor does with it, fixed** (B§3's five sentences): a rule's steps run in key order; a variable is
  bound at its first occurrence and tested at every later one; an `atom` step joins the rows so far with the
  relation on its bound columns, a `not` step is a semi-join with `neg`, a `cmp` step a filter; a relation is a base
  input, a tape view, an answer, or the head of rules; a module's rules are one stratum, run as a Δfixpoint, strata
  in `k_order`; a rule that reads one document's views runs in that document's single-file wave, any other in the
  cross-file wave. It refuses a `not` or `cmp` with an unbound variable, a head variable no step binds, and a
  stratum that negates its own head, and never rewrites a program.
- **The kernel subset (K0):** clauses with one head; bodies of atoms, negated atoms and comparisons, in order;
  arguments that are variables, strings, numbers or `_`; facts. No arithmetic. A K0 rule's tape is its K-form, read
  node by node.
- **L1, the K0 pass:** one fixed Rust pass over `bootstrap-l0.rerail`'s tape, writing P2's K-form rows and refusing
  anything outside K0. As a delta operator: the tape's bistate in, P2's bistate out, the file its grain; no program
  port.
- **`bootstrap-l0.rerail`:** the pass's meaning as 13 K0 rules (P2's reader), the `k_order` rule, and the `boot` rows of
  the next level's files. Its check: P2 run over `l0`'s own tape gives P2 again, row for row.
- **The chain and the loop:** each level is a Δphase whose program came from the level before; its output's K-form
  relations are the next program and the rest are the next facts; B§8's loop runs it until a phase exports no
  program.
- **The operators:** P§12's `DeltaOp` (bistates in, demand, cache; bistates out, asks), composed by L6; a phase is a
  Δfix per stratum, a Δfix its Δwave repeated until the output is the identity, a wave the single-file waves by L7
  and the cross-file wave; `Δanswer` answers loads. Relations are traces with layers (P§12).
- **Built beside the engine:** `native/crates/delta_exec`, on `blocks` and the rr parser alone. No old-engine crate is
  a dependency or a test oracle. Tests are narrow, in the crate, run by Relax on this machine (the user, 2026-11-25:
  "never run any tests outside of the NARROW new-architecture exploration"; "never do ANYTHING on a farm host").

## The first build, and why it was deleted (2026-11-25)

The first cut (38dbe75c8a), `change::body` (5e29b44a08) and the rework (88e9b70c5b) ran, but not as designed: a
driver of four fixed levels with name prefixes (`Boot::turn`); an executor whose wave, fixpoint and discovery were
three code paths, none with the operator signature; a new k-way operator in `blocks`; arithmetic in K0; strata
stated per head; a program change taken by module because I treated node renumbering as a problem; view-id ranges in
place of partitions; and a roll-up that rewrote rows every turn. *0686: Agreed and Built* draws it and lists each
divergence with its reason. The user: "why couldn't you just build what we designed? How am I supposed to reason
about this when you build something different to what you said you did?", and then: "delete the incorrect code,
everything which doesn't match the final shape", "create ARCHITECTURAL milestones along the way, which you will be
required to check", and "settle on the open points". The design pages were restored to the agreed versions; the
one addition is B§6a, the stratification section the user asked for, as options.

## Settled points (2026-11-25)

- **O1, a rule body's delta across k steps** (B§11, open): steps are separate operators over a frame of row ids
  (`source`, `join`, `semi`, `filter`, `head`); a body's delta is a sum of terms, each a chain of them starting from
  one step's change. The user: "we can do the individual operators, but we will need to eventually optimize them to
  avoid rewriting frames over and over, so maybe we will eventually need to be able to rewrite them as a combined
  operation", and "start with them separate, but we will certainly want to do better very quickly once we hit any
  scale". So a chain is data, a list of steps, and a fused form is a rewrite of it, checked against the chain.
  `change::body` is deleted.
- **O2, step order with no arithmetic in K0:** "if we can do without arithmetic, then do without arithmetic". Steps
  have a key of two columns, `(k, j)`; a lowering places a filter after atom `p` at `(p, f)`.
- **O3, rule identity across edits: not a question.** The user: "why do you always insist on bringing back the "no
  renumbering" … non-requirement?", pointing at the bend and oscar survey: node ids stay inside their file (0677, I3);
  what crosses is keyed by symbols, paths and values. A changed module is read again whole; what leaves is its heads'
  δ by value.
- **O4, strata:** "do it file by file, as agreed". A module is a stratum, numbered by its load (B§5).
- **O5, roll-up:** "geometric as it provides better complexity bounds and allows the merge operations to be simpler".
- **O6, the export:** "structurally, these steps should be like all others. Clearly k_form facts are the program and
  the rest are facts." No decision was left.
- **Auxiliary information** (the user, earlier the same day): "for the slow lookups and other auxiliary information
  what is needed but can't be stored in the facts or accessed efficiently from the facts, either it needs to be
  placed in the cache or we need to improve the program storage to include extra information." This applies to the
  plans of M4 (an operator's cache, L5) and to the traces' indexes (M1).

## The rebuild: milestones

Each starts only when the one before has passed every check; *0686: Rebuild Plan* §4 lists each check.

- **M0** the clean slate (done).

Rewritten 2026-11-25. The user, on the Operations page's revision 5: "okay, this is fine / but the milestones need to
be targeted very directly and precisely towards having a single boot operation work". The target is L1, the request's
"bootstrap level 1 operation" (B§4): `bootstrap-l0.rerail` on disk to P2's bistate in the program trace, one
delta operator built from operations over blocks, right cold, unchanged and edited.

- **B1** operations on blocks, as much as L1 needs: signatures of at most 30 slots, pinned through 0491's regions;
  the version registry; the twins (`no_std`, no alloc); a CPU executor.
- **B2** the program trace: `Trace`, `Frontier`, `Bistate`, `LayerW`, partitions by module with digests,
  `consolidate`; one bistate definition in `blocks`: a layer is `blocks::relation::Relation`, and `View` generalized to
  n layers is `Version` and `Bistate` behind P§12's `Read` (P§13). Roll-up after the target.
- **B3** the operator contract: `DeltaOp` as P§12 item 7, the cache's blocks and header (`cache_open`,
  `cache_commit`), the reference and the harness (L1 to L5).
- **B4** l0's tape through the file layer as it is (observer, `read_placed` into a slab room, the rr parse into a
  room); the tape a blob by content, its views `Read` states (B§7); the tape port's bistate; `Text`'s `String` gone. Any change to an existing crate goes to the user first.
- **B5** symbols, L1's facts in: B§7's Intern answerer (Pure, text ↔ symbol) as the `intern` operation, into the atoms block; no interning inside a view's read.
- **B6** L1 as a `DeltaOp` with B§4's ports, the `k0_pass` operation, run as B§8's `run` begins (the tape as a world
  read, then `k0_pass(tape.since(last), last)`): the target.

After the target: the self-reading check (B§5), whose milestones (a rule body, Δwave, Δfix, Δphase, roll-up,
planning) are written once B6 passes; then loads, the loop, program δ and the first K1 feature. *0686: Rebuild Plan*
§4 lists every check.

Enforcement: `scripts/check/check_delta_exec_shape.py`, which gains a section per milestone, runs with the copy check
before each commit; each milestone's commit and this record carry its checklist with evidence; any place where the
code would differ from the agreed pages stops the work and goes to the user first.

## M0: the clean slate (2026-11-25)

Deleted: the driver (`Boot`), the executor (`run.rs`: `Exec` and all its paths), `src.rs` (`Src`, `Rounds`), `db.rs`
(its relations, writes, roll-up, documents and view-id ranges), plan reading (`kform::read_module`, `Program`,
`Words`; it read through `Db` and made plans for `change::body`, and comes back in the new form at M3–M4), the K0
pass's writing into `Db`, arithmetic (`k_fn`) in the pass and in `l0`, stated strata, the kernel file, eleven tests,
and `blocks`' `change::body` (`change.rs` as it was before 5e29b44a08, which only added it).

Kept: `sym`; `tape` (a document's `Text` with its ERROR refusal, the reads, the names, and a view of one document's
tape whose row ids are its node ids); `kform` (the eight relations and `Refused`); `k0`'s walk with `(k, j)` keys;
`world` (`World`, `Files`); `bootstrap-l0.rerail` with 13 reader rules and `k_order`'s.

Checklist:

- [x] The crate compiles with 0 warnings; its 4 tests pass under Relax on this machine:
  `views_over_a_three_line_tape` (now read by the views' probes, with no executor), `k0_pass_rows_by_hand` (with
  `(k, j)` keys), `k0_pass_refuses_an_aggregate`, and `l0_is_k0` (new: `l0` is read as K0, 14 rules, every head a
  K-form relation).
- [x] `check_delta_exec_shape.py` holds: no `Boot`, `Exec`, `Rounds`, `settle`, `Db`, prefixes, `k1_`, `k_fn`,
  `eval`, `stratum`, `change::body` in the crate's Rust; no arithmetic and no `stratum(` in its `.rerail` files; no
  `fn body` in `change.rs`. Its patterns hit 18 lines of the deleted `lib.rs`, 26 of `run.rs`, and the deleted
  kernel's 6 arithmetic lines and 19 `stratum` facts.
- [x] `check_no_block_copies.py` passes.
- [x] This record states no proposal as a decision: the strata and auxiliary sections of the first build are gone.

Open, for the user: B§5 writes the view `doc(d, p, _)` with a third column the pages do not define; the view is
`doc(d, path)` until it is said what that column holds. B§3 also lists a `node` view, which nothing reads yet; it
comes when a rule needs it.

`doc` (2026-11-25): the user, "take a look at the other engine in this branch which defines it". The engine's
`doc(d: number, path: symbol, module: symbol)`: the module its load named (`Document::module`), else named from its
path. The view has that column (1301743933).

## B1: operations on blocks (2026-11-25)

Built to *0686: Operations* §2–3, on the blocks' ledger (0487) and regions (0491) as they are:

- `rerail_blocks::region::pin_block(block, op, write)`: one slot, the whole block, pinned for the CPU through the
  block's own region (`whole`, 0491 step 3) and its ledger, under the regions' checks. A conflict is refused (`Err`),
  not failed, and names the region, the operation and what holds the block. A CPU use now records who holds it, a
  guard's source line or an operation's name (`By`); the guards (`Read`, `Write`) keep their failures and messages.
  `BlockPin` is a guard of that region, as `Read` and `Write` are, not a handle of its own. The ledger gains
  `try_cpu`, its CPU check as a result.
- `rerail-delta-twins` (`native/crates/delta_twins`): `#![no_std]`, `#![forbid(unsafe_code)]`, no dependencies. A
  twin is `fn(&mut [Slot], &[u8])` over its slots' bytes and its params; `params!` declares a params struct once,
  and its `FIELDS` give UKL the declaration (`ParamsLayout::ukl`).
- `rerail_delta_exec::op`: `Signature` (at most 30 `SlotSpec`s, each a `Mode` with its UKL row and a block kind;
  `ParamsLayout`; `Phase`s of §4's `Shape`s), `Registry` and `Version` field for field as §3 writes them (`Impl::Twin`,
  `Ukl`, `Compiled`; the twin registered first as the one reference), and `run_cpu`, which pins every slot or none and
  runs the twin on the pinned bytes.

Checks, each passed under Relax on this machine (`relax test rerail-delta-exec --lib`: 10 of 10;
`relax test rerail-blocks --lib -- region::`: 16 of 16):

- A second writer, a writer beside a reader, one operation reading and writing one block, and a write of a frozen
  block each refuse the start, naming the operation and what holds the block, and leave nothing pinned:
  `op::tests::a_conflict_refuses_the_start_and_leaves_nothing_pinned`,
  `region::tests::a_slot_pin_refuses_what_conflicts_and_names_the_operations`.
- A 31st slot is refused where the operation is declared: `op::tests::a_thirty_first_slot_is_refused` (and
  `Signature::new` asserts it for a constant at compile time).
- `check_delta_exec_shape.py` (section `b1`) refuses a `static`, the heap or std, a lock, a cell, an atomic,
  `unsafe` or a dependency in the twins' crate; tried against a crate holding each, it named every one.
- A twin reads only its slots and params: run twice on the same blocks it gives the same facts, and its write moves
  the block's version: `op::tests::an_operation_runs_its_twin_on_its_blocks_in_place`.
- Also: a slot takes its kind of block and the params their declared size; the twin is the one reference and comes
  first; params give UKL their layout.

## The twins may use std, for now (2026-11-25)

B2's `consolidate` writes a frozen layer, whose layout and freeze only `blocks` (std) holds; Operations §3 put the
twins in a no_std, no-alloc crate with no dependencies. The user: "Leave std in for now. So long as the interface is
accepted and there aren't stupid things like global caches, using Rust data structures internally is going to be slow
but not wrong. Do make sure that we use linters and comments to ensure that the rules are respected, though."

So `rerail-delta-twins` depends on `blocks` alone, and a twin's `Slot` is its pinned block (`BlockPin`): it reads a
block's structures in place and writes only a write slot's. Inside a call it may use Rust data structures and blocks'
code. Nothing may outlive a call or come from outside it: no `static`, thread-local, `Once`/`Lazy` cell, lock or
atomic of its own, no thread's scratch (`scratch::with`), no file, environment, process, network or time, no
`unsafe`. The crate's doc states the rules; `check_delta_exec_shape.py` (section `b1`) refuses each (tried against a
crate holding each one); the crate's `clippy.toml` disallows them by path (written, not run: clippy is a build
outside the narrow Relax runs).

## B2's trace: two states, layers as storage (2026-11-25)

P§12 gives a trace any frontier (`at`, `between`, `fork`, named `pin`s). The user on 2026-11-24 (0678): "Nothing
reads more than two versions. I don't want a generic MVCC system." O5 chose geometric roll-up, which keeps one state
as about log n layers. Asked which governs B2, the user: "yes build it that way", to this reading: layers are only how
one state is stored; a trace has two states, the committed one and the run's open layer on it; 0678's `Bistate<S>`
stays the one definition, with `S` a `View` widened to n layers; no reads at other frontiers, no `fork`, no named
pins. B2's check is that a random chain of layers reads as a flat set at both states.

## B2: the program trace (2026-11-25)

Built as the user narrowed P§12 (above): two states, layers as storage.

- **The trace** is `rerail_blocks::trace::Trace`: one relation's layers in one block, and the run's open layer. A
  layer is a `Relation` as it is (P§13, "Keep"): the first full, each later one `Relation::try_continue` of the one
  below, so its ids follow theirs, its own rows are the rows its run added, its dead bits its kills of rows below.
  `Trace` holds only the block and its layers' offsets (bookkeeping of the operation graph).
- **The state** is `View::full` of the last layer, read down the chain: `View` widened to n layers (`View::chained`;
  `Relation::live_down`, a row live when no layer from its own up killed it; `Read::live_word` the same a word at a
  time; `ViewColumn` reads down the chain). Its ordered scan is `View::layered` and its probe `View::layered_probe`:
  each layer's own rows in order (a range of its sorted rows, or its index's bucket), live in the state, merged on the
  sort column, the lower layer first on a tie. `ordered_iter` and `probe_iter`, the old engine's hot paths, are left
  as they were and refuse a chain, naming these. The old engine's views and forks read as before.
- **The change** is `View::bistate` of the last layer: 0678's `Bistate`, the one definition, before the state below and
  after the state with the layer. Its gone rows are the layer's `changed` ids, so it is read in its size;
  `is_identity` answers from the counts alone (no id flipped and no row of its own) before any row is read.
- **`LayerW`** is `RelW` on the open layer: rows pushed, rows of the state killed.
- **Freeze over a chain.** `freeze_in` of a continuation given its base finds a pushed row's duplicate in any layer
  below (each layer's row index, else its sort column's range, as the two-level freeze does: `find_index` finds only
  bucketed indexes), live in the state below; a row killed and
  pushed again in one layer is brought back (its kill cancelled), so the change is the least one. Such a layer gets a
  row index too. A freeze with no base, or of a relation that is not a continuation, is unchanged.
- **Partitions** on a column the trace names (the module, for P2), with the partition index on each layer. Beside each
  layer, `consolidate` writes `parts(part, digest, added, gone)`, the digest's move a sum of `row_digest` (no order
  needed), and `gone(part, id)`, the layer's kills by partition, each pushed in partition order so its freeze keeps
  it sorted on the partition (a freeze keeps rows sorted on the column they arrive in order on: `gone` pushed in id
  order stayed sorted on the id): derived facts written once where computed. `restrict(p)` reads partition `p`'s rows
  of the last change through those indexes and says how many ids it touched (whole buckets and ranges);
  `part_digest(p)` sums the layers' moves; `part_ids(p)` is `layered_probe` on the partition index.
- **`consolidate`** is an operation (`delta_exec::trace::CONSOLIDATE`): one slot, the trace's block, read and written;
  params `ConsolidateP` (the layer's offsets, the sort and partition columns); its twin
  `rerail_delta_twins::trace::consolidate` calls blocks' `trace::consolidate` with a scratch of the call's own.
  `delta_exec::trace::commit` runs it through `run_cpu` and commits the layer.
- **Not built:** P§12's `Frontier`, `at`, `between`, `fork`, named pins (the user's two states); the roll-up (after
  L1). A row's liveness is read down the chain, one step a layer.

Checks (Relax, local):

- [x] A random chain of layers reads as a flat set at both states: 24 rounds of pushes (new rows, rows held, rows just
  killed) and kills, 4 seeds unsorted and 3 sorted on the partition column; each round's state is the model set, its
  before the previous one, its change passes `bistate::check` (L1) and `is_least`. With the chain dedup turned off the
  three trace tests fail (tried, restored).
- [x] A change restricted to a partition is that partition's added and gone ids. On a trace sorted on its partition
  it touches exactly those ids; on one that is not, its bucket's, and a small partition's change beside a big one
  touches fewer than 10 ids of a layer of 102 (`a_small_partition_s_change_does_not_read_a_big_one`). Counted by
  `restrict` itself: the Work counters are B3's. Its digest is its rows' digest. The count first showed `restrict`
  scanning `gone` (an ordered index is not `find_index`'s, and `gone` stayed sorted on the id): both fixed.
- [x] `layered` gives each live row once, in order on the sort column; `layered_probe` on the partition index gives
  the partition's rows (`part_ids`, both the bucket and the ordered case).
- [x] A round that changes nothing (nothing pushed, only rows held, a row killed and pushed again) is the identity by
  its counts, with no row of its own and no partition changed.
- [x] `consolidate` runs as an operation on the trace's block, in place; a reader holding the block refuses its start.
- [x] One `Bistate`, in `blocks`; no frontier API in the trace; its layers continue `Relation`s; no second trace or
  bistate in `delta_exec` or the twins (`check_delta_exec_shape.py` section `b2`; tried against a second `Bistate` and
  a `fork`: both named).
- [x] blocks `--lib` 125/125 (its relation and bistate tests unchanged); delta_exec `--lib` 11/11.

## Operations: the execution shape (2026-11-25, being designed)

The user: "We eventually want to be able to run these computations entirely on GPUs … chunk the computation into
"operations" which can be handled by a kernel launch, and whose logic can be expressed in an uglykl file … a fixed
number of blocks (less than 31) of both readable, writable, read/write and write-only (scratch), perform a meaningful
quantum of work, and then be chained together … a pure-Rust implementation as well as an uglykl one … a pure function
of its blocks and kernel: no globals or other ways of sneaking state in. Rust heap-allocated objects may be used to
keep track of the actual operation graph, etc but not to perform the actual computation."

Further constraints, in the user's words:

- "the CPU-side will be replaced with SIMD-optimized UKL kernels … and the executor will schedule across both CPU and
  GPU simultaneously. So the data-structures should be SIMD and GPU friendly and we can't use Rust data-structures."
- "We may need some blocks to refer to blocks that contain previous generations, so we will need for anything that is
  backed by blocks to have APIs to manage those blocks and dependencies."
- "I want a strict Rust one-writer or multiple-readers invariant for operations."
- "We may want to generate specialized ukl kernels for some operations; these need to be able to run on an
  interpreted/generic operation while we're waiting for the specialization to compile."
- "All execution requires planning. This will be a major part of the work."

Decided on the proposal's first revision (*0686: Operations*, https://claude.ai/artifact/YcK6jLbuHuF74LftE3TvU9):

- D1, agreed: an operation binds at most 30 whole blocks (Metal's 31 argument entries, one of them the params'); a
  relation, layer or segment is a region inside a block.
- D2, agreed: an operation's outputs are the same bytes on every executor and tiling.
- D3: "that won't be possible in practice; tiles may need to size following tiles and launch them from the tile
  itself, and we have some work shapes that need work queues. How we implement the tile execution will depend on the
  hardware."
- D4 (batching steps across terms): "as much as possible, but up to the executor". D5 (loops): "up to the executor".
- D6, agreed: an operation over its output's capacity writes what it needs and places nothing; the executor grows the
  block and runs it again.
- D7: "as much as possible, planning is part of the device path as only it may have the information necessary. In
  either case, planning needs to be able to happen on either side to avoid host/device switches."
- D8: reading files on the host, "and this needs to be handled carefully"; parsing on the host "okay for now, but we
  are close to much parsing being doable on the GPU".

On the second revision the user decided (2026-11-25), in their words:

- The file layer: "There is a specific Rust file-handling layer, that deals with checksums, tapes, parsing packing
  into blocks, fswatch, etc. We need to maintain that and ensure that we key everything properly so that each file's
  facts are content-addressed and we will re-generated things when that content changes. That's mostly existing code
  that needs to be minimally adapted."
- Caches: "They are vital to not redoing work and the overall performance. They need to be implemented as blocks but
  are every bit as important as the inputs and outputs."
- E1: "The runtime should track read and write references and fail if we attempt to (for example) run two writers at
  once or a writer when there are readers, just like Rust".
- E2: "We will need a descriptor table (like we currently have) to enable references to be managed across GPUs".
- E3: "we just need the ability to register versions of an operation and some metadata so that they can be chosen.
  For specializations of things inside other operations, we will need to design this separately once we have the
  ability to JIT-compile things".
- E4: "Please survey to figure out the missing shapes. There are a lot of shapes."
- E5: "Agreed, planning is not done in rerail programs".
- E6: "Reading files needs to be basically the same as it is today in the existing engine, you should never have
  created a new World::read string/text path."

M0's `World` and `Files` (a read returning a `String`) were that path of its own. They are deleted (`f5d060b50d`),
and `check_delta_exec_shape.py` refuses a file read in the crate. Loads are to be answered through the engine's file
layer as it is: the observer (stat reuse under the racy rule, the journal, a Git blob's digest), the run's one read
placed into a residency slab room, sources and tapes as content-addressed objects, documents' places, and a file's
facts keyed by content, path and plan.

The page's third revision surveys the file layer and the caches as they are, and the work shapes of the engine's
kernels (23, on six axes). On it the user decided (2026-11-25):

- E4: "no idea, be thorough". The survey was done again (the page's revision 4, §4): every `.uglykl` file, the
  UKL library, the kernels inside Rust sources, MLDB's forest and Metal kernels, the CPU executors' forms and the
  decisions on how work runs. It found 46 shapes in seven families (47 once revision 5 restored uneven segments, which the earlier list had), placed on six axes (how work is found, an item's
  unit, what is shared inside it, what joins the units, where output goes, the bounds). The seventh family is how the
  executor places work (batches, morsels, handoffs, splits, the scoreboard), not an operation's shape.
- F1 (a search with several valid answers): "so may a CPU search, so what? ANything numeric can vary. The results
  need to be correct; determinism is nice to have but not required". This revises D2: a result must be correct on
  every executor and tiling; the same bytes are not required. Tests check correctness (the same set of facts, a valid
  answer, a numeric result within its bound), not bytes.
- F2 (B§5's `load(path, k)` against the engine's `load(path, module, grammar)`): "use something different for
  bootstrapping than load, we already agreed before to not have a separate strata fact". The bootstrap's files are
  `boot(path, k)`, a file's place in the bootstrap being its stratum (O4), and `k_order(d, k) :- boot(p, k), doc(d,
  p, _).`; `load` is the engine's. `check_delta_exec_shape.py` refuses `load(` in the crate's `.rerail` files.

Revision 4 also corrects revision 3, which drew a new ledger for E1. The engine already has the tracker E1 asks
for: one ledger a block (0487) and region handles (0491: `Mutable`, `Shared`, `Frozen`, every use counted and
checked, naming the operation, versions, a mirror a memory domain), built on the user's words of 2026-11-20. E2's
tables exist as the job address tables (0100 §9), the slabs' address lists (0471) and the directory of domains and
replicas (0440 §7.2). The operation layer is to use them as they are, adding three things: slots pinned from an
operation's signature when it starts (the step 0491 leaves below it), references to earlier generations as
`Frozen` handles kept by the operation graph, and a descriptor table a launch and device.

On revision 4 the user said (2026-11-25): "You're missing a second kind of cache. You have shown how we write caches
in datalog. However, the input to the delta operations also has a "cache", which is an in-memory data structure
where any auxiliary information like indexes that we don't want to regenerate can be kept. This is mutable and needs
to come in as one or more blocks."

That cache is the agreed contract's operator cache (P§2, L5: it changes cost, never answers; P§11.3: mutated in
place, its header naming the state it describes, with an open mark). Revision 5 of the Operations page gives it its
place: one or more `Mutable` blocks an operator, kept in memory across runs, partitioned by file for a per-file
operator, taken by the operator's operations as read or read-write slots under the region checks, opened and
committed by two operations (`cache_open` resets it to ∅ when the header is open or names another state;
`cache_commit` writes the after state last). It holds what an operator reads to go fast and could make again:
indexes a plan asks for that a layer lacks (written by `order`), lines and the joins' memo, statistics past a
layer's sketch, the versions chosen, slow lookups. What answers read (kept rows, levels, derivation counts) stays
data, in traces. The caches written in datalog (0103) are the other kind.

## B1 and B2 rebuilt to the pages (2026-11-25)

The user asked for every divergence of B1 and B2 from the design pages (*0686: Operations*, P§, *Bootstrap from one
tape*); the audit found 23. The user: "close all of these gaps with respect to the original plan". The sections "B1"
and "B2: the program trace" above record what was built first; this section supersedes their as-built statements
(e9c74edefc, 4fabca7c57). What the user approved stays as approved: std inside the twins, two states, roll-up policy
after the target.

What is built:

- **Blocks refer to blocks** (O§2, E2). `blocks::gens`: a block's reference table (after the engine's header, at 64:
  up to 30 block ids) and its body (at 192, its used length first). `delta_exec::dir::Directory` hands out block ids,
  writes a reference only to a frozen block (refused otherwise, naming both), and holds a `Frozen` handle of each
  referent while its referrer lives, so a generation lives until its referrer is rolled up and no reader holds it
  (`sweep`). A freeze trims the body to its used length; blocks are free-listed, so a freeze gives back an operation's
  work past it.
- **A launch** pins every slot by its mode, read-pins every block the slots reach, transitively, and writes the CPU's
  descriptor table (`Desc`: id, kind, version, address, length) in a block of its own; `gens::Table` resolves an id to
  a pinned block's bytes. Any conflict refuses the start, naming it, with nothing left pinned (E1).
- **Shapes are phases on the six axes** (O§4, "No enum of shapes"): `Phase{found, unit, inside, across, output,
  bounds}` and an `Independence`; no numbered shapes. A twin is `TwinPhase`s (items, a tile's run, `again` for passes
  and turns, bounded by `Bound::Turns`), its phases its signature's by name; state between phases lies in blocks. The
  executor cuts each phase's items into tiles (`Tiling{tile, seed}`, the seed shuffling their order).
- **Versions are registered checked** (O§3): `Registry::check` runs a candidate and the reference on cases at given
  tilings and compares the written slots' facts by meaning (`Signature.facts`, rows sorted); `register` takes the
  `Checked` it gives, for that version only.
- **The trace is layer blocks** (O§7, O§10, P§11–§12). `blocks::layer`: a trace's `Shape` with every P§12 field;
  δ runs, claimed by an atomic add, counting what overflows (`needed`); a layer block per generation, over the one
  below by reference: rows in canonical order (partition first), a plus bit and a kill id each, its kills ascending
  (`flips`) and by partition (`pflips`), the partition index (key, rows, flips, added, gone, digest), a sketch a column
  (min, max, count, 8 AMS counters), the shape's order indexes, the header written last. Ids continue from the layer
  below.
- **consolidate and merge are the twins'** (O§10): one pipeline of 23 phases (sort by chunks and merge passes,
  segmented sums, emit by count/scan/place, kills by a batched search of the layers below, partitions counted,
  scanned and filled, flips sorted, indexes sorted, sketches by partials, the seal), every loop over rows in the
  twins' crate. consolidate (scratch δ runs, write facts, rw control) sums each row's diffs, drops zeros and refuses
  a change that is not a set change into control: diffs past ±1, a row gone not live, a row added already live.
  merge (read the newest layer, older ones by reference; write facts) rolls layers up into one, an add and its later
  kill cancelling.
- **The host holds handles** (`delta_exec::trace::Trace`: its shape and its head's id). `commit` makes the layer's
  block, refers it to the head, runs consolidate, reads control, freezes it; `roll_up` runs merge and lets the merged
  generations go. A state is `Chain` (the layers through a launch's table): `bistate::Read` with scan, probe
  (`matching`), part, stats and flips; `States` with restrict and changed_parts. `Bistate` gains invert, compose,
  restrict and changed_parts; a change between adjacent layers is read in its size (the layer's flips and counts).
- **Small ones:** the `node(d, n, k, start, end)` view, with the engine's columns; K-form's trace shapes 32 bits a
  column but `k_num`'s value; delta_exec on blocks and rr_syntax alone.

Checks, each passed under Relax on this machine (`relax test rerail-delta-exec --lib`: 20 of 20;
`rerail-blocks --lib -- bistate`: 8 of 8; `-- region::`: 16 of 16):

- [x] Map, emit, sort, merge and queue give the same, right facts at four tilings and orders
  (`op::tests::each_shape_gives_the_same_facts_at_two_tilings_in_shuffled_order`); consolidate writes the same layer
  at three, by the registry's check (`trace::tests::consolidate_writes_the_same_layer_at_every_tiling`).
- [x] E1 conflicts refuse the start with nothing left pinned; a reference to a block not frozen fails; reading a
  block read-pins what it refers to, and its table gives each by id (`op::tests::references_are_…`).
- [x] A version is registered only once checked; a wrong one (the cube for the square) is refused; a 31st slot is
  refused; a twin's phases must be its signature's, and a phase that runs again must bound its turns.
- [x] A random trace of six runs reads as its brute-force set in every state, partition and probe; every change
  between any two states passes `bistate::check` with the right rows gone and added, and the right changed
  partitions; the sketches count the live rows (`trace::tests::a_random_trace_reads_as_its_set`).
- [x] Compose and invert; the committed state read beside a commit; refusals with the head unchanged; a roll-up's
  state, and its merged generations gone once no reader holds them; a referenced generation living until its
  referrer is rolled up.
- [x] `check_delta_exec_shape.py` (sections b1, b2, rewritten): no enum of shapes, the six axes, `register` taking
  `Checked`; no trace in one block, no lock or frontier API on the host, `Chain` holding no list, `Trace` its shape and
  head, the twins' crate free of `TwinFn` and of blocks code that computes over rows. `check_no_block_copies.py`
  passes.

Then (the same day) the rest of P§12's shape a run block's trace holds, and no bound on a chain:

- **Every index kind** (P§11.2), in the formats that exist, written once: an order with fences and prints
  (`blocks::order`'s, searched by `order::find`), a hash order (positions by the lead's 16-bit tag, then the key, with
  prints), a permutation, and a Bloom filter (`blocks::bloom`'s hash and mask, about ten bits a key). consolidate and
  merge write them (phases `iaux`: a chunk an item; `bloom`: a word an item, the keys sorted by word first, so no two
  items write one word); a probe skips a layer whose filter rules its key out.
- **No bound on a chain:** `Scan`, `Flips`, `changed_parts` and merge follow every layer (a cursor a layer); the
  first build's `Scan` asserted past 8.
- **A partition's key is 32 bits:** a partition column is a module's or a document's id; a wider one is refused.

Checks (Relax, local; `rerail-delta-exec --lib` 22 of 22, `rerail-blocks --lib -- bistate` 8 of 8, `-- order::` 1):
every probe of a random trace with all four index kinds finds the brute-force set's rows, before and after a roll-up;
the filter holds each key of its layer and rules out most absent ones; the indexed layer is the same at three tilings
(`every_index_kind_answers_its_probes`; breaking the prints, the tags or the fences each fails it). A chain of 14
layers scans, changes and gives its changed partitions as its sets do, and rolls 12 layers into one
(`a_deep_chain_reads_as_its_set`).

The independence axis is kept and checked: an operation whose items are `Not` independent runs each phase whole and in
order at any tiling asked; a version is checked at tiles in shuffled order against the reference run whole, so a false
claim of independence writes other facts and is refused (`op::tests::the_independence_axis_is_kept_and_checked`, a
running sum declared both ways). merge is checked as consolidate is: the same rolled-up layer, with all four index
kinds, at three tilings (`trace::tests::merge_writes_the_same_layer_at_every_tiling`). `rerail-delta-exec --lib`: 24
of 24.

What a run block's trace refuses, each saying where the plan puts it (`a_shape_the_plan_puts_elsewhere_is_refused_…`):

- `levels`: a recursive stratum's level column (0364), run by Δfix, after L1.
- `Packed`, `Ranged`, `Dict`: P§13's form of a block that leaves its run (kept, journaled, a pinned slab); a run
  block's layers are `Open`.
- `KeptFile`, `PinnedSlab`: the kept stores and the device executor, after L1.
- `Cache`: the operator cache (P§11.3), B3, one state mutated in place; `History`: named frontiers, which the user's
  two states rule out.

Outside these runs: the descriptor table is the CPU's (a device's comes with a device executor; no Metal runs here);
the twins' clippy.toml and a cargo build of these crates were not run (I wrongly took "stop wasting time" as a ban
on them; clippy was run later, below); references of a block that is gone are let go at the directory's next `release`, `make` or `sweep`.

## The device's descriptor table; clippy and cargo (2026-11-25)

With cargo and clippy on a farm host and Metal allowed for it (the user, 2026-11-25), what the section above left
outside its runs:

- **A device's descriptor table** (E2): `Launch::device_table` writes the launch's rows again, each block's address
  its device buffer's (`gens::device_rows`: the block's memory wrapped in place, `Block::device_buffer`, nothing
  uploaded), in a DESC block of its own the kernel binds, and names the reached buffers as used through addresses.
  `gens::DESC_READS` is the table's device half in UKL: a row as `Desc` lies, its address a pointer, and `desc_row`,
  an id's row as `Table::row` finds it. `op::tests::a_kernel_follows_references_through_the_device_table`: a kernel
  reads four blocks (a chain of references) through the table, their headers, reference tables and values, as the
  CPU's table gives them, and an id the launch cannot reach has no row; passed on the CPU backend (Linux, cargo) and
  on Metal (one farm Mac, the device opened).
- **Clippy**, the twins' `clippy.toml` with it, over `rerail-blocks`, `rerail-delta-twins` and `rerail-delta-exec`
  (lib and tests): the delta crates have no finding. In blocks it found a real fault: `Rows::lower_bound`'s search of
  a column under key runs was guarded by `(0..1 << 63).contains(&key)`, an empty range (`1 << 63` is `i64::MIN`), so
  it never ran, and once run it compared a run's high bits unsigned where the column's order is its values' (i64).
  It now compares them signed and runs for every key; its test's column is sorted as i64 (the run whose high bits
  are negative first). blocks' findings left are in `maps.rs` (the run room's, not this line's).
- **Cargo**: the three crates build and pass under cargo (Linux host): blocks 118 of 118, the delta crates 25 of 25.

## B2's layer rebuilt as a Relation (2026-11-25)

Compared with the plan pages again, the layer built above was a format of its own, not P§12's ("A layer is one
`blocks::relation::Relation`: rows added, kills of rows below, its indexes, its partition digests, its statistics"),
nor the rebuild plan's B2 ("A layer is `blocks::relation::Relation` as it is … built on blocks' relation and 0678's
bistate, as one definition") or B1 ("a twin uses blocks' layouts and freeze in place"). The user: "BUILD THE PLAN".
This section supersedes the layer format, the 23-phase pipeline and the sketches described above.

What is built:

- **A layer is three Relations in its block** (P§12). `rows`: the rows it adds, continuing the relation of the layer
  below (`Relation::try_continue_far`: `DELTA | CONTINUES | FAR_BASE`, the base named by block id and its offset
  there, `Relation::far`), its ids from the rows below on, its kills of rows below its `changed`, its hashed indexes
  (each `Hash` index of the shape, and the row index) built by its freeze, and its sorted orders (`Order`: fences and
  prints, `Relation::add_order`). `parts(part, digest, added, gone)` and `gone(part, id)`: the partitions' change
  and the kills by partition, each a relation sorted on the partition (an index in the layer's block, not a copy:
  0353). The header (`LayerHead`, written last) holds the three relations' offsets, the counts, the digest and each
  Bloom filter's offset (`blocks::bloom`, in the block's chunks).
- **Frozen where it lies**: `relation::freeze_layer` sorts the rows on the shape's first order column (they are
  written in the whole canonical order, which the stable sort keeps), records the column, builds the indexes and
  stats, and keeps the kills as the relation's change. The build of an index is the one `freeze_in` uses
  (`build_indexes`, factored out of it).
- **Statistics are blocks' sketch** (O§10, 0678 step 10): a layer keeps each column's `blocks::sketch::Sketch` of the
  state it ends, in its kept form (`Relation::sketches`): the layer below's, its rows added, its kills taken. The
  layer's own sketch type (`SketchW`, 8 AMS counters) is removed.
- **No copy of a layer's kills**: the separate `flips` and `pflips` arrays are gone; a layer's kills are its
  relation's change, and by partition the `gone` relation.
- **Index kinds** (P§11.2): `Hash` is the relation's hashed index; `Order` is an order with fences and prints;
  `Bloom` a filter. `Perm` is refused, naming packed's permutation (P§12), which belongs to the Packed encoding.
- **The state** (`Chain`, P§12's `Version`, read through a launch's table) reads the relations in place: a row is
  live when it is an own row of a layer and no layer above it kills it. A restriction reads a layer's partition as a
  range of its rows (`Layer::part_rows`) and its kills from `gone`. A change across exactly one layer reads its
  counts from the layer (`exact_counts_since`; a partition's from `parts`), touching no row. Changes are named by
  layer (`flips_since`, `new_ranges`), so a layer that only kills (no rows of its own) is a change of its own.
  `Chain::counted` counts every row and liveness word a read touches.
- **The twins write the layer with blocks' code** (B1). consolidate and merge are 13 phases: sort, passes, segments,
  count, scan (the layer's relations made and its rows claimed), place (its rows written, `Relation::put_own`), kills
  (a batched search of the state below by its indexes), ksort, freeze, parts, orders (an Order or a Bloom filter an
  item), sketch, seal. merge's source is the merged layers' own rows (+1) and their kills (−1, the killed row's
  values). `Ctx::read_and_pin` gives a phase a read slot's bytes and the written slot's pin at once.
- **The descriptor table names the launch's slots** (O§2: "per block id it can reach (its slots, and what they refer
  to, transitively)"): a slot pinned to write is its pin's row (`Table::is_written`; a host read through the table
  finds none, the writer reads it through its slot), a slot pinned to read is pinned and read like any reached block.
  A device's table is made from the same rows.
- **Signed 32-bit columns** (O§2: "integers, 32 bits where the values fit"; the Bootstrap page's keys use −1, as in
  `k_arg(c, -1, i, t)`): a relation's 4-byte column may hold signed values (`rows::I32` in a spec's widths; the
  column's `runs` set to `rows::SIGNED`: its high bits are its low bits' sign). Every read of a 4-byte column
  sign-extends it: `ColDir::get`, `read_col`, `lower_bound`, and the CLIF's `col_get`, from which the UKL reads
  header is generated again (`ukl/reads/relation.h`). A layer's `I32` columns, and its partition columns, are these.

Checks, each passed under Relax on this machine:

- `rerail-delta-exec --lib` (all but the device test): 26 of 26. The trace's tests are ported to the new layer and
  two are added: `a_restriction_reads_only_its_partitions_rows` (a restricted scan, probe and change read only
  partition 50's rows and kills, counted where the reads are made, against about a thousand rows in seven other
  partitions) and `is_identity_reads_a_layers_counts_only` (a layer of rows, a layer whose run cancelled, a layer
  that only kills: `is_identity`, whole and restricted, touches no row). The sketch's distinct values are checked
  against the set's (within the sketch's own 20%); the descriptor table's rows include a launch's slots
  (`references_are_to_frozen_blocks_…`).
- `rerail-blocks --lib -- rows:: relation:: layer:: bistate::` 21 of 21, with `a_signed_columns_values_read_as_written`;
  `-- sketch:: bloom:: gens::` passed.
- `rerail-clif --lib -- relation:: reads::` 15 of 15: the JIT's reads of a signed column are the host's, and the
  generated UKL header is the CLIF's. `the_jits_lower_bound_is_the_tables` had been failing since 231207f9b0 (freeze
  keeps rows that arrive in another column's order, and the test's second column arrived sorted); its data now
  arrives unsorted on both columns, as the test means.
- `check_delta_exec_shape.py`: B1 no longer refuses blocks' relation code in the twins (the plan's B1 asks for it);
  B2 checks that a layer's rows and parts are Relations, its statistics blocks' sketch, `Perm` refused, no second
  sketch or kill copy, the twins making, freezing and indexing the relation, the descriptor table not filtering the
  slots, and the two tests above.

Not built yet, each still the plan's:

- `relation::View` and `Chain` are two definitions; the plan's B2 asks for one (View generalized to n layers).
- E1 on a device: GPU uses pinned through fences, and replicas made by the directory where a device needs them.
- consolidate's kills search each row's key in every layer below (a binary search a layer, by its order); its cost
  is not measured.
- Clippy with the twins' `clippy.toml` was not run on this code, so it is not shown to catch anything yet. (I wrote
  here that cargo and clippy runs were not allowed: the user never said so. It was run later, below.)

Then (the same day), two of the items above:

- **A version runs by its own shape** (`Version.shape`, "the shape it runs the work as"): the registry checks a
  twin's phases against its version's shape, and the executor runs each phase by that shape's bounds. A version of
  the sort whose shape bounds its passes at two turns is refused by the check on a case of four passes
  (`op::tests::a_twin_fits_its_signature`).
- **B§7, "depends on blocks and rr_syntax alone", in the code too**: the device table moved into `blocks::gens`
  (`gens::device_table`, beside `device_rows` and `DESC_READS`), and blocks no longer re-exports the device layer
  (added in 425fb811e3 so that delta_exec could name a device). delta_exec gives a launch's pins (`Launch::pins`); the
  caller makes the table on its device; only the device test, through its dev-dependency, names one. Format 16 still
  reaches delta_exec as rr_syntax's own output (`rerail_rr_syntax::tape_format`, rr_syntax's interface since
  2026-11-13). `check_delta_exec_shape.py` refuses any other crate named in delta_exec's code, and a device
  re-export in blocks.

Checks (Relax, local): `rerail-delta-exec --lib` 26 of 26 (all but the device test, which compiles; no Metal here).

And references by block id and offset (O§2: "A block refers to another by block id and offset, in its header"): a
block's reference table holds `gens::Ref { id, at }`, the offset (32 bits) of what it refers to in that block; 30 of
them move the body from 192 to 320 bytes. `Directory::refer` takes the offset; a layer refers to the layer below's
header (`BODY_AT`), and a chain refuses a layer whose relation continues one it does not refer to. Checks (Relax,
local): `rerail-delta-exec --lib` 26 of 26; `rerail-blocks --lib -- gens:: layer:: bistate:: relation:: rows::` 21 of
21.

And the first item above, **one definition** (the rebuild plan, B2: "Version and Bistate are its View generalized
from a base and one delta to n layers ... as one definition"): `layer::Chain` is gone, and a trace's state is
`relation::View` itself. A View is a full relation, a delta and its base in hand, or a trace's state
(`View::chain(table, head, arity)`): its top layer's relation and the layers below it, each reached by block id and
offset through the launch's descriptor table, checked once when it is made, as `Chain::new` checked them. Its empty
state has no top (`top: Option<&Relation>`; `View::rel` gives a relation's). One `bistate::Read` and one
`bistate::States` are implemented for View: the delta-and-base reads as they were, and the chain's reads (liveness by
the layers' kills, the probe by each layer's indexes and Bloom filters, the merged scan in canonical order, a
partition's rows and kills, the counted reads, the partitions a change touches) as `Chain`'s were. The layer's
`parts` and `gone` stay its own relations, read through `layer::Layer`. The read counter is an atomic, so a View stays
`Send` and `Sync` for the readers that share one across threads. The old engine's uses of View now build it with
`View::new(base, top)`. `check_delta_exec_shape.py` refuses a `Chain` type, or a `Read` or `States` for one, beside
View. Checks (Relax, local): `rerail-delta-exec --lib` 26 of 26; `rerail-blocks --lib -- bistate:: relation:: table::
change:: sketch:: facts:: layer:: rows::` 42 of 42; `rerail-clif --lib -- relation::` 8 of 8; the twins compile.
Not run: the old engine's tests that build a View (setfix's blocks comparison tests, store's kept facts) and blocks'
examples. Their change is the constructor only. Compiled since (2026-11-25, `cargo check`): blocks' examples and
store's library, and logic's library and tests, with no error.

And **the kill search's cost, measured** (the third item above). A View's counted reads now count every search step
too: each row a layer's binary search compares, and each step of a search of a layer's kills. Over a trace of eight
runs (eight layers, 963 rows), each live row's probe as consolidate's `kills` makes it (the canonical order, its first
live match) read 62 rows and kills on average and 165 at most, within its bound of two searches a layer and a search
of the kills above a candidate (590 here) (`trace::tests::a_kill_search_reads_a_search_a_layer`).

Counting the search steps showed two reads that were not what P§ says:

- A chain's scan searched the kills of every layer above for each row it gave, and a restricted state searched each
  layer's whole kills, not its partition's. The scan is now a merge: a cursor a layer above into its kills (its
  partition's, when restricted), each moving forward only. A row's kill check reads its partition's kills alone.
- A restricted state found its partition's rows by a search of the layer's rows on the partition column, which reads
  other partitions' rows. P§: `restrict` is "a range of the block, by the partition's index", the index "part → its
  rows, its digest and count". `parts` is now `(part, digest, added, gone, first, kills_at)`: a partition's own rows
  are `first .. first + added`, its kills `kills_at .. kills_at + gone` of `gone`. The twins write both (the sums of
  the partitions before it), and a restricted probe searches only inside its partition's range.

The restriction test now bounds each read by partition 50's rows, kills and the searches within its ranges (the
partition index's own entries are not counted). Its line "far fewer reads than the trace's rows, by ten" is gone: it
was set when search steps were not counted, and with them the reads are 80 of the trace's 770 rows. The test also
checks the index's ranges against the rows and kills of every partition of every layer. Checks (Relax, local):
`rerail-delta-exec --lib` 27 of 27; `rerail-blocks --lib -- bistate:: relation:: table:: change:: layer:: gens::` 26
of 26.

And **E1 on a device** (Operations §2: "every slot pinned from the operation's signature when it starts", the GPU's
uses held until its fence completes; E2: "replicas by the directory (0440 §7.2)"):

- A slot's CPU pin is handed to a fence (`BlockPin::to_fence`). Under the region's one lock its CPU use ends and a
  GPU use by the operation begins, with the region's own GPU checks. Nothing can begin between the two. A refused
  handoff gives the pin back as it was.
- A launch is handed whole (`Launch::to_fence(fence, op, domain)`): every slot by its mode, and every block it reaches,
  to read. Its device table's block goes the same way (`DeviceTable::to_fence`). Until the fence completes, a launch
  that would write a block the device reads, or read one it writes, is refused, and no block is freed. A launch on a
  domain where a block it reaches has no copy is refused and given back.
- Replicas are the directory's (`Directory::replicate(id, domain, mirror)`). The directory keeps the block's region,
  and with it the replica, until the block is frozen or let go. The region brings the replica to the latest version
  before a kernel there reads it (`Region::sync_host` brings it back after one writes). A device's table rows in that
  domain name each replica's address (`gens::device_rows_in`, from `Mirror::device_address`).
- The device test (`a_kernel_follows_references_through_the_device_table`) now hands its launch to a fence before the
  kernel runs, and completes the fence after the command buffer does, as the engine's watched queue would.
  `check_delta_exec_shape.py` checks each of these (its E1 checks).

Checks (Relax, local): `rerail-delta-exec --lib` 28 of 28, with
`op::tests::a_launch_on_a_device_holds_its_blocks_until_its_fence_completes` (the handoff, refusals until the fence
completes, the replicas and their rows, on fences completed by hand); `rerail-blocks --lib -- region:: gens::` 17 of
17, with `region::tests::a_slot_handed_to_a_fence_is_the_gpus_until_it_completes`.

Not built or not run:

- The device test compiles here but was not run (no Metal runs here).
- No backend implements `Mirror` yet, so a replica is only the tests' simulated device memory. The table's own block
  in a discrete domain, made and bound there, is not built: `device_table` makes it in the host's memory, read in
  place, as before.
- The executor's `execute` runs twins only. A device version launched by the executor through this handoff is
  registered versions' work (E3), not yet built.

And **clippy** (the last item above), over blocks, the twins and delta_exec, libraries and tests, with the tests'
features (`cargo clippy -p rerail-blocks -p rerail-delta-twins -p rerail-delta-exec --lib --tests --features
rerail-delta-exec/test-hooks`, 51 s from the target as it was, a few seconds after an edit). Its findings there,
each fixed:

- `Launch::to_fence` returned the launch unboxed in its `Err`; it is boxed now.
- Two tests used `.err().expect()`; they use `let … else`.
- blocks' maps (0678): `Mem` was neither `Send` nor `Sync` only because it caches a pointer into its `Region`,
  which is both, so an `Arc<Mem>` was flagged six times. It is now `Send` and `Sync`, by the same reasoning as the
  maps that hold it. A string's hash reads its words by `as_chunks`, and the maps' `UnwindSafe` impls come before
  their tests.

The twins' `clippy.toml` is shown to catch what it names: a call of `std::env::var` added to a twin was reported
("the environment is not a twin's input"), then removed. Its three lints are now `deny` in the twins' crate, so a
twin that breaks a rule fails the lint rather than warning. The lint is clean over the three crates; the warnings
left are in crates they depend on (ukl_vec, tape_format, ukl, c_pp). Checks (Relax, local): `rerail-blocks --lib --
maps:: sketch:: region::` 31 of 31; `rerail-delta-exec --lib` 28 of 28.

## The audit's findings, fixed (2026-11-25)

The audit of what was built against the pages (the user's request: "compare what was built to what we planned")
found seven gaps. Each is fixed here, in the order fixed.

**A phase runs by its axes** (Operations §4, §6). The executor read only a phase's turn bound before; its other axes
were labels. Now:

- A phase whose count is `Found::Fixed` has its count from the launch's start, and running it with any other count
  is an error ("an earlier phase made it (an indirect count)").
- Only units that share nothing, combine, or make a round (`Across::Nothing | Combine | Rounds`) run in shuffled
  order. An ordered phase runs its tiles in order.
- `fits` refuses a twin that runs again where its shape's units do not relate by rounds, and a shape whose units
  relate by rounds where its twin has no finisher.

consolidate and merge are now twenty phases. Each runs the items its shape declares:

- an ordered scan of chunk counts;
- the layer opened (one group);
- the kills sorted, by chunks and merge passes, and placed in the change;
- a partition an item for parts and gone;
- a column's range a lane;
- a hashed index a group;
- the freeze (one group);
- a column's sketch a lane;
- the seal.

Before, six of them ran as one item. Only `open`, `freeze` and `seal` are one item now, each declared `Fixed,
Threadgroup`. `check_delta_exec_shape.py` checks both: the executor's rules, and the one-item phases.

**The blocks code a twin calls keeps the twins' rules.** A layer is written by `relation::layer_write`, an item a
call: `open`, `stat`, `index`, `kills_mut`, `seal`, `sketch`. `freeze_layer` and `set_sketches` are gone, and
`build_indexes` builds one index a call (`build_one_index`). blocks has a `clippy.toml` with the twins' lists, and
its lints are `deny` in `layer_write` (allowed in the rest of blocks). A `Mutex` added there was refused by clippy,
and a `static` and a lock by the shape script, which checks the module with the twins' patterns.

**A layer keeps its kills in two places, and no more** (made one, below). They are in its rows' change
(`changed`) and in `gone(part, id)`, read by partition. The shape script checks that a Layer is exactly its bytes, head, rows, parts and gone, that `gone` is
`(I32, 4)` sorted on its partition, and that the twins make two relations and one layer, whatever they are named.

**B2's random chains, at the plan's size.** The plan asks for "24 rounds of pushes and kills, 7 seeds".
`a_random_trace_reads_as_its_set` now runs them, each seed on a thread of its own, 1.7 s under Relax. Its runs
include:

- new rows;
- rows killed and added back in the same run;
- rows killed in an earlier round and added back.

It checks:

- each round's state is its set;
- each round's change is the least one, by the values' sets;
- each partition, restricted;
- a change across six layers is right by value.

The dedup was turned off two ways: consolidate's kill search reading only the newest layer, and a row's liveness
reading only the newest layer's kills. The test failed both times, and the code was restored.

What it showed: a change across several layers is by id. A row killed in one layer and added back in a later one
is gone under its old id and added under its new one, with the same values, so the change is not the least one by
value. Only a round's change (two adjacent states) is checked as least, and a span's rows gone and added are
checked less the rows live at both ends. Nothing in P§ asks a span's change to be least by value. Its partitions
changed are by digest, and are exact.

**The geometric roll-up runs** (O5). consolidate's seal reads the layers' headers through the descriptor table
(`due_to_merge`). From the new layer down, a layer joins while what is merged so far is more than half its size, by
its rows and kills. It writes the count to control (now 48 bytes), and `Trace::commit` rolls that many layers up
(`roll_up`, merge) when it is two or more. After a commit, every layer is at most half the one below it, so a trace
keeps O(log n) layers.

`RollUp::Never` is for the tests that keep every head they committed, and for merge's version-check case. Measured
by `trace::tests::a_geometric_trace_keeps_log_layers`, over 64 commits (one first run of 3,000 rows, then small runs
of 1 to 60 changes, 5,194 pushes):

- 35 commits merged;
- at most 6 layers;
- each layer at most half the one below, after every commit;
- each state is its set;
- a key's kill search read 43 rows and kills on average and 48 at most, within its bound of 64.

The unmerged eight-layer trace of `a_kill_search_reads_a_search_a_layer` read 62 on average and 165 at most. The
shape script checks that a new trace rolls up geometrically and that commit runs it.

Checks:

- Clippy over the three crates: clean (the narrow command above).
- Relax, local: `rerail-delta-exec --lib -- --skip device_table` 30 of 30; `rerail-blocks --lib` 123 of 123.
- Both check scripts.

**A trace's shape settings are built** (P§11.3, P§12, 0364). `Shape::encode` refused them before; now each one
changes what is written and read:

- **Levels** (0364): `ShapeW::level_col` names the level column. A key is live at one level at a time: the kills
  phase finds a key at another level and kills it there too (`one_level`). A trace with a level column and a second
  key order is refused, saying where. `levels_are_a_column_one_live_a_key` checks it.
- **Cache** (P§11.3, retention `Cache`): a cache trace holds one state, which its header names (`kind::CACHE`, 32
  bytes, `cache.rs`). `CACHE_OPEN` and `CACHE_COMMIT` are twins, with phases of their own. `Trace::open_cache`,
  `commit_cache` and `cached` use them; a commit to a cache whose header is not open is refused, and so is `change()`
  of a cache. `a_cache_is_one_state_its_header_names` checks it.
- **Placement** (P§12): `KeptFile` lays the layers in a trace folder's files (`blocks::kept::trace_folder`, the only
  file access, so the shape script still refuses `std::fs` in the crate). `Trace::open_kept` opens it again from the
  generations' references, and a gone layer's file is removed when it is swept. `PinnedSlab` lays them in slabs
  through a `Backing`. `a_kept_trace_lies_in_its_files_and_opens_again` and `a_slab_trace_lies_in_its_slabs` check
  them.
- **Encodings** (P§12): an encoded layer's own rows are written once, in blocks' own forms, and read where they
  lie:
  - `Packed`: `packed::Packed`, each column in the smallest of col's encodings.
  - `Dict`: the same, with `DICT` where it is smallest (`choose_for_reads`).
  - `Ranged`: `ranged::Columns`, each column cut into ranges, each range in its own encoding.
  - `Perm`: packed's permutation index on one column the order does not lead. It is allowed only with Packed or
    Dict, and refused otherwise, saying where.

  The pipeline gains two phases, 22 in all. `place` records each added row's source item; `encode` writes a column
  an item and `perm` an index an item, both reading the source where it lies (`layer_write::encode_open`,
  `encode_col`, `encode_perm`, over `packed::writer` and `ranged::writer`). Nothing is first written open: an encoded
  layer's relation holds no rows. `layer::OwnRows` reads them for `Layer`, for the twins' later phases (ranges,
  indexes, orders, sketches) and for probes, where a Perm column's probe goes through its index. A chain's end is now
  its newest layer's end (`View::total`), not its top relation's row count. col's passes run piece after piece when
  a twin calls them (`choose_serial`, `write_serial`).

  `every_encoding_reads_as_its_set` checks each encoding: B2's random chains (states, least changes, restrictions,
  probes), layers that lie encoded with their relations empty, a roll-up, probes on every index kind, and a Perm
  probe that reads at most 4 rows per row found (plus 8). consolidate and merge write the same facts at every tiling.
- **History** stays refused, by the user's answer recorded with B2: "a trace holds two states … no Frontier, at,
  between, fork or pins". The refusal test keeps it, along with the level column's errors and Perm without
  Packed or Dict.

**The device side runs outside the tests, and runs** (E1–E3; Operations, revision 4: "UKL versions after the
twins, checked against them on ukl_vec; no Metal run"). There were four gaps:

- **The executor runs UKL versions.** `execute` runs every version through one phase driver (`drive`), the
  axes' rules above included, with a twin's calls or a device's kernels behind it:
  - The device is `op::Kernels`, given by the crate above it. delta_exec compiles and launches nothing (B§7), and
    no longer has a dev-dependency on rerail-gpu.
  - The launch is pinned as a twin's is. `Kernels::bind` makes its device table and hands that to the fence, then
    `Launch::to_fence` hands over the launch.
  - Phase `p` is three kernels, launched by tiles in any order where the axes allow: `<entry>_<p>`, one thread an
    item; `<entry>_<p>_items`, its count; and, for a phase by rounds, `<entry>_<p>_again`.
  - The fence completes when the last kernel has. In a device domain of its own, the written slots then come back
    to the host (`Region::sync_host`).
  - `Registry::check_on` checks a UKL version against the twin at each tiling. `run_on` runs the device's default
    version, and runs the twin when there is none.
- **A backend implements `Mirror`.** `region::DeviceCopy` is a block's copy in a buffer of the device's own: uploaded
  and downloaded by the device's queue, with its address and buffer (`Mirror::device_buffer`, `copy_buffer`).
- **The table's own block lies in the device's domain.** `gens::device_table_in` writes the rows (each replica's
  address there), then places its block there (a `DeviceCopy`, uploaded at the handoff). The kernel binds that copy,
  and the fence keeps the block's region until it completes.
- **The device test runs.** `rerail-delta-device` (`native/crates/delta_device`) is `Kernels` on the process's device,
  whichever backend it is. `Gpu::host` reads each block in place, and `Gpu::discrete(domain)` reads replicas
  (`Gpu::replicate`). Its tests choose the CPU device's vector path (`RERAIL_GPU=cpu`, `RERAIL_CPU_EXEC=vec`), unless
  the process names a device, so there is no Metal run:
  - `a_kernel_follows_references_through_the_device_table`, moved here from delta_exec. It had never run before.
  - `a_ukl_version_is_checked_against_its_twin_on_the_device`: a UKL `square`, read and written through the device
    table, is checked against `samples::SQUARE` at three tilings, then registered, and `run_on` runs it. A broken
    one (the cube) is refused, which shows the kernels ran. With no device given, the check is refused.
  - `a_ukl_version_runs_on_replicas_in_a_device_domain`: the same in domain 1, where every block is a `DeviceCopy`
    and so is the table's block. The facts are right only if the replicas were uploaded, read and written by the
    kernels, and the written one was brought back.

  The shape script checks each of these, and that delta_exec does not depend on rerail-gpu.

On a GPU (the user's answer: run it on the farm's M3 host): `rerail-delta-device`'s three tests ran on an Apple M3
Max's Metal GPU (`RERAIL_GPU=metal`; the tests print the device they opened, `Devices::METAL`), 3 of 3, in 0.43 s:
the device table read by a kernel, the UKL `square` checked against its twin and run by `run_on`, and the same on
replicas in domain 1, each a `DeviceCopy` in a Metal buffer of its own. No discrete GPU ran: an Apple GPU's memory
is unified, so domain 1 there is the device's own buffers, as on the CPU backend. `Impl::Compiled` is refused
("compiled ahead, which is not built").

Checks:

- Relax, local: `rerail-delta-exec --lib` 35 of 35, nothing skipped; `rerail-delta-device --lib` 3 of 3;
  `rerail-blocks --lib` 123 of 123.
- All three check scripts.
- Clippy: at first it failed in the installed farm wrapper. On a crate it compiled within two hours (its "hot"
  marker), the wrapper puts its remap flags before the `rustc` argument that clippy-driver takes first, and
  clippy-driver then reads that path as an input file. Each failed attempt refreshes the marker. The fix is in
  `scripts/build/farm/farm.py` (the remaps go after that argument), not installed. With this worktree's markers of
  the four crates aged, clippy ran over blocks, the twins, delta_exec and delta_device, libraries and tests: clean
  (after the step below).

**A layer's kills are kept once** (the audit's item 4, finished). Before, they were kept twice: in its rows'
relation's change (`changed`, ascending) and in `gone(part, id)`, the form B2 names. Now they are in `gone` alone:

- `layer_write::open` lays out no change, and `kills_mut` is gone. `seal` refuses a layer whose rows record a change.
- consolidate and merge lose `ksort`, `kpasses` and `kplace`, the three phases that sorted the kills into the
  change, so they are now 19 phases. `parts` writes `gone` from the rows out, as before.
- A layer's kills are read by partition, each partition's run ascending:
  - `Layer::kills()` is `gone`'s id column; `kill_runs`, a run a partition; `kills_within`, a partition range's kills
    among a range of ids.
  - Liveness (`chain_live_word`): each layer's rows in the word are a range of partitions, read from the rows'
    partition column (the canonical order leads with it), and each is searched in the layers above.
  - `killed_above` searches the row's own partition's kills.
  - A scan's cursor holds the row's partition's kills, and is placed again by one search when the layer's rows reach
    the next partition.
  - Flips merge their runs through a heap (`bistate::Flips`), since a layer gives a run a partition.
- The shape script refuses a layer change laid out or written, a read of one in `layer.rs`, and a second sort of
  the kills in the twins.

Two checks of the change:

- **Cost.** The first liveness read searched the union of the partitions of all the word's layers, and the B2
  random test ran in 9.0 s against the committed code's 5.3 s (warm, at loads of 58 to 68). Reading each layer's own
  run brought it to 4.1 s at a load of 73.
- **Dedup off.** Both breaks were run again on this code: the kill search reading only the newest layer, and
  liveness, scans and searches reading only the newest layer's kills. B2's random test (24 rounds, 7 seeds) failed
  under each, and passed once each was restored.

Checks (Relax, local): `rerail-delta-exec --lib` 35 of 35, `rerail-blocks --lib` 123 of 123,
`rerail-delta-device --lib` 3 of 3; the check scripts; clippy as above.

**B1's bounds and the operator cache, made real** (2026-11-25, after the audit against the pages: five of a phase's
six bounds were declared and never read, and the operator cache was a trace of frozen layers).

- Every bound is the executor's (`op::drive`). A twin reports through its meter (`Ctx::steps`, `depth`, `sample`,
  `room`/`reach`): an item past its phase's bound fails the launch, and so does a report of a quantity the phase does
  not bound, so a twin cannot do what its declared shape says it does not. A slot's capacity is how far its block's
  allocations may reach inside its reservation (`Directory::make_capped`, `capacity`, `grow`): past it, `Refuse`
  fails naming the slot, `Grow` raises the capacity, the bytes where they lie, and runs the phase again, and `Rerun`
  runs the launch again from its first phase, once, at the capacity it needs. Past the reservation, it fails.
- The launch budget is not a phase's bound but one ledger (0601): every phase's turn is a launch charged to the open
  evaluation (`Directory::evaluation`, `charge`; 200 by default, `LAUNCH_BUDGET`), and one past it fails with the
  account by operation and phase. `Bound::LaunchBudget` is gone.
- Bounds fit their axes when an operation is declared: each kind once; a unit holding stacks bounds their depth, and
  only such a unit does; a claimed output bounds its capacity; a phase by rounds bounds its turns; a launch run again
  past a capacity writes no slot it reads. A UKL version declaring a bound its items report is refused (a kernel's
  meter is not built).
- A real bound on a real operation: the kill search in `consolidate` and `merge` reads a layer a step, bounded at 64
  (`KILL_STEPS`). A trace never rolled up fails its 66th commit there; the geometric roll-up keeps it far below.
- The operator cache (`delta_exec::cache`, twins in `rerail_delta_twins::cache`, no longer test-only) is P§11.3's:
  mutable blocks of kind `CACHE`, never frozen, no generations, each with a header naming its state. `cache_open`
  resets a header left open or naming another state to ∅ where it lies, then marks it open; `order` adds a layer's
  row ids in a column order, with fences, unless the cache holds them, growing the cache within its reservation
  (`Capacity(Grow)`); `cache_commit` writes the state, the mark last. A trace of `Retention::Cache` is refused, naming
  the operator cache.

Checks (Relax, local): `rerail-delta-exec --lib` 45 of 45 (new: each bound inside and past it, bounds against axes,
the ledger, the kill search's 64 layers, five of the cache); `rerail-blocks --lib` 123 of 123; `rerail-delta-device
--lib` 3 of 3; the shape script's new `bounds` and `cache` sections; clippy over the four crates, clean.
