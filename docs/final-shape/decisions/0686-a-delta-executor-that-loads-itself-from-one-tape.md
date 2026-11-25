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
  time; `ViewColumn` reads down the chain). An ordered scan and a probe of a chained view are not built: they refuse.
  The old engine's views and forks read as before (a full relation, a base and delta, a fork with no kills).
- **The change** is `View::bistate` of the last layer: 0678's `Bistate`, the one definition, before the state below and
  after the state with the layer. Its gone rows are the layer's `changed` ids, so it is read in its size;
  `is_identity` answers from the counts alone (no id flipped and no row of its own) before any row is read.
- **`LayerW`** is `RelW` on the open layer: rows pushed, rows of the state killed.
- **Freeze over a chain.** `freeze_in` of a continuation given its base finds a pushed row's duplicate in any layer
  below (each layer's row index, or its sort range for an ordered one), live in the state below; a row killed and
  pushed again in one layer is brought back (its kill cancelled), so the change is the least one. Such a layer gets a
  row index too. A freeze with no base, or of a relation that is not a continuation, is unchanged.
- **Partitions** on a column the trace names (the module, for P2), with the partition index on each layer. Beside each
  layer, `consolidate` writes `parts(part, digest, added, gone)`, the digest's move a sum of `row_digest` (no order
  needed), and `gone(part, id)`, the layer's kills by partition, each frozen with an index on the partition: derived
  facts written once where computed. `restrict(p)` reads partition `p`'s rows of the last change only, through those
  indexes; `part_digest(p)` sums the layers' moves; `part_ids(p)` reads the state's rows in `p`.
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
- [x] A change restricted to a partition is that partition's added and gone ids, read through its indexes alone (the
  ids it reads are the ids it returns; counted by `restrict`, not measured); its digest is its rows' digest.
- [x] A round that changes nothing (nothing pushed, only rows held, a row killed and pushed again) is the identity by
  its counts, with no row of its own and no partition changed.
- [x] `consolidate` runs as an operation on the trace's block, in place; a reader holding the block refuses its start.
- [x] One `Bistate`, in `blocks`; no frontier API in the trace; its layers continue `Relation`s; no second trace or
  bistate in `delta_exec` or the twins (`check_delta_exec_shape.py` section `b2`; tried against a second `Bistate` and
  a `fork`: both named).
- [x] blocks `--lib` 124/124 (its relation and bistate tests unchanged); delta_exec `--lib` 11/11.

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
