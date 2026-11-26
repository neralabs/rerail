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

**B3: the operator contract** (2026-11-25; `delta_exec::contract`).

- `DeltaOp` is P§12 item 7's, port for port: `delta(ins, want, cache, out, asks) -> Work` (a bistate per input port,
  read where its layers lie; the demand on its outputs; its cache; a δ runs block a port for its outputs and for its
  asks) and `run(ins, out)`, the plain function, on one state a port. Where its form differs: `type Ports` is
  `fn ports()`, since a port's shape is a value; `type Cache` is the operator cache's block (`CacheW` over an
  `OpCache`), whose layout is the operator's operations'; each takes `Cx`, the registry and directory its operations
  run in, and returns a `Result`.
- `Work` is P1's counters (rows read, probes, rows written, each excess named).
- The cache: `CacheW::read` is ∅ (`None`) when the header is open or names another state than the run's before (the
  digest of its inputs' states, by content: `Trace::digest`, the sum of its layers'); its first `write` runs
  `cache_open`; the driver (`Instance::step`) runs `cache_commit` with the after state when the run wrote it, so an
  identity run leaves it untouched. A run whose outputs outgrew their runs blocks runs again at the size they needed,
  its half-written cache read as ∅ (its header is open).
- The generic reference (`reference`): `run` on the before side and on the after side, diffed, written once.
- One harness (`harness`), over a random edit chain (`random_chain`): each step run by a threaded instance, by one
  whose cache is dropped at a given step, and cold; L2 (the cold run is ∅ → op(X)), L1 (the threaded change is the
  reference's), L4 (every input the identity: the output too, the work none, the cache's facts unchanged), L5 (the
  dropped-cache run's output is the threaded one's) and L3 (the threaded output is the cold run's). It returns the
  first law broken, with its step.

Checks (Relax, local, `rerail-delta-exec --lib` 53 of 53): a toy (`out(b, a) :- in(a, b), a % 3 != 0`, its added
rows read through the orders its cache keeps of its input's layers) passes over 6 chains × 4 drop points; four toys
fail, each on its own law and no other: one drops its kills (L1), one writes a row of its own on a cold run (L2), one
reads its input when nothing changed (L4), one drops its kills when its cache reads as ∅ (L5). A cache left open or
naming another state reads as ∅; the reference on a hand-made pair of states; an identity run's work, output and
cache. The shape script's `contract` section; clippy over the crate, clean.

**B4: l0's tape through the file layer** (2026-11-26; `delta_exec::files`).

- The pieces this crate could reach without the old engine's higher crates: the observer (`rerail-observe`: its
  digest step, its placed read, its counts), the tape's home (`rerail-tape-format`: `Home`, `home::Text`) and the
  slabs' rooms (`rerail-slabs`). Not reachable: the placer, in `lang`, and a parser that writes its tape into a room
  (format 16's writer grew a vector of its own, which `lang` copied into a room, `place_stored`). Two changes to
  existing crates, put to the user and agreed: the placer (`place`, `placer`, `home_of`, `slab_room`, the slab room)
  moved from `lang` to `rerail_slabs::placed`, `lang` re-exporting it (a vector-only `place` where there is no
  device layer); and format 16's writer takes a room for its tape's bytes (`Sixteen::room`, `Writer::out_in`,
  `Sink16::in_room`, `rerail_rr_syntax::tape_in`), every other writer as it was (`room: None`). A block kind for a
  tape (`kind::TAPE`).
- A document is named by its path relative to the root, its module from the answer (the path without `.rerail`).
  `Files::answer` asks the observer's digest step, which reuses its answer on an unchanged stat under the racy rule; a
  file it must read is read once by its placed read into the slabs' placer's home and its digest recorded; the rr
  parser writes the tape into a tape block's room (`InBlock`), where it lies. The document, its source and tape, is a
  blob named by its content (`Blobs`, sha256 of both). The tape port (`Doc`) is ∅ → tape on the first answer, the
  identity while the digest is unchanged (read again or not), the old tape → the new one when it changed. A file
  that cannot be read, is not UTF-8 or does not parse is refused, naming its path.
- `Text` holds its source where the read placed it (`home::Text`), not a String; a source in hand (a test's, the
  bootstrap's own) is made a home of its own (`text`).

Checks (Relax, local, `rerail-delta-exec --lib -- files` 6 of 6): an unchanged file gives the identity, and the
observer's counts since show no byte read (one answer reused on its stat); an edited file is read once and placed once
(the observer hashed one file); a file that does not parse, and one that is not there, are refused, naming the path;
the tape lies in a tape block; a cold answer names the path and its module. The shape script's `B4` section (and its
file rule: a test may write the files it answers, and reads none). Clippy over slabs, tape_format, rr_syntax and
delta_exec, clean.

**B5: symbols, written by `intern` alone** (2026-11-26; `delta_exec::intern`, `delta_exec::sym`, twins
`rerail_delta_twins::{intern, tape, k0}`).

- The atoms block is a directory block (`sym::Atoms`): an atoms structure laid out inside it (`blocks::atoms::
  lay_out_in`), read and appended where it lies, from a pinned slot's bytes (`find_in`, `name_in`, `count_in`,
  `append_in`, sharing `AtomsW::append`'s code). `Symbols`, a locked writer every read could intern through, is gone.
- `intern` is an operation (slots: the document's tape block, read; the atoms, read-write; the document's symbols
  block, read-write), its twin the reference. One item, the document: the language's names and K-form's step kinds,
  the answer's path and module, then each node in node order: a leaf's text (an identifier, a variable, an operator,
  a literal) and a string's unescaped value. Its output is the document's node → symbol relation, laid out by node id
  in the symbols block (the document is its partition, L9). `Interned` keeps each document's symbols block with the
  blob it came from: an unchanged tape's is the one it had, and `intern` does not run.
- The views read the symbols block: `text` is a leaf's symbol (a clause's whole text is no longer a symbol: B5 interns
  identifiers and strings), `value` a string's symbol or a number's value, `doc` the path's and the module's.
  `Names` finds the kinds' and fields' symbols; nothing in a read interns.
- The walk (L1's reading, B6's twin) moved into the twins (`k0::walk`), reading a document's tape and source where
  they lie (format 16's reader over a slot's bytes, `f16::read_in_place`, added to `tape_format`), its symbols from
  the symbols block and its names from the atoms. The twins' crate depends on blocks and the rr parser's tape reads
  (`rerail-rr-syntax`), the pair B§7 names for the executor; the shape script holds it to those two.
- A change to B4 as built: an operation reads a document's source and tape through one slot, and a residency slab is
  not a directory block, so the observer's placed read now places the source into a room of the document's tape
  block (`files::place_in`), where the parser then writes the tape; the slabs' placer is not used here (it stays the
  one placer, in `rerail-slabs`, for `lang`). When a device reads tapes, the tape block is what the slab is for it.

Checks (Relax, local, `rerail-delta-exec --lib` 61 of 61): the same text gets the same symbol in a later run, and a
new text a new id past the first run's; an unchanged tape interns nothing (no run of `intern`, the atoms' count as it
was, the same symbols block); every row of every view of l0 read, and the atoms' count as it was. The reads that
cannot write: `tape.rs:142-143` (`text` and `value` read `syms::text` and `syms::value`), `sym.rs:42,47,53`
(`find_in`, `name_in`, `count_in` over a read launch), `rerail_delta_twins::k0::walk` (`find_in`, `syms::text`,
`syms::value`); the one writer, `rerail_delta_twins/src/intern.rs` (`put`, `append_in`). `k0_pass_rows_by_hand` and
`l0_is_k0` (14 rules) pass with symbols read, not made. The shape script's `B5` section; clippy over blocks, twins,
tape_format and delta_exec, clean.

**B6: L1, the boot operation** (2026-11-26; `delta_exec::{k0, l1, boot}`, twin `rerail_delta_twins::k0`).

- `k0_pass` is an operation (read the document's tape block, its symbols block and the atoms · rw control · write
  each K-form relation's δ runs block), of three phases, emit (shape 11): `count` walks each statement (an item) and
  counts its rows by relation, writing the first refusal by node order into control (its node and why); `scan` turns
  the counts into each statement's places, claimed at once in each runs block (`blocks::layer::runs_claim_in`);
  `place` walks each statement again and writes its rows at its places with the run's sign (`runs_set_in`). No
  atomics; a refused document writes no row. Its twin is today's walk, split a statement at a time
  (`k0::statement`), registered as its reference. The host's walk (`k0::rows`) is gone: every reading goes through
  the operation.
- L1 (`l1::L1`) is a `DeltaOp` with B§4's ports: **tape**, a row a document (its tape block, where its source and
  tape lie in it, its node count); **no program**; **one production**, K-form's eight relations, a trace each;
  **facts in**, a row a document naming its symbols block; **output**, P2's bistate in those traces.
- File grain, as built: a document added is read whole by `k0_pass` (+1), a document gone is read whole again from
  its kept tape (−1), and `consolidate` nets the two, so the δ is the new rows against the kept ones. The kept rows
  are not stored a second time: the old tape is a blob, never versioned, and its rows are its reading. L1 keeps no
  cache. An unchanged tape is the identity: no operation runs. A symbols block now lives as long as its blob
  (`Interned` keyed by path and blob), since a kept state of the facts-in port names it.
- B§8's run, its first two lines (`boot::run`): the tape as a world read (`boot::Tape::read`: the file layer's
  answer, `intern` when changed, the ports' next state), then L1's instance stepped over the tape since the last read.
  Not built: the run's returning its pins, and the rest of B§8's run.

Checks (Relax, local, `rerail-delta-exec --lib` 66 of 66):

- the harness (`contract::harness_on`, over input traces made by the world read) passes L1 over a chain of edits of
  a copy of l0 on disk: cold, unchanged, a rule added, a rule changed, unchanged, a rule removed, back to l0; the
  cache dropped at steps 1, 3 and 5 (`the_harness_passes_l1_over_edits_of_l0_on_disk`);
- cold: `k0_pass_rows_by_hand` (a five-line file's rows, through L1's plain function) and `l0_is_k0` (14 rules), and
  `l1_cold_on_l0_gives_its_14_rules` through `boot::run`;
- unchanged: the work is the checks alone, the outputs' heads and the cache's bytes as they were, and the file not
  read (`files.reads` unchanged, under a 100 ms racy window);
- edited: the outputs' change equals the reference's, the diff of two cold runs; the old tape and the new one read
  once each;
- `k0_pass_refuses_an_aggregate` (through L1's plain function) and `k0_pass_refuses_an_aggregate_through_l1` (as a
  delta operator, through `boot::run`): refused naming the rule's node, no row written.

L1's Rust beyond the operations, read: `l1.rs:34-53` (the ports' shapes and rows), `l1.rs:61-77` (a port's row read
into `k0_pass`'s document), `l1.rs:81-86` (one `k0_pass` call into the outputs' runs), `l1.rs:97-112` (`delta`:
identity, the gone documents −1, the added +1), `l1.rs:114-121` (`run`: every document +1); and the world read,
`boot.rs` (the file layer, `intern`, the ports' next layer). The shape script's `B6` section holds L1's file to
running no operation but `k0_pass`, writing no row itself and walking no tape on the host, and `k0_pass`'s phases to
count, scan, place. Clippy over blocks, twins and delta_exec, clean.

## B4–B6 rebuilt to the pages (2026-11-26)

Compared with the pages, B4–B6 as first built had eight departures that were not founded on the plan. Each is
rebuilt here, in the order the user set.

**Facts in carry the atoms** (61d2bdfb6c). B§4: facts in are the program globals. The contract has global ports
(`contract::{Global, GlobalIn}`): a block every level shares, append-only (P§13: the atoms are "a trace with no
kills"), so its state is its count. A global's count is part of the cache's digest and of L4's identity check
(`In::is_identity`). L1's one global port is the atoms (`Ports::globals = ["atoms"]`), read through
`ins.globals[ATOMS]`; L1 no longer holds them. A document's node → symbol facts travel with its tape port row, so
the facts-in trace port is gone.

**Ports name blobs, housed from placed bytes** (7e615e9b68). P§12 item 1 (`house(bytes: Placed) -> BlobId`, "takes
bytes where they lie"; `get` in place), P§11.4 (source text and tapes are blobs, by content) and P§11 (a relation
names a blob by its id):

- `files::Blobs` maps a `BlobId` (the sha256 of the bytes where they lie) to a `Placed` (a frozen block, an offset,
  a length). `house` refuses bytes in a block not frozen: a blob is never versioned. The same bytes housed again
  are the first blob, and the second block is let go.
- A document's tape block is frozen once parsed. The parser's regions are let go first. On a block with no free
  list, letting a region go frees nothing, so the bytes stay where they were written. The source and the tape are
  then two blobs (`files::DocIds`).
- `intern`'s symbols block is frozen and housed too. Its first word is now its body's length, which is what a freeze
  keeps.
- L1's tape port row is `(d, source, tape, symbols)`, each blob as four words: 13 columns. L1 finds each blob's
  place through `Cx::blobs`.
- A view reads a document inside a read launch (`files::reads` over the block's bytes), not through an owned `Text`.

How the store houses objects (`rerail-store`'s SQLite rows and object segments) is not brought into `delta_exec`.
What is kept from it is what P§12 asks: bytes housed where they lie, named by content, read in place, never copied.

**L1 diffs against its output's kept rows**. B§4: "an edited l0 is read again whole and diffed against its kept
rows". P§3: "the file's last result is its output trace's before, not cache". P§11.3: kept rows "are data, kept in
traces".

- `Out::kept(port)` is each output trace's head before the run (`None` for a cold run and for `run`).
- A new generic operation, `drop_parts`, is in `delta_exec::drop` with its twin `rerail_delta_twins::drop`. It reads
  a trace state's head, with the layers below reached by reference, and a parts block (the partitions). It writes
  every live row of those partitions into a δ runs block with a sign, read through `View::chain(..).part(p)`: one
  item, counted, claimed at once, then written in order.
- L1's delta retracts each gone document's kept rows from every output trace with `drop_parts` (−1). It reads an
  added document with `k0_pass` (+1). It never reads an old tape again.
- L1 keeps no cache. P§3's cache holds a wave's plans and compiled code, and L1 has neither. The user's goal asked
  that L1 keep its rows in its cache, so that the cache and L5 checks test something real. That would make the
  file's last result cache, which P§3 and §11.3 rule out. So the kept rows are read from the output traces, and L5
  stays vacuous for L1: its first operator with plans will exercise it. (Superseded by "L1's cache" below: the
  kept rows stay in the output traces, and the cache holds each document's sizes.)

Checks (Relax, local, `rerail-delta-exec --lib` 69 of 69):

- `drop_parts` over two layers, the second killing a row of the first, gives exactly the live rows of the partitions
  listed. Past its room, its claims are counted as needed.
- `a_gone_document_is_retracted_from_the_kept_rows_not_its_old_tape`: with only the new document's blobs held, the
  run's output is the new document's cold rows. Its rows written are the kept rows retracted plus the new ones.
- `the_source_and_the_tape_are_blobs_in_the_documents_frozen_tape_block`: both blobs' names are the sha256 of their
  bytes where they lie, in one frozen tape block. Housing the same bytes again gives the same blobs.
- The harness over l0's edits on disk, and the B4–B6 checks above, pass unchanged in meaning.

The shape script's `B6` section now also holds L1 to its atoms as a global port (no `Atoms` beside its ports), to
retracting through `drop_parts` over `out.kept`, and to no old tape read again with −1.

**One `k0_pass` launch reads every changed tape**. Its slots are a docs block, the atoms, control and the eight runs
blocks.

- The docs block refers to each document's source, tape and symbols blocks (`Directory::refer`, each frozen). It
  lists each document's module and where its source and its tape lie.
- A block refers to at most `MAX_REFS` (30) others, so past 9 documents the docs blocks are chained: each refers to
  the next. They are made back to front, because a block refers only to frozen ones. The launch reaches every block
  by reference and reads each where it lies.
- An item is a statement of any document, numbered across all the documents in order. A refusal is the first by
  document and node order, and control names its document.
- L1 gathers its added documents (in `delta`) or its live ones (in `run`) and calls `k0_pass` once. Its parameters
  are the first docs block and the sign.

Found on the way, and fixed here: the change that made source and tape two blobs let a new source share an earlier
document's tape. A tape holds a document's shape, not its text, so two sources of the same shape have one tape blob.
The new source's tape then lies in the earlier block, and `intern` refused it ("a document's source and tape in two
blocks"). Every reader now takes the two places, each read in its own block: `files::reads`, `files::nodes`,
`intern`'s slots (source, tape, atoms, symbols) and the docs block's three references per document.

Checks (Relax, local, 71 of 71):

- `one_k0_pass_launch_reads_every_document`: 20 documents, so two docs blocks, chained. Their rows equal those of
  each document read alone. The launch is 3 phase launches in all (`Directory::launches`). A refusal at the 18th
  document names that document.
- `a_new_source_may_share_an_earlier_tape`: two sources of one shape. The second's tape blob is the first's, in
  another block, and is read through `reads` from the two blocks.

The shape script holds `l1.rs` to one `k0_pass` call, and `k0.rs` to one launch over `docs: &[K0Doc]`.

**`run` takes and returns `Pins`** (B§8: `run(l0, globals, world, last: Option<&Pins>) -> Pins`).

- `boot::run(l1, inst, l0, globals, docs, bench, last)` returns `(Pins, Doc, Work)`. Its `Pins` say where the run
  left each trace it read or wrote: the document set's head, L1's outputs' heads, and the atoms' count.
- A trace has two states, the committed one and the run's open layer on it. This is "B2's trace: two states" above:
  no reads at other frontiers, no named pins. So `Pins` is a record that holds no block, and P§12's `Trace::pin` (a
  named frontier) is left out by that decision.
- `last = None` is a cold run. Every bistate starts from ∅ (L2): the document set's, the atoms', and L1's outputs,
  which are emptied first (`Trace::clear`).
- `last = Some(p)` must be where the last run left the traces. Pins of any other state are refused, as a cache of
  another state reads as ∅ (L5).
- The document set (`boot::DocSet`) is B§7's first answerer and the trace it keeps (P§11.4: path → tape id,
  partitioned by document). A changed document's old row is retracted where the set keeps it (`drop_parts`), so the
  host no longer keeps a second map of the rows. Its trace is not rolled up, so a run's before is a layer in its
  chain.
- As agreed in B4, l0 is named by its path, which the file layer reads.

The first build of this step pinned frontiers with a block that referred to the head, and let a run start from older
pins. That is the named-pin API the user ruled out for B2, and the shape script's frontier check caught it; it was
rebuilt as above before it was committed. The names `Boot` and `World`, which M0 retired with the first build, are
not used: the document set is `DocSet`, and L1 and its instance are passed as they are.

Checks (Relax, local, 72 of 72): `a_run_takes_and_returns_pins` covers four runs.

- A cold run's pins are its traces' heads.
- A run from them after an edit is the delta: the kept rows are retracted and the edit is read.
- The old pins are then refused. A run from the new pins with nothing changed is the identity, and returns the same
  pins.
- A run with no pins is cold: it reads the document whole and gives the same rows.

The harness over l0's edits now takes its states from the document set's heads, read by read. The shape script
holds `boot.rs` to `run` taking and returning `Pins`, refusing other states' pins, and keeping no map of the set's
rows.

**Blobs housed as the store houses objects**. The store houses an object as a row and its bytes. The row, in its
objects table, names the object's digest and where its bytes lie: a segment, an offset and a length
(`object_segments::row_frame`). The bytes are read in place in the segment (`ObjectFrame::bytes`,
`ObjectBytes::Mapped`). The ports-name-blobs step above kept the placement half of that: bytes housed where they
lie, a `Placed` (block, offset, length). Its index was still a host `HashMap`, the "own blob map" departure, so
`files::Blobs` is now rows in a trace:

- Each row is `(name's four words, block, at, len)`, sorted by name (`blobs_shape`).
- `house` adds a row as a layer, unless a probe of the name finds the bytes already housed.
- `get` probes the name at the trace's head (`Read::matching`, through each layer's index).
- A row names a frozen block, as an object's row names its segment.

The store's segments are mapped files and its rows are in SQLite. Neither is brought in: a run's blocks are its
directory's, and where they reside is the slabs' question below.

Found on the way: `an_unchanged_file_gives_the_identity_with_no_read_past_its_stat` has been flaky since B4. The
observer hashes one trusted file in 64 again (`VALIDATE_EVERY`), choosing by path and the current second. So on
about one run in 64 the test saw l0's bytes read, and failed. The test now allows a hash past the stat only as that
sampled check, which must find no mismatch.

Checks (Relax, local, 72 of 72): every earlier check runs over the trace-held blobs. These include 20 documents'
60 blobs, with shared tapes and geometric roll-ups, all found by their names, and the same bytes housed twice giving
one row. The shape script holds `Blobs` to rows in a trace, with no map.

**`intern` names only identifiers and strings**.

- A node's text symbol is made only for an identifier or a variable. An operator's is the language's name for its
  token: found in the atoms, never made. A number is its value, and a string is its unescaped value, so neither
  literal's text is interned.
- The `text` view's rows are the nodes with a name: identifiers, variables and operators. A literal is read through
  `value`. The thirteen reader rules use `text` only on names and operators.
- The language's names (its kinds and fields, and K-form's step kinds) are interned once, by the first run that
  lacks them. They are appended in order, so one `find` of the last of them answers for all. Before, each run looked
  up every one of them.

Checks (Relax, local, 73 of 73): `intern_names_only_identifiers_and_strings` interns a second document. Its new atoms
are exactly its path, its module, its identifiers and variables, and its string's value. A number's text, a string's
quoted text and an operator are not among them. The operator reads in `text` as the language's `!=`. The shape
script refuses interning every leaf, and the language's names interned without the one guard.

**Slabs as directory blocks (for item 4)**. P§11.4 has sources and tapes read in place in residency slabs. The
directory already makes a block in a pinned slab (`make_placed(PLACE_SLAB, …)`, from the source `with_slabs` gives
it), and a trace's layers lie there under `Placement::PinnedSlab` (`a_slab_trace_lies_in_its_slabs`). A document's
tape block did not: it was always a sparse reservation in the run's memory. `files::tape_block` now makes it in a
slab when the directory has a source of them, and in the run's memory otherwise.

- A slab is a directory block like any other. It is referred to, frozen, housed as a blob and read by launches as
  the reservation was, so no reader changed.
- When nothing holds the block, its slab goes back to its source at the sweep.
- A slab is sized at the reservation's bound (80 times the source). A run-memory block is sparse, so only what is
  written is resident; a slab is not.

Checks (Relax, local, 74 of 74): `a_documents_tape_block_lies_in_a_slab_when_the_directory_has_slabs` reads l0 with a
slab source. Its tape block lies in a slab, and its source and tape read there as in run memory. It is one slab, and
the slab is given back once the block is let go. The test slab source moved out of the trace test so both tests share
it.

**P§12 item 7's trait against ours, re-read**. P§12's `DeltaOp` has associated `Ports` and `Cache` types, and
`delta(ins: &In<Self::Ports>, want: &Bistate, cache: &mut CacheW<Self::Cache>, out: &mut Out<Self::Ports>, asks:
&mut LayerW) -> Work`, with `run(ins: &At<Self::Ports>, out)`. `contract::DeltaOp` differs as follows:

- **A context, `cx: &mut Cx`** (the registry, the directory, the blobs, the trace and cache operations). P§12's
  trait has no way to reach the directory its launches run in. Without `cx`, an operator would hold it itself, or
  reach it as global state, which the engine does not have.
- **`Result<Work, String>`, and `run` returns `Result`**. K0 refuses a construct outside it, naming its node, and the
  refusal must reach the caller (`k0_pass_refuses_an_aggregate_through_l1`). P§12's `Work` names excess, not refusal.
- **`want: Option<&Bistate>`**. `None` stands for all of the outputs. Every caller so far passes `None`: no operator
  yet takes demand.
- **No associated `Ports` or `Cache` types**. Ports are a value (`fn ports() -> Ports`), `In`, `At` and `Out` hold
  vectors by port index, and `CacheW` is untyped. Nothing forces this: every use of `DeltaOp` is generic
  (`Instance::step<O>`, `apply`, `reference`, `harness_on`) and none is `dyn`. So a port's count and shape are
  checked when the instance is made, not when the operator compiles.
- **`Out` carries each port's kept head** beside its δ runs block. This is P§3's "the file's last result is its output
  trace's before": L1 retracts a gone document from the rows its output keeps.
- **`asks` is an `Out`**, a runs block an input port, where P§12 has one `LayerW`.

P§3 and §11.3 keep the last result out of the cache: it is the output trace's before. L1's cache holds something
else, below.

**L1's cache**. The Bootstrap page's Figure 2 gives `k0_pass` a read-write slot for L1's cache, between
`cache_open` and `cache_commit`. L1 had no cache, so B6's "the cache is unchanged" check and L5 passed without
testing anything. The kept rows cannot go in the cache: P§3 makes them the output trace's before, and Operations
§6.1 keeps out of a cache anything an answer reads. §6.1 says what does go in: what an operator reads to go fast and
could make again, partitioned by file for a per-file operator, a file that goes taking its entries with it. For L1
that is each document's sizes:

- **What is cached:** the rows `k0_pass` gave each document, by relation. It is a cache entry with no columns (an
  order has one to four), keyed by the module (`cache::sizes`).
- **Written by** `k0_pass`'s fourth phase, `keep`, which grows the cache within its reservation:
  - the sizes come from the statements' places, end to start;
  - the gone documents' entries are let go first (`forget`; their modules are listed in control);
  - a new document takes a let-go entry's room.
  - A run that is refused, or whose runs were too small, keeps no sizes.
- **Read by** L1's delta, before it writes the cache. The room its runs need is each gone document's kept rows plus
  each added document's rows as they were, an eighth more. `Out::room` opens a port's runs at that room before the
  first write.
- **What it saves:** l0 gives 270 `k_arg` rows, past the first room of 256. An edit retracts 270 and adds about as
  many, so without the sizes every edit's run outgrew its runs and ran again, `k0_pass` and `drop_parts` included.
  The driver now names each such attempt as excess in `Work` (`OUTGROWN`).
- **L5:** dropped, the cache costs the next run an attempt, and its rows are the same.

The plain function (`run`) reads no cache; `k0_pass` keeps its sizes in a block of the launch's own.

Checks (Relax, local, 76 of 76):
- `k0_pass_keeps_each_documents_sizes_and_a_gone_one_takes_its_entry`: two documents' sizes equal the rows each gives
  alone. A gone module's entry is let go with no document read, and the next module takes its room (the cache's
  bytes unchanged).
- `l1s_cache_sizes_its_runs_and_dropped_costs_an_attempt_not_an_answer`:
  - a cold run outgrows its runs; the cache then holds l0's sizes, equal to its rows by relation;
  - an edit's run is one attempt;
  - with the cache dropped, the next edit's run takes an attempt more, and its change is the reference's.
- L1's harness now drops the cache at a random step for each of three seeds, not at steps 1, 3 and 5. Its L4 check
  ("the cache is unchanged") now compares the sizes too.
- The shape script refuses an L1 with no cache or none read, a `k0_pass` with no cache slot, and fixed drop steps.

**Slabs: the per-document slab taken out**. The slab change above (`8b5532c5b6`) reinvented what the residency
already does, and did it worse. In the residency (`rerail_slabs`), a slab is a 64 MB chunk block (`SLAB_BYTES`)
leased from the device pool. It is shared by many documents, and each source and each tape is a room in it: an
allocation of its own length, plus a word (`placed::place`, `tape_room`). Slabs are retired whole, by the budget and
least recent use. The change instead gave each document a slab of its own, reserved at the tape's bound of 80 times
the source and resident whole. `tape_block` is back to a sparse reservation in run memory, and the change's test is
gone. (Superseded by "Slab rooms, as the residency's", next: sources and tapes now lie in the residency's rooms.)

**Slab rooms, as the residency's (B4)**. Sources and tapes now lie as the residency places them (`rerail_slabs`):

- **Shared slabs.** Many documents share a slab, a block of `SLAB_BYTES` (64 MB, the residency's constant) or the
  need, whichever is larger.
- **One room each.** Each source and each tape is a room of the slab, written once: the observer's placed read
  writes the source, and the rr parser writes the tape (`InBlock`).
- **Made by the block policy.** `Directory::slab_room` makes a slab with `alloc::block` (`Need::SLAB`,
  `Device::May`). That is host memory, reserved as address space and resident as rooms are written, and a block the
  device reads where the run is bound to one. A slab does not live on the device to be usable by one; it only has to
  be laid out so a device can use it.
- **Sealed rooms are fixed.** A slab is never frozen while it takes more. Instead the directory seals it to what is
  written (`seal`), and `is_fixed(id, end)` holds for a frozen block or a sealed room. A blob is housed only from
  fixed bytes, and a reference may point into a sealed room (`Held::Slab` holds the slab).
- **A slab's room.** A document needs its source and the tape's bound in address space. One past that starts the
  next slab.
- **No more per-document blocks.** The 80-times sparse reservation is gone.

(Superseded by "Slabs retired whole", below: a run now retires the slabs no document names.)

Checks (Relax, local, 76 of 76):
- `documents_share_a_slab_their_rooms_sealed_as_written`:
  - three documents' tapes lie in one open slab of at least `SLAB_BYTES`, fixed up to what is written and not past
    it;
  - a block refers to a sealed room while the slab takes another document, and a launch reaches the slab through
    that reference;
  - a source past the slab's room starts the next slab.
- The earlier B4 checks hold over slab rooms, with sealed in place of frozen.
- The shape script refuses a per-document block for a document, and a documents' slab made otherwise than by the
  block policy, as the device may read it.

**Item 4 kept** (the user's answer, 2026-11-26): sources and tapes as rooms of shared slabs, as built in
`4b631818bc`.

**`DeltaOp` typed by its ports and its cache** (P§12 item 7: `type Ports; type Cache;`). The trait re-read above
found ports passed as values and an untyped `CacheW`. Now:

- **Port sets.** `type Ports: PortSet` is the operator's port set. It declares its ports (`ports()`) and their
  counts (`INS`, `GLOBALS`, `OUTS`). `In<'_, P>`, `At<'_, P>`, `Out<P>` and `Asks<P>` are typed by it, and their
  ports are read through the set's handles (`PortIn<P>`, `PortGlobal<P>`, `PortOut<P>`): `ins.port(L1Ports::TAPE)`,
  `ins.global(L1Ports::ATOMS)`, `out.block(L1Ports::relation(k))`. A handle of one operator's ports does not compile
  as another's, and no port is read by a bare index. An instance whose port set's ports are not its counts is
  refused.
- **Cache bodies.** `type Cache: CacheBody` is what its cache keeps, read through its own type:
  `CacheW<C>::body(cx, f)` gives `f` the body read where it lies, or `None` when the cache reads as ∅. L1's is
  `Sizes` (`sizes.of(d)`), the toy's is `Orders` (`orders.order(layer, cols)`), and `()` keeps nothing.
- **Asks.** P§12's `asks` is `Asks<P>`, one runs block per input port. Every run passes `Asks::none()`, since no
  operator asks of its inputs yet.

Checks (Relax, local): `a_port_set_whose_ports_are_not_its_counts_is_refused`. The toy's and L1's harnesses pass
over the typed contract. The shape script holds the trait to P§12's associated types and typed signatures, and
refuses a port read by a bare index.

(Superseded by "Retirement is the residency's", below. This was a policy of my own, not the residency's: it threw the
content hit away, and its test asserted that it did.)

**Slabs retired whole** (B4, the residency's retirement). A run starts by retiring what the document set's
committed state (the run's before) does not name (`DocSet::retire`). It does this once the set or the current slab
has moved since it last retired.

- **Live.** The live blobs are the source, tape and symbols of each row at the set's head.
- **Retired.** A block is retired when it holds none of them and is not the slab rooms go to now
  (`Blobs::retire`). The candidates are the blocks the blobs' rows name (slabs, and symbols blocks) and the
  directory's slabs (`Directory::slabs`). The second set includes a slab whose rooms were all housed already, which
  happens when a re-read in the racy window places the same bytes again.
- **Letting go.** A retired block's blob rows go (−1), and the directory lets go of its own handle, so the block
  goes once no reference or reader holds it. The observer's answers and the interned symbols that name a retired
  blob are forgotten (`Files::forget`, `Interned::forget`), so the path is read again when next asked.
- **When.** Retiring at the start of a run, rather than its end, keeps the last run's two states readable until the
  next run starts.
- **For tests.** `Directory::with_slab_bytes` sets a slab's size, so a test can give each document a slab of its
  own.

Checks (Relax, local, 78 of 78):

- `a_run_retires_the_slabs_no_document_names`:
  - each edit of l0 lies in a slab of its own, and after each run two slabs and six blobs are held (the run's
    before and after);
  - an unchanged run then retires the last edit's before, leaving the live document's three blobs, and over four
    more unchanged runs the slabs never exceed three (the live document's, the one a racy re-read emptied, and the
    current one);
  - going back to l0, whose blobs were retired, reads it again, and its 14 rules are the cold run's.
- The shape script refuses a run that does not retire.

## The plan, built whole (2026-11-26)

The comparison of what was built against the pages found seven departures and called three missing pieces
"deferred by the plan". The user agrees to no deferrals: every piece the pages give is built here.

**`intern` in its shape** (*Operations* §10: a hash build (18), then a scan (10) for new ids). It was one phase
with one item. Now:

- **hash** (an item a node, in place): probes each node's names in the atoms' hash table and writes the ids it
  finds. A name it does not find is marked for the scan: new for an identifier, variable or string, and to be found
  for an operator's token, which the scan finds and never makes. This phase writes no atom.
- **scan** (one item, ordered): interns the language's names on the first run that misses them, then the path and
  the module, then each marked name in node order, so new ids follow node order.
- Clippy's leftovers went too: two unused imports, an unused parameter, and the directory's holders marked as such.

Checks (Relax, local): `interns_hash_then_scan_alike_at_every_tiling_new_ids_in_node_order`. At tiles of 1, 2 and
7, shuffled and not, the symbols and the atoms equal the whole run's, and new names take ids in node order. B5's
four tests pass.

**Per-column stats of a chained view** were listed as open (the rebuild plan's B2: "Not built: per-column stats").
They were built on 2026-11-25 (`996f410a59`, `View::chain_stats`: the bounds over the layers, and the sketch of the
state's head), and `a_random_trace_reads_as_its_set` checks rows, distinct values and bounds at every state. The
comparison that listed them as open was wrong.

**Blobs in the blocks crate** (P§12: the interfaces "live in the blocks crate, beside the layer implementation they
wrap"). `rerail_blocks::blob` holds `BlockId`, `BlobId` (with `of`, the sha256 of bytes), `Placed` and the `Blobs`
trait (`get`, `house`, through a home's directory and registry). `delta_exec`'s home of them is `BlobRows`, the rows
in a trace, and implements the trait. The directory's `BlockId` is the blocks crate's. The shape script refuses a
second definition of any of these names in `delta_exec`. Relax, local: 79 of 79.

**What lasts across processes** (*Operations* §5: the file layer kept as is; P§11.4: "leaves read: path, digest,
hit or miss" and "blobs, plus a trace from key to blob id"; P§12 item 7). Two maps of the crate's own stood where the
pages have traces (`Files.kept`, `Interned.syms`). The objects store, documents' places and the journal were not
used, and nothing survived a restart. Now `files::World` is the world a run reads, kept in a `Store` folder:

- **The store's folder** (0700, decision 0574):
  - the content store's objects (`objects.db` and its segments);
  - the observer's answers beside it (`objects.db.files`, the P0 store, which keeps them in the content store's
    facts);
  - the store's atoms (`atoms.blk`, mapped writable: decision 0401's ids, which last);
  - the kept traces' blocks (`kept/`, a file a block).
- **Answerers** (P§12 item 7: `Answerer`, `AnswerKind::{World, Pure, SubRun}`):
  - **load** is a World answerer. Under the observer's journal (`begin`, `watch`, `flush`: decision 0105) it writes
    each leaf it reads to the leaves trace `(d, digest, found)`, misses too (D3). An unchanged leaf writes nothing.
  - **parse** is Pure, keyed by (content, grammar). `rr_syntax::grammar()` is the parser's version: the sha256 of
    its sources. A hit skips the read and the parse (*Operations* §6.2): the tape is either resident, or loaded from
    the store's objects into a slab room (a load to the exact bytes, not a copy, 0353). A miss parses, then puts the
    source and tape in the store's objects (`put_source_and_tape`).
  - **intern** is Pure, keyed by (tape, path, module). Its answer is kept by key (D4) while the store's atoms keep
    their ids. Its symbols block is a kept block.
  - The answers trace keeps parse's and intern's answers, by answerer and key.
- **Documents' places** (0494): a trace `(d, source's segment, the tape's hash and segment, its length)`, from the
  store's new `ContentStore::document_places(sha256, grammar)`. A later process reads a document's source and tape
  where the segments hold them, with no SQL.
- **The document key** (0052 §3) comes from the path's hash: `rerail_blocks::blob::doc_key`, the next free key on a
  collision. It is one definition, which `corpus_memo::doc_keys` now calls too. It is no longer the module's atom id.
- **The document set's row** (P§11.4: path → tape id, grammar) is `(d, path, grammar, source, tape, symbols)`, with
  a hash index on the path's symbol. A document is found by its path.
- **Kept traces.** Every trace a run writes is placed where the directory keeps what lasts (`Directory::lasting`):
  - the document set;
  - L1's outputs;
  - the leaves, the answers and the places;
  - the blobs' rows.
- **The pins.** A run's pins are kept as a block that refers to every head (`Pins::keep`).
- **Opening the store** (`contract::Bench::open`):
  - each kept block is mapped where it lies (`Directory::open_kept`);
  - the newest pins are found among the unreferred blocks (`Pins::open`);
  - every kept block they do not reach is let go, its file removed, except a block a blob's row names;
  - the blobs' rows of blocks no process keeps (slabs) are forgotten.
- **Rolling up the set.** The document set rolls up at a run's end, once its delta is read (`Trace::roll_up_run`).
  The run's layers merge into one, then the geometric policy applies below, so the next run's before is one head.
- `Files` and `Interned` are gone, and their maps with them.

**Retirement is the residency's** (`rerail_slabs`, its policy as it is; the user, 2026-11-26). The directory's slabs
are held by `residency::Residency`, not by the directory:

- **Retired under the budget only.** A slab is retired only while what is held and a new slab pass the budget, and
  then the least recently used slab no reader holds goes. The budget is `rerail_accel::budget::resident`, the
  residency's share. That function and `SLAB_BYTES` and `DROP_AFTER` moved into `rerail_accel::budget`, one
  definition, which `rerail_slabs` now takes.
- **What counts as used.** A run uses each live document's slabs (`DocSet::resident`).
- **The drop.** Everything is dropped `DROP_AFTER` after the last execution, by the residency's own thread, as
  `rerail_slabs::executed` does.
- **The content hit is kept.** A document whose slab went is loaded from the store's objects where its places say,
  not parsed.
- **The lock.** The residency's state is shared with its drop thread, behind a lock, as `rerail_slabs`'s is. It holds
  slabs, not claims, so it lives in a module of its own and the directory's and traces' code stays lock-free.

Checks (Relax, local, 84 of 84):

- `every_leaf_read_is_recorded_misses_too`.
- `a_content_answered_before_is_never_parsed_again`. The same content is not parsed again in any of three cases:
  - at another path (a resident hit);
  - after the residency's drop (loaded from the store's objects);
  - in a new world over the same store.
- `the_residency_retires_the_least_recently_used_slab_only_past_its_budget`. Nothing is retired within the budget.
  Past it, the least recently used slab goes, and a slab a reader holds is skipped.
- `a_run_retires_the_least_recently_used_slab_past_the_budget_and_keeps_the_content_hit`. This replaces
  `a_run_retires_the_slabs_no_document_names`. l0's slab is retired past the budget, and going back to l0 loads it
  without a parse.
- `the_residency_drops_all_after_a_run_and_the_next_run_loads_again`.
- `a_later_process_starts_where_the_last_run_left_off`. A second bench is opened over the store's kept folder, with
  the observer forgotten, as a new process has none:
  - L1's outputs are where the run left them, and the atoms keep their ids;
  - the first run is L4's identity, checks only, with nothing parsed or interned and no read but the observer's
    sampled check;
  - an edit then gives its delta, 15 rules.
- `an_unchanged_tape_interns_nothing`: the intern answer by key.
- The shape script's new `kept` check holds each of these pieces. Its retirement check now refuses a run's own eager
  retirement and requires the residency's policy.
- Its patterns were narrowed, not dropped, where they had matched the pages' own names:
  - `World` is B§8's `world`, so the check refuses a `World` trait or `dyn World`, not the name;
  - `Trace::resumed` replaces `at`, the frontier name, and is a constructor;
  - `roll_up_run` replaces `settle`.
- Clippy is clean on `delta_exec` and the twins, the earlier warnings included.

