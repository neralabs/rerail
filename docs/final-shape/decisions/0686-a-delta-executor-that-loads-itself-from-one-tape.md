# 0686: A delta executor that loads itself from one tape

Status: being rebuilt to the agreed design, milestone by milestone; M0 (the clean slate) done. Design pages: *Bootstrap
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

- **M0** the clean slate. **M1** traces and bistates in `blocks` (P§12 items 2–5; geometric roll-up). **M2** the
  operator contract and one harness for L1, L3, L4, L5. **M3** a rule body as composed steps. **M4** Δwave (L7 a
  combinator; program δ by module). **M5** Δfixpoint (the wave repeated; levels for removals). **M6** Δanswer and the
  World. **M7** Δphase (strata from `k_order`; the refusals). **M8** L1 as an operator, the export, B§8's loop.
  **M9** program δ through the chain. **M10** the first K1 feature (the binding lowerer, four stratum files).

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

- E4: "no idea, be thorough". The survey is to be done again, thoroughly.
- F1 (a search with several valid answers): "so may a CPU search, so what? ANything numeric can vary. The results
  need to be correct; determinism is nice to have but not required". This revises D2: a result must be correct on
  every executor and tiling; the same bytes are not required. Tests check correctness (the same set of facts, a valid
  answer, a numeric result within its bound), not bytes.
- F2 (B§5's `load(path, k)` against the engine's `load(path, module, grammar)`): "use something different for
  bootstrapping than load, we already agreed before to not have a separate strata fact". The bootstrap's files are
  `boot(path, k)`, a file's place in the bootstrap being its stratum (O4), and `k_order(d, k) :- boot(p, k), doc(d,
  p, _).`; `load` is the engine's. `check_delta_exec_shape.py` refuses `load(` in the crate's `.rerail` files.
