# Campaigns: the open work, grouped by the goal it serves

Status: a tracker (2026-11-23). Priority is the user's to set, in the table's last column.

## Intro

A campaign is one of the user's goals and the decision records that serve it. This file groups the
374 records that were open before the audit of 2026-11-23. That audit checked each against the code
and set its status. For each record, it also read the session that wrote it, to see where it came from:

- **a, you asked:** your own words asked for this, or for its direction.
- **b, proposed, you agreed:** the assistant offered it and you agreed, or picked from options the
  assistant framed.
- **c, the assistant's own:** no message of yours asked for it or approved it. The assistant chose it
  while working towards a goal of yours, which is named.

Of the 374: 174 a, 169 b, 31 c. By state: 291 built, 61 partly built, 1 not started,
19 superseded, 2 analysis. Each record's origin, with the words it rests on, is in
`notes/t54-decision-origins.md`.

The 163 records already marked built, done, withdrawn or superseded before the audit are not in these
counts. Their origins were not read.

## Open, underway and closed

Every record has one of three states now (2026-11-23, 15:20), read from its own status line in this tree, which has
merged the hub, django-bugs, Native-and-GPU-strata and Typing-support:

- **closed:** built, done, superseded, withdrawn, removed, or an analysis or measurement that answered.
- **underway:** not closed, and an active worktree's own records name it ("Who works on what", below).
- **open:** not closed, and no worktree is on it.

Of all 568 records: 477 closed, 15 underway, 76 open. A campaign is closed
when every record of it is closed and a check keeps its invariant ("How to close them").

## The campaigns

| Campaign | Records | Closed | Underway | Open | a / b / c | Priority |
|---|---:|---:|---:|---:|---|---|
| [Generated parsers in the shape of simdjson and the lean parsers](#parsers-shape) | 88 | 80 | 1 | 7 | 28 / 56 / 4 | |
| [Fast parsers for every product language, on every host](#lean-parsers) (closed 2026-11-23) | 8 | 8 | 0 | 0 | 5 / 3 / 0 | |
| [One tape format and one block layout, read in place by CPU and GPU (never copied)](#tape-blocks) | 26 | 20 | 1 | 5 | 17 / 8 / 1 | |
| [An edit costs what it changes](#edit-cost) | 22 | 14 | 0 | 8 | 7 / 14 / 1 | |
| [The store: caches are facts, writes keep up, growth is bounded](#store) | 29 | 22 | 1 | 6 | 21 / 6 / 2 | |
| [Cold runs fast: every core used, the critical path short](#cold-run) | 32 | 30 | 0 | 2 | 12 / 17 / 3 | |
| [The planner: never costs more than it saves, improves from feedback](#planner) | 17 | 13 | 0 | 4 | 14 / 1 / 2 | |
| [Rules on Metal: label walks, device waves, bounded launches, memory budgets](#metal-rules) | 49 | 43 | 2 | 4 | 16 / 22 / 11 | |
| [Any GPU, and the CPU's SIMD units: kernels written once in UKL](#any-gpu) | 2 | 0 | 2 | 0 | 2 / 0 / 0 | |
| [A C/C++ preprocessor on the CPU and every GPU](#preprocessor) | 17 | 15 | 0 | 2 | 10 / 7 / 0 | |
| [Main's rules run directly, as Datalog (.rr), with main's findings and W's speed](#rules-parity) | 43 | 36 | 0 | 7 | 21 / 20 / 2 | |
| [Embedded languages: strings parsed as their language, no guest documents](#embedded) | 9 | 8 | 0 | 1 | 5 / 4 / 0 | |
| [Typing and libraries: cross-file facts as ordinary facts, libraries held once](#typing) | 7 | 3 | 0 | 4 | 4 / 1 / 2 | |
| [Programs as flat images and versions, passes over them, never text](#programs) | 11 | 9 | 0 | 2 | 6 / 3 / 2 | |
| [Work bounded by its readers: nothing widened past what is read, no widened output kept](#widening) | 8 | 6 | 1 | 1 | 3 / 5 / 0 | |
| [A re-entrant engine with bounded memory and no global state; one shape across the worktrees](#hygiene) | 6 | 5 | 0 | 1 | 3 / 2 / 1 | |

### Underway, across the campaigns

| Record | Campaign | Worktree | Left |
|---|---|---|---|
| [0104](decisions/0104-everything-kept-is-facts.md) Everything kept is facts, on the same fact path | [store](#store) | Typing-support | an unchanged run's fixed cost, so the kept frames can go, and the kept entries read in place (t33, "K4: what is left") |
| [0353](decisions/0353-blocks-are-never-copied-every-reader-reads-in-place.md) Blocks are never copied: every reader reads the data where it lives | [tape-blocks](#tape-blocks) | Native-and-GPU-strata, Benchmarks-and-parity, wolfeel | 5 marked copies, step 3's one (core.rerail's per-node rows) and step 4a's four (a queued tape not in a room, the parsers' columns, a source made in memory) |
| [0414](decisions/0414-a-grammars-proofs-are-answers-by-content.md) A grammar's proofs are answers by content | [parsers-shape](#parsers-shape) | Two-pass-roots-and-cross-file, Swift-Kotlin-Scala-support | Answers read from and written to the engine build's content store for every grammar compiled for a run (Q234), not only Kotlin's shipped seed |
| [0440](decisions/0440-gpus-beyond-metal-through-ukl.md) GPUs beyond Metal: kernels written once in UKL, a device layer under … | [any-gpu](#any-gpu) | CUDA-support | step 10 block directory/coherency across domains, 11 partitioned memory on discrete GPU, 12 multi-GPU/host; ROCm/HIP and WebGPU backends |
| [0540](decisions/0540-a-device-wait-is-on-a-running-thread-and-every-launch-has-a-budget.md) A device wait is on a thread that is running, and every persistent … | [metal-rules](#metal-rules) | django-bugs | Step 4 (a 16z block decoded by its SIMD group together); the resident interpreter's Metal gate; the preprocessor's own MSL still waits by polling |
| [0567](decisions/0567-ukl-on-the-cpu-is-vector-code-between-sync-points.md) UKL on the CPU is vector code between sync points, made by … | [any-gpu](#any-gpu) | uglykl-CPU-SIMD-parallelization | Step 3: own register allocator, per-call saves, Cranelift from SSA, the Rust SIMD back end, and the 1000x gate |
| [0597](decisions/0597-a-guest-context-is-a-pass-of-its-own-rules-reading-its-host-explicitly.md) A guest context is a pass of its own rules, reading its host's … | [widening](#widening) | cost-classifier | Step 4: remove hiding's annotation range, FoldOwn, narrow copies and one_document (still in the code) |
| [0603](decisions/0603-a-stratums-device-work-is-one-launch-a-work-queue-whose-rounds-are-continuations.md) A stratum's device work is one launch: a work queue whose phases and … | [metal-rules](#metal-rules) | Two-pass-roots-and-cross-file, django-bugs | 5d's other half (the lattice fold still reads host tables), 5e (an index sorts claimed rows on the device), the closure launch test, non-recursive heads' rooms. |

### Not closed, in no campaign (29)

Records the audit of 2026-11-23 did not take (written since, or merged in from a branch), whose status is not
closed. Each is a candidate for a campaign.

| Record | State | Worktree | Status |
|---|---|---|---|
| [0285](decisions/0285-a-small-runs-fixed-cost.md) a small run's fixed cost | open |  | built in part (2026-11-15). Amends the cost statistics (decision 0100 §14) and the checkout's finding (`source_tree`, decision 0224). |
| [0388](decisions/0388-a-held-view-has-its-inputs-old-rows-and-delta.md) A view the run holds of an input's columns has its input's old rows … | open |  |  |
| [0564](decisions/0564-the-token-loop-keeps-the-gap-masks-and-the-semicolons-fills.md) The token loop keeps the gap masks and the semicolon's fills | open |  | decided (the user, 2026-11-21: "Next: step C item 3, the escapes tested over code … Then the template masks, which need state kept in the … |
| [0566](decisions/0566-a-shift-outside-the-regions-is-of-the-masks-outside-values.md) A shift outside the regions is of the masks' outside values | open |  | decided (the user, 2026-11-21: "… After those, nothing reads the regions' walk and it can go (step D)", the instruction "do this"). The … |
| [0572](decisions/0572-a-region-tokens-parts-write-tier-2s-fields-and-test-its-closes.md) A region token's parts write tier 2's fields and test its closes | open |  | decided (the user, 2026-11-21: "Next: the same shift rule inside a region's body, which covers the raw-string and character escapes and … |
| [0574](decisions/0574-a-mapped-file-and-its-folder-are-the-users-alone.md) A mapped file and its folder are the user's alone | underway | Native-and-GPU-strata |  |
| [0587](decisions/0587-an-escape-of-the-code-is-tested-in-the-tokens-it-may-hold-in.md) An escape of the code is tested in the tokens it may hold in | open |  | decided (the user, 2026-11-21: "maybe compare the rust code", on choosing the next step by the lean parser's Rust). Follows 0609 and 0608 … |
| [0589](decisions/0589-a-mark-of-rows-is-the-picked-rows-constant.md) A mark of rows is the picked row's constant | open |  | decided (the user, 2026-11-21: "now please again compare the rust code"). Follows 0614 (the first byte's group picks the row). |
| [0590](decisions/0590-an-escape-at-a-tokens-start-is-tested-in-its-rows.md) An escape at a token's start is tested in its rows | open |  | decided (the user, 2026-11-21: "now please again compare the rust code"). Follows 0589 and 0587 (an escape of the code is tested in the … |
| [0593](decisions/0593-the-token-loop-reads-a-blocks-first-bytes.md) The token loop reads a block's first bytes | open |  | decided (the user, 2026-11-21: "please address these", of four differences from the lean lexer found by comparing the Rust; this is the … |
| [0594](decisions/0594-a-kept-fills-value-is-made-in-the-rows-that-read-it.md) A kept fill's value is made in the rows that read it | open |  | decided (the user, 2026-11-21: "please address these": the second). Follows 0593, 0609 (the type after `as` a kept fill) and 0564 (the … |
| [0595](decisions/0595-a-templates-span-is-tested-in-the-rows-it-may-hold-in.md) A template's span is tested in the rows it may hold in | open |  | decided (the user, 2026-11-21: "please address these": the third). Follows 0594 and 0562 (the loop keeps a template's code as its own … |
| [0596](decisions/0596-the-openers-pick-the-rows.md) The openers pick the rows | open |  | decided (the user, 2026-11-21: "please address these": the fourth, "strings and comments are opened as numbered regions with `plain` … |
| [0598](decisions/0598-a-tail-goes-back-to-the-string-its-template-closed.md) A template's tail goes back to the string its template closed | open |  | decided (the user, 2026-11-21: "go ahead with the full change", on reading strings as the lean lexer does; this is its first step). … |
| [0607](decisions/0607-a-grammar-states-what-the-token-loop-knows-and-the-compiler-checks-it.md) A grammar states what the token loop knows, and the compiler checks it | open |  | decided (the user, 2026-11-21: "please do 1 / to make it easier for the compiler, we can also add information in the bitlex grammar about … |
| [0608](decisions/0608-the-last-escapes-leave-the-walk-and-the-walk-is-the-test-builds.md) The last escapes leave the walk, and the walk is the test build's | open |  | decided (renumbered from 0573, which another worktree holds; the user, 2026-11-21: "go ahead", on converting the two escapes left on the … |
| [0609](decisions/0609-the-loop-keeps-the-type-after-as-a-fill-within-a-kept-fill.md) The loop keeps the type after `as`, a fill within a kept fill | open |  | decided (the user, 2026-11-21: "go ahead", on keeping the `as` escape's state token by token in the loop, as the semicolon's fills, in … |
| [0610](decisions/0610-the-gap-before-a-token-is-scanned-once-a-mask.md) The gap before a token is scanned once a mask | open |  | decided (the user, 2026-11-21: "now please again compare the rust code"). Follows 0590. |
| [0613](decisions/0613-macro-expansions-are-memoized-requests-run-in-waves-their-results-a-tree.md) Macro expansions are memoized requests, run in waves; their results … | underway | Go-C-and-C-support | proposed (2026-11-22, the user: "macro expansion should be a) something that we can memoize and b) something that we can do in parallel; … |
| [0614](decisions/0614-a-tokens-first-byte-picks-its-region-and-row-by-nibble-tables.md) A token's first byte picks its region and its row by nibble tables | open |  | decided (the user, 2026-11-21: "no, use simdjson's approach for this where possible. We already have done some of the proving … |
| [0615](decisions/0615-the-token-loop-keeps-a-templates-code-as-its-own-state.md) The token loop keeps a template's code as its own state | open |  | decided (the user, 2026-11-21: "Next: step C item 3, the escapes tested over code … Then the template masks, which need state kept in the … |
| [0616](decisions/0616-a-sessions-corpus-batch-takes-its-unchanged-groups-from-the-last-run.md) A session's corpus batch takes its unchanged groups from the last run | open |  | accepted (2026-11-22). Amends 0494 (a batch makes a document when first read) for a session's corpus batch. |
| [0617](decisions/0617-matrix-units-through-ukl.md) Matrix units through UKL: native-shaped fragments, owned by one or … | underway | CUDA-support | decided (2026-11-22), by the user's answers: |
| [0623](decisions/0623-rrs-generated-rust-beside-the-lean-parser.md) `.rerail`'s generated Rust, read beside the lean parser | underway | Parser-generators, Swift-Kotlin-Scala-support | the comparison is made, and the order below is done. The item is the user's (2026-11-22): "please figure out how JIT is faster than AOT, … |
| [0628](decisions/0628-the-planners-statistics-and-the-tapes-code-tables-in-blocks.md) The planner's statistics are overlay blocks; a tape's code table is … | underway | Two-pass-roots-and-cross-file, Swift-Kotlin-Scala-support |  |
| [0638](decisions/0638-format-16-codes-are-the-grammars-numbered-by-a-corpus.md) Format 16's codes are the grammar's, numbered by a corpus's counts | underway | Bend-language-investigation | decided (2026-11-23), by the user: "you may ORDER the codes based on a corpus, so long as it's large (we have a 502-repo corpus and the … |
| [0641](decisions/0641-a-programs-phases-are-indirect-on-metal-too-its-kernels-call-free.md) A program's phases are indirect on Metal too: its kernels call-free | open |  | accepted (2026-11-23). Amends 0648 (a program's levels are stacked dispatches): its Metal exception, a phase dispatched directly with its … |
| [0647](decisions/0647-a-rounds-phases-cost-their-work-in-one-group-not-their-dispatches.md) A round's phases cost their work in one group, not their dispatches | open |  | spike (2026-11-23). Measured; nothing here changes 0641's times past the M3's run-to-run noise. Each of the three mechanisms below is in … |
| [0648](decisions/0648-a-programs-levels-are-stacked-dispatches-one-launch-a-stack.md) A program's levels are stacked dispatches, one launch a stack | underway | django-bugs | accepted (2026-11-23), amended by 0641: Metal dispatches a phase indirectly too, its program's kernels call-free (the M3 hung at an … |

## How to close them

A campaign is closed when its invariant holds everywhere in the code, and a check keeps it holding without
exceptions. Three invariants already work this way: a check refuses a new violation, and a pinned list of the old
ones only shrinks. For those, the open work is a countdown. The other campaigns have tests for what is built, but
nothing refuses a new violation or counts the old ones. So for each of them **the first step is the check**. It
lists today's violations as its pinned exceptions, and from then on the work is a countdown too. The check each one
needs is under "Check to add" below.

### Where the counts stand

This tree has merged the hub (`final-shape-serverplan-quality`), django-bugs, Native-and-GPU-strata and
Typing-support (2026-11-23), so it is the furthest along of any branch. The records' statuses are this tree's. These
are the counts here, on the hub, and on the branch furthest along:

| Count | Here | The hub | Furthest | Where |
|---|---:|---:|---:|---|
| Marked copies (0353) | 5 | 27 | 5 | here (Native-and-GPU-strata merged) |
| Tables not yet facts (0104) | 0 | 8 | 0 | here (Typing-support merged) |
| Maps not yet facts (0104) | 0 | 6 | 0 | here (Typing-support merged) |
| Shared statics (0442) | 4 | 4 | 4 | none |
| `FoldOwn` references (0597) | 1 | 1 | 0 | cost-classifier |
| `HIDDEN_KEYS` references (0597) | 16 | 16 | 16 | the hub |
| `rr_narrow_` references (0597) | 15 | 14 | 14 | the hub |
| `rr_long_text_node` references (0174 step 3) | 25 | 20 | 18 | wolfeel |
| `rr_wide_` references (widened requests) | 44 | 44 | 44 | none |
| `combine_read` references (0246 stage 3) | 2 | 2 | 2 | none |

### The order of work

1. **Merge what is done (done 2026-11-23).** This tree merged the hub, then django-bugs, Native-and-GPU-strata and Typing-support.
   - **tape-blocks:** Native-and-GPU-strata: the marked copies from 27 to 5.
   - **store:** Typing-support: the tables and maps not yet facts from 8 and 6 to 0.
   - **metal-rules:** django-bugs: its 0638 renumbered 0648 (stacked dispatches).
   - Left: Swift-Kotlin-Scala-support's 0591, which collides with the hub's.
2. **Closed (2026-11-23).**
   - **lean-parsers:** Every record built; kept by the byte tests on every backend.
3. **The short ones.** Each has one or two open records, and none waits on the two hubs.
   - **widening:** 0597 steps 4–7 are in the hub. Left: the remaining `HIDDEN_KEYS`, `rr_narrow_` and `one_document` references, then 0174 step 3 (long text on request), which also removes 0353's step 3 copy.
   - **embedded:** 0431's four loose ends.
   - **hygiene:** The 4 `shared` statics, then 0094 §5–§6. No worktree is on it.
   - **programs:** 0246 stage 3, which final-shape-rr-align is specifying as 0639; then 0281 §3.
   - **preprocessor:** Go-C-and-C-support is on 0613. Header units, the stored objects and the words on Metal have no worktree.
   - **cold-run:** 0113, then 0114. No worktree is on it.
4. **What is left of the hubs.**
   - **tape-blocks:** 0353's last 5 copies: step 3, which is three worktrees' row 7, and step 4a.
   - **store:** 0104's 6 maps, which are 0628's and 0387's statistics and kept plans.
5. **What the hubs free.**
   - **tape-blocks:** 0266, 0260, 0458, 0336, 0156.
   - **store:** 0115, 0053 §7, 0142, then 0164 and 0190.
   - **planner:** 0387 (Two-pass-roots-and-cross-file's 0628 is its first half), 0361, 0354, 0202.
   - **metal-rules:** 0549, 0540, 0095, 0098. django-bugs has 0603 done and 0540's gate passing.
   - **edit-cost:** 0090, 0092 and 0093 in that order, then 0116, 0117, 0225, 0328, 0132. No worktree is on it.
   - **typing:** 0083 and 0065 on 0090 and 0092's per-definition deltas, then 0089 and 0047. No worktree is on it.
6. **The long ones.**
   - **parsers-shape:** Kotlin's chain (Swift-Kotlin-Scala-support); 0623 and 0618 (Parser-generators); 0414; then Swift and Scala.
   - **rules-parity:** Its device forms go with Metal's waves; W's speed (wolfeel, Bend-language-investigation); then 0044 and 0563.
   - **any-gpu:** 0440 step 10 after 0353; 0567 step 3 (uglykl-CPU-SIMD-parallelization); 0617 (CUDA-support).

Some open items reach past their campaign's invariant: a second hash, the Neural Engine lane, multi-GPU, more
backends, deferred device functions. Each campaign lists them under "Beyond the invariant". Whether they stay in it
is your call. Kept, the campaign closes when they are done. Made records of their own, it closes without them.

## Who works on what (surveyed 2026-11-23, 15:20)

Each active worktree, the campaigns its work since 2026-11-16 serves, and what it is doing. "Not here" counts
its commits this tree has not taken. Worktrees idle since 2026-11-20, and detached scratch checkouts, are left out.

| Worktree | Campaigns | Doing | Records | Not here |
|---|---|---|---|---:|
| Native-and-GPU-strata | [tape-blocks](#tape-blocks) | Removing 0353's copies: steps 3-8 merged here (27 marked copies down to 5: step 3's one and step 4a's four). Now asked for rows 7 and 17. Merged here at baf42d65cf (2e9760dbb2); its 43 newer commits are not. | 0353, 0574 | 43 |
| Benchmarks-and-parity | [tape-blocks](#tape-blocks), [rules-parity](#rules-parity) | The token relations, `visit_index`, `tree_digest`, `structural_before`/`after` and `doc` made views of the tape (0353 row 7); heap blocks moved into the run's blocks; parity held at 463 findings. 71 commits behind this tree. | 0353 | 13 |
| wolfeel | [rules-parity](#rules-parity), [tape-blocks](#tape-blocks) | The Python rules moved from Rust to `.rerail`, and their speed back: the gap to W measured (t53), sealed chunks (0622), a node read in one step (0640), a probed node's text off its tape (0353 step 3). 835 commits behind this tree. | 0611, 0622, 0640, 0353 | 41 |
| Two-pass-roots-and-cross-file | [planner](#planner), [tape-blocks](#tape-blocks), [metal-rules](#metal-rules) | The planner's statistics as overlay blocks (0628); `BlockVec` folded into `Vector`, one run room; a closure's levels as one launch (0601, 0603); a `.rerail` file read as every document is (0569 §7). Now running the benchmarks. 80 commits behind this tree. | 0628, 0569, 0603, 0414 | 6 |
| Typing-support | [store](#store) | t33 steps 1-8 and K1-K4, merged here: no table and no map is left not yet facts (the hub's: 8 and 6). A session keeps its store, its connections and its last corpus batch. Now: K4's fixed cost, so the frames can go. Merged here at acdd65304e (6f277440ff), with 0616's corpus batch kept in its session (the user's call). Despite its name, its current work is the store's. | 0104 (t33) | 6 |
| final-shape-rr-align | [programs](#programs), [store](#store) | Draft 0639: a program linked from module images that phase 0 plans and keeps as facts. This is 0246 stage 3, specified, with no code yet. The session is More-ergonomic-rule-syntax's. | 0639 (draft) | 13 |
| cost-classifier | [widening](#widening), [embedded](#embedded) | 0597 steps 4–7: hiding's annotation range, `FoldOwn` and the narrow copies removed; f-strings and Vue as contexts; a pass's tape-local rows kept per tape value. All in the hub. Now: a Vue file's embedded script read in place on Metal. | 0597 | 3 |
| Bend-language-investigation | [rules-parity](#rules-parity), [tape-blocks](#tape-blocks) | Format 16's codes numbered by the sampo corpora's counts (its 0638); a node view's membership test as its own read. Now: the probe code. | 0638 (its own) | 10 |
| django-bugs | [metal-rules](#metal-rules) | 0603 done; 0637 (idle groups poll, bounded); 0648 (a program's levels as stacked dispatches, one launch a stack, merged here); 0540's resident interpreter gate passes. Now: why Metal's indirect dispatch hangs. Merged here at 7155e34bad (ff39158899); its 0638 renumbered 0648. | 0603, 0637, 0648, 0540 | 1 |
| Go-C-and-C-support | [preprocessor](#preprocessor) | 0613: macro expansions as memoized requests run in waves on the GPU, their results a tree. 979 commits behind this tree. | 0613 | 5 |
| Parser-generators | [parsers-shape](#parsers-shape) | 0623: `.rerail`'s generated Rust read beside the lean parser, tier 1 and tier 2 against lean's; 0618 tier 2 as one loop over tier 1's records (0496's direction). | 0623, 0618 | 24 |
| Swift-Kotlin-Scala-support | [parsers-shape](#parsers-shape) | Kotlin read as the lean parser reads it (0621, 0624–0636). Now: the proofs' cost (0414). 1,158 commits behind this tree; its 0591 collides with the hub's. | 0621–0636, 0414 | 21 |
| CUDA-support | [any-gpu](#any-gpu) | 0617: matrix units through UKL. Now: kernels compiled for each GPU family in production builds (Apple M families, CUDA, ROCm). | 0617, 0440 | 9 |
| uglykl-CPU-SIMD-parallelization | [any-gpu](#any-gpu) | 0567 step 3: the CPU device runs kernels as `rs.rs`'s Rust. Now: the CPU's join path through UKL, the same kernel as the GPU's. | 0567 | 5 |
| Final-shape-quality | [hygiene](#hygiene) | The hub: roll-ups of every branch, from the Pulling-it-all-together session. Merged here (94de18b47a); its 15 newer commits are not. |  | 15 |
| engine-crating | none | The crate split, step 10 (build time; in no campaign). | crate-split plan | 13 |
| build-and-test-loop-time | none | The build farm (in no campaign). |  | 0 |
| oscar (this tree) | [hygiene](#hygiene) | The audit, the campaigns and this plan; the merges of the hub and the three branches. The hub, django-bugs, Native-and-GPU-strata and Typing-support merged here 2026-11-23. |  | 0 |

**Where worktrees overlap:**

- **0353 step 3, row 7: the per-node rows made views of the tape.** Native-and-GPU-strata (asked for rows 7 and 17), Benchmarks-and-parity (token relations, `doc`, digests as views), wolfeel (a probed node's text off its tape).
- **A device program's levels as one launch.** django-bugs (0603 done, then 0638's stacked dispatches), Two-pass-roots-and-cross-file (the closure's levels on a work queue), cost-classifier (accel's graph closure as one launch, in the hub).
- **Statistics and kept plans as blocks.** Two-pass-roots-and-cross-file's 0628 is this tree's 0387 and 0104 step 6 (the 5 maps `KeptPlans`, `SESSION_PLANS`, `Memoized`, `SESSION_MEMO`, `ObservedAt`).
- **Linking from module images.** final-shape-rr-align's draft 0639 is 0246 stage 3, which 0225 §7 waits on.
- **Record numbers used twice.** 0638: Bend-language-investigation's (format 16's codes numbered by a corpus) is in the hub, so django-bugs' (stacked dispatches) needs a new number. 0591: Swift-Kotlin-Scala-support's (the gap before a token scanned once, a mask) against everyone else's (tier 1's UTF-8 check in simdjson's form).

**No worktree on:** edit-cost, cold-run, typing, embedded (past 0597), hygiene (the 4 statics, 0094).

<a id="parsers-shape"></a>
## Generated parsers in the shape of simdjson and the lean parsers

88 records, now: 80 closed, 1 underway, 7 open. At the audit: 74 built, 8 partly built, 6 superseded. Origin: 28 a, 56 b, 4 c.

**Invariant.** A generated parser is the lean parser's shape, written from the grammar by the Bitlex compiler: tier 1 is a token loop that decides at the token what the lean lexer decides there (0177, 0415); tier 2 takes tier 1's records a batch at a time (0496); the writer writes the tape in one pass, as simdjson does (0434, 0530).

**Worktrees.** Parser-generators, Swift-Kotlin-Scala-support.

**Kept by.** Every grammar equals the interpreter and the reference (`every_grammar_compiles_here_and_equals_the_interpreters`, `every_grammar_parses_as_the_reference`). Nothing checks the shape: a mark or mask the lean parser does not have passes.

**Check to add.** Per grammar, the tier-1 mask words tier 2 reads and the marks decided after their token, pinned against the lean parser's. Kotlin today reads 31 mask words, where the lean lexer has five masks (0451).

**To apply it everywhere:**

- Kotlin, in order: 0177 (marks and checks at the token) → 0415 stages 3–4 → 0429 (the marks' context as loop state) → 0430 (dotted, acc, generic, decl_mod at the token) → 0451 (recv and dotted; tier 2's mask words) → 0496 (batches).
- 0414: a grammar's proofs read from and written to the content store for every grammar a run compiles.
- 0332: Kotlin's lost block comments (Q198), then Swift and Scala in Bitlex.

**Closes when.** Kotlin's tier 1 is the lean lexer's masks and nothing else, tier 2 takes batches, and Swift and Scala have their ladders.

**Your words:**

- "okay so implement the tier 2 lowering, and then implement the necessary improvements and fixes so that the tier 2 code is identical in structure and close in shape to simdjson" (0087)
- "The issue is the structure of the generated code vs simdjson, not the lack of micro-optimizatoins. Start by comparing the Rust code only with the Rust JSON parser and SIMDJSON, and get the structure right." (0338)
- "so go ahead and remove the back fills. … Then make the fixes and transformations to make the grammar generate the same general structure and peeks as the lean parser" (0407)

**Underway:**

| Record | Left | Worktree |
|---|---|---|
| [0414](decisions/0414-a-grammars-proofs-are-answers-by-content.md) A grammar's proofs are answers by content | Answers read from and written to the engine build's content store for every grammar compiled for a run (Q234), not only Kotlin's shipped seed | Two-pass-roots-and-cross-file, Swift-Kotlin-Scala-support |

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0177](decisions/0177-tier1-on-the-cpu-is-a-token-loop.md) Tier 1 on the CPU is a token loop, as the lean lexer is | marks and checks made at the token as its bits, so the CPU makes no mask but the loop's | a |
| [0332](decisions/0332-kotlin-swift-scala-in-bitlex.md) Kotlin, Swift and Scala in Bitlex: ladders against the lean parsers, … | Swift and Scala Bitlex versions not started; Kotlin's open gaps (lost block comments, Q198) | a |
| [0415](decisions/0415-kotlins-generated-parser-in-the-lean-parsers-shape.md) Kotlin's generated parser, in the lean parser's shape | stage 3-4 end state: tier 1 body's masks the lean lexer's five, context as token-loop state fully | a |
| [0429](decisions/0429-context-over-tokens-is-the-token-loops-state.md) Context over tokens is the token loop's state | Marks' context (acc, recv, generic, dotted) still byte masks, not token-loop state; class header context; region masks | c |
| [0430](decisions/0430-a-line-break-before-a-token-is-the-token-loops.md) A line break before a token is the token loop's, as the lean lexer's … | dotted, acc, generic, decl_mod marks: decide at the token where the lean parser does, not as marks every level ends on | b |
| [0451](decisions/0451-tier-1-written-in-the-lean-lexers-two-stages.md) Tier 1 written as Rust in the lean lexer's two stages | marks' masks (recv, dotted) at the token; escapes into token readers; tier 2 reading 31 tier-1 mask words | a |
| [0496](decisions/0496-tier-1-feeds-tier-2-a-batch-at-a-time.md) Tier 1 feeds tier 2 a batch at a time | Today's generated Rust (bitlex_rust) and the JIT lex the whole file before tier 2; no path feeds records a batch at a time | b |

**The assistant's own choices (c), and the goal each served:**

- [0322](decisions/0322-the-jit-lowers-a-match-as-rustc-does.md) The JIT lowers a `match` as rustc does (built). Served: close the JIT's gap to rustc's AOT code, from "please do that and record it".
- [0363](decisions/0363-the-blocks-loop-has-one-entry.md) The blocks' loop has one entry (built). Served: fix the shape of the generated code, from "Stop measuring cycles and just fucking fix the shape of the generated cde".
- [0407](decisions/0407-token-marks-tier-2-tests-alone.md) Token marks that tier 2 tests alone hold no bit of the code (built). Served: remove back fills; generate the lean Kotlin parser's structure and peeks, from "so go ahead and remove the back fills. … Then make the fixes and transformations to make the grammar generate the same general structure …".
- [0429](decisions/0429-context-over-tokens-is-the-token-loops-state.md) Context over tokens is the token loop's state (partly built). Served: Kotlin Bitlex parser on par with, shaped like, the lean parser, from "we're not moving off Kotlin until we have its bitlex performance roughly on par with the lean parser".

<a id="lean-parsers"></a>
## Fast parsers for every product language, on every host (closed 2026-11-23)

8 records, now: 8 closed, 0 underway, 0 open. At the audit: 8 built, 0 partly built. Origin: 5 a, 3 b, 0 c.

**Invariant.** Every product language has a lean parser on every host.

**Worktrees.** none.

**Kept by.** The byte tests' masks on every backend (`every_backend_gives_the_byte_tests_masks`, each lean crate).

**Closes when.** Closed 2026-11-23: every record built, and the byte tests keep every backend's masks.

**Your words:**

- "I would like you to work on getting the fast parsers (SIMDJSON, the lean Rerail parser) from the parser generators branch and hooked up into the product" (0134)
- "please ensure that the platform-specific parts are in a separate crate and that we have a parser router" (0148)
- "figure out what can be done (even as a second path when we detect a syntax error caused by a comment)" (0158)

<a id="tape-blocks"></a>
## One tape format and one block layout, read in place by CPU and GPU (never copied)

26 records, now: 20 closed, 1 underway, 5 open. At the audit: 19 built, 6 partly built, 1 superseded. Origin: 17 a, 8 b, 1 c.

**Invariant.** Data lives in one place, in one layout, and every reader reads it there (0353). A view is a read of the tape, not a relation. The reads are written once in CLIF (0310), proven within the tape's bytes (0463).

**Worktrees.** Native-and-GPU-strata, Benchmarks-and-parity, wolfeel (in part), Two-pass-roots-and-cross-file (in part), Bend-language-investigation (in part).

**Kept by.** `check_no_block_copies.py`; 5 marked copies left (steps 4, 6, 7 and 8 merged from Native-and-GPU-strata, 2026-11-23).

**To apply it everywhere:**

- Step 3 (1 site, `syntax.rs`, the eager per-node rows): goes with widening's 0174 step 3 and Q121.
- Step 4a (4 sites: the parsers' column vector, `tape.rs`; a queued tape not in a room, `core.rs`; a source made in memory, `placed.rs`, row 17): Native-and-GPU-strata is asked for rows 7 and 17.
- Then 0260 (facts between waves as base and delta blocks), 0458 (documents made when first read; phase runs read old sources without SQL, after 0104), 0336 (the planner's choice of decompression, Q65), 0156 §6 (text as a stream of pieces).

**Closes when.** No `copy (0353 step N)` marker is left, so the check refuses every copy with no exception.

**Beyond the invariant (your call):** 0156 §5's algorithm-tagged digests and second hash, and tradeoffs T1–T9; 0336's axis A question.

**Your words:**

- "It's particularly worrying that we pay such a hefty "upload" cost for a unified-memory device where "uploaded" data is in the same address space and resident in the same RAM!" (0289)
- "I want the kernels to operate over the tape directly, either compressed or decompressed, and format 15 or 16. There must be no intermediate 'decompress to an array of nodes' step." (0324)
- "The invariants are that the actual stored tape format is directly written and read, the same on CPU and GPU, with no decoding/translation to a different format." (0336)

**Underway:**

| Record | Left | Worktree |
|---|---|---|
| [0353](decisions/0353-blocks-are-never-copied-every-reader-reads-in-place.md) Blocks are never copied: every reader reads the data where it lives | 5 marked copies, step 3's one (core.rerail's per-node rows) and step 4a's four (a queued tape not in a room, the parsers' columns, a source made in memory) | Native-and-GPU-strata, Benchmarks-and-parity, wolfeel |

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0156](decisions/0156-tape-format-16-draft.md) Tape format 16, a byte tape read in place, in fixed blocks at rest … | §6 text as a stream of pieces (load without source); §5 algorithm-tagged digests and a second hash; tradeoffs T1-T9 still open | a |
| [0260](decisions/0260-blocks-one-slab-relative-offsets-cpu-and-gpu.md) Everything lives in blocks: linear, relative offsets, the same bytes … | facts handed between waves as base+delta blocks; scheduler and scoreboard over kernels; remaining 0353 copies | a |
| [0266](decisions/0266-a-block-is-a-big-thing-with-everything-nested-inside.md) A block is one of the engine's few big things; everything it holds … | step 4: setfix relations as RelationW/RelationV blocks, maintained runs as deltas, kernels reading relations in place | a |
| [0336](decisions/0336-the-tape-reaches-every-reader-as-stored.md) The tape reaches every reader as stored: blocks decompressed on the … | the planner's choice between per-block and whole decompression (Q65); whether the stored header is the view (axis A) | a |
| [0458](decisions/0458-the-cross-part-reads-the-base-relations-off-every-files-tape.md) The cross part reads the base relations off every file's tape | documents made when first read (batch still makes every document); phase-run old sources read without SQL | b |

**The assistant's own choices (c), and the goal each served:**

- [0161](decisions/0161-the-tape-format-from-11-to-15.md) The tape format from 11 to 15: named-only tapes, and no stored digest (built). Served: tapes store only named nodes (RER-131), from "Please help me to figure out if we need all of the treesitter node types, or if we can remove some from the tapes".

<a id="edit-cost"></a>
## An edit costs what it changes

22 records, now: 14 closed, 0 underway, 8 open. At the audit: 13 built, 8 partly built, 1 superseded. Origin: 7 a, 14 b, 1 c.

**Invariant.** An edit costs what it changes: a relation carries its delta (0117), maintenance reaches only what the delta reaches (0090, 0101), what an edit invalidates is keyed by definition (0092), and rules are keyed by module (0225).

**Worktrees.** none.

**Kept by.** Tests that an edit reads and reruns only its own path (`an_edit_reads_only_the_trees_on_its_path`, `a_relation_two_programs_share_is_kept_once_and_a_rule_edit_reaches_only_its_readers`). Nothing pins an edit's work to its delta.

**Check to add.** On a fixed edit, each stratum's rows read against the delta's rows, pinned. Today's surplus becomes the list that shrinks.

**To apply it everywhere:**

- 0090 §1 (kept relations read in place: the held rows, 0353 step 6's code) → §2 per-file base deltas → step 6 per-definition deltas, with 0092's symbol-keyed exports and definition keys.
- 0093: the check that a global fact feeds only findings, and the lints for forms 1 and 5.
- 0116 and 0115: derived frames written by document granule.
- 0117 step 5: a reached stratum's component kept with the session.
- 0225 §4, §5, §7 (§7 is 0246 stage 3), 0328 (the image as a pass's output), 0132 (bulk planning).

**Closes when.** The edit check shows no stratum reading past its delta, on ep2 and on the fixture.

**Your words:**

- "look at the sibling worktrees for prior art on the the cross-file work; it's becoming an issue for most of the worktrees" (0077)
- "Cross-file maintenance cost: survey the sibling worktrees, take a step back, and look at the overall direction we're heading in." (0090)
- "merge it now and identify the path forwards. It is not necessarily focusing on the same thing and I want to have two different solutions to compare" (0345)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0090](decisions/0090-cross-file-maintenance-costs-what-the-delta-reaches.md) Cross-file maintenance costs what the delta reaches | §1 kept relations read in place (held_rows copy remains); §2 per-file base deltas; step 6 per-definition delta; §4 cost estimate | a |
| [0092](decisions/0092-what-an-edit-invalidates.md) What an edit invalidates: definition keys, symbol-keyed exports, and … | Symbol-keyed exports and per-definition fact keys end to end; 0093's restriction on global facts | a |
| [0093](decisions/0093-global-facts-feed-findings.md) A fact over every document feeds findings, not rules; lints for the … | §2 the check that a global fact feeds only findings; §3 lints for forms 1 and 5; findings assembly leak | a |
| [0116](decisions/0116-what-an-edit-costs.md) What a one-line edit costs, and why | step 5: derived relations' frames written by document granule not confirmed | a |
| [0117](decisions/0117-a-relation-carries-its-delta.md) A relation carries its delta; maintenance is the core's, driven by … | step 5 amendment: a reached stratum's component (plans, spec, compiled joins, indexes) kept with the session across steps | a |
| [0132](decisions/0132-planning-searches-only-what-can-pay.md) The planner searches only where a search can pay, and could plan in … | greedy-order spike off by default; classifier/approximation planning and batched, scoreboarded bulk planning not built | b |
| [0225](decisions/0225-preparation-costs-the-change.md) An edit costs what it changes: rules keyed by module, and … | §5 split and congruence as rules over the image; §4 interim numberings removed; §7 prepared program kept per module (0246 draft) | b |
| [0328](decisions/0328-maintenance-is-how-the-evaluator-runs-a-rule-not-a-program-it-writes.md) A rule is stored once, small; maintenance is metadata and a mode of … | the program image as a pass's output format (0173) is not retired; record's own status still says nothing built | b |

**The assistant's own choices (c), and the goal each served:**

- [0345](decisions/0345-a-part-is-prepared-alone-against-its-interface.md) A part is prepared alone, against its interface, and the parts are linked (built). Served: a second ep2 edit-cost solution to compare with Typing-support's, from "merge it now and identify the path forwards. It is not necessarily focusing on the same thing and I want to have two different solutions …".

<a id="store"></a>
## The store: caches are facts, writes keep up, growth is bounded

29 records, now: 22 closed, 1 underway, 6 open. At the audit: 19 built, 7 partly built, 3 superseded. Origin: 21 a, 6 b, 2 c.

**Invariant.** Everything kept is facts, on the same fact path (0104). A cache is two rules (0103). Kept facts are blocks in mapped files (0401). The store stays bounded (0053).

**Worktrees.** Typing-support, final-shape-rr-align (in part).

**Kept by.** `facts_only.rs` pins 0 tables and 0 maps not yet facts (t33 steps 1-8, merged from Typing-support 2026-11-23); `check_no_new_caches.py` refuses new ones.

**To apply it everywhere:**

- t33 K4: a warm run's fixed cost, so the frames (the main run's and phase 1's) can go; then `inputs::still` and `git::still` with them.
- A session holds its store, its connections and its last corpus batch (0616): the batch stays a cache the store's facts validate (by documents' keys and digests), the user's call at the Typing-support merge.
- Then 0053 §7 (the atom epoch), 0115 (digest-only kept relations; inputs not kept twice), 0142 (artifact stamps), 0164 (orders kept in frames), then 0190 (re-sorting at idle).

**Closes when.** Both pins at 0 (done), K4's frames retired, and the atom epoch keeps the store bounded.

**Beyond the invariant (your call):** 0190's launchd agent for machines without a daemon.

**Your words:**

- "let's start by looking at what's happening in the store writer … tell me about all the crazy amounts of inefficiency inherent in how it works today and how we should fix it" (0240)
- "SQLite writeback as rows needs to happen, but as a bulk insert and as a kind of "archive" operation for the packfile; this is essential an application WAL" (0295)
- "We are moving away from sqlite as bulk storage and towards it having metadata pointing to memory mapped files with the data accessed through virtual tables." (0456)

**Underway:**

| Record | Left | Worktree |
|---|---|---|
| [0104](decisions/0104-everything-kept-is-facts.md) Everything kept is facts, on the same fact path | an unchanged run's fixed cost, so the kept frames can go, and the kept entries read in place (t33, "K4: what is left") | Typing-support |

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0053](decisions/0053-background-store-collection.md) The store collects in the background, keeping recent runs and a … | §7: the atom epoch (empty the arena and clear atom-keyed tables past 64 MB) is not built | a |
| [0103](decisions/0103-a-cache-is-two-rules.md) A cache is two rules | move the remaining pre-0103 caches (tape/index tables, run_record, stat_blob, derivation graph) to two rules | a |
| [0115](decisions/0115-what-a-run-keeps-for-the-next.md) What a run keeps for the next: rows where a maintained reader needs … | digest-only kept relations; derived frames split by document granule; inputs referenced from the memo, not kept twice | b |
| [0142](decisions/0142-artifacts-cached-by-what-they-depend-on.md) Run artifacts: rules first, then entries kept by what they depend on | per-entry artifact stamps and Unknown places, digests from phase 0, stamps stored by digest, entry inputs as import closure | a |
| [0164](decisions/0164-one-order-written-once-probed-by-density.md) One sorted order a key set, written once, kept in the frames, probed … | orders kept in frames (full/delta, compaction), density probing (dense merge, node-key interpolation), readers building nothing | a |
| [0190](decisions/0190-upkeep-jobs-run-when-the-machine-is-idle.md) Upkeep jobs run when the machine is idle, at the lowest priority, … | re-sorting stored rows (§5, O160, waits on 0164's frames); a launchd agent for machines without a daemon | a |

**The assistant's own choices (c), and the goal each served:**

- [0167](decisions/0167-computed-values-wait-for-narrowing-joins.md) A computed value waits for the joins that narrow its arguments (built). Served: stop long texts bloating the store past READ_WHOLE, from "tell me about how to address this: … One hogql run leaves 485 k strings, 77 MB, in the store".
- [0138](decisions/0138-a-frames-use-apart-and-checkpoints-beside-the-writer.md) A frame's use is kept apart from its rows, and the WAL is checkpointed beside … (built). Served: finish the edit-cost profile work without new caches, from "4. then complete the profile work".

<a id="cold-run"></a>
## Cold runs fast: every core used, the critical path short

32 records, now: 30 closed, 0 underway, 2 open. At the audit: 28 built, 2 partly built, 1 superseded, 1 analysis. Origin: 12 a, 17 b, 3 c.

**Invariant.** A cold run uses every core, and its critical path is short: strata are issued from a scoreboard (0113), and no thread ever blocks, because every wait is a scoreboard event (0114).

**Worktrees.** none.

**Kept by.** Measurement only.

**Check to add.** The blocking waits outside the scoreboard, counted and pinned; 0114 §4 takes the launches' and the loops' waits off the list.

**To apply it everywhere:**

- 0113: the interner overlay (§4), one plan per component, sessions and answered requests issued; measured at 10 to 300 files.
- 0114: answers loaded and evaluated on workers, written back in order (§3); launches and loop waits as events (§4, with 0540).

**Closes when.** No blocking wait outside the scoreboard, and the 10-to-300-file measurement recorded.

**Your words:**

- "53.3s user + 8.9s sys CPU is still vastly too high. Please help me understand what the next set of improvements could be here" (0060)
- "document loads need to be collected and submitted as a batch. And why do you need this done through the main thread? We should be able to load it in a worker" (0114)
- "if we continually improve the critical path, and discover the new critical paths once we've optimized each one, we will eventually end up very, very fast" (0282)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0113](decisions/0113-strata-issued-from-a-scoreboard-merged-then-parallel.md) Strata issued from a scoreboard: merged into components first, then … | interner overlay (§4), shared plan/open per component, sessions and answered requests issued, wall measured at 10-300 files | a |
| [0114](decisions/0114-suspend-never-block-waits-are-scoreboard-events.md) Suspend, never block: every wait is a scoreboard event, and answers … | §3's second half (answers loaded and evaluated on workers, written back in order), §4 launches and loop waits as events | b |

**The assistant's own choices (c), and the goal each served:**

- [0309](decisions/0309-the-guest-slices-statistics-measured-once-per-mix.md) The guest slice's statistics measured once per mix, as the run's are (built). Served: a shorter cold PostHog run, from "we still have a 35 second cold run which feels like it's too long and lots of memory usage we need to get a handle on".
- [0319](decisions/0319-a-chunks-counted-constants-are-estimates-and-branded-ones-are-counted.md) A chunk's counted constants are estimates, and the branded ones are counted (built). Served: a shorter cold PostHog run, from "we still have a 35 second cold run which feels like it's too long".
- [0432](decisions/0432-a-phase-export-is-not-placeless-its-readers-are-cross-file.md) A phase's export is not placeless: a rule that joins it is cross-file (built). Served: fix disastrous wall times on the phased run path, from "the wall times are a disaster. They are way worse than they should be. Figure out what is happening.".

<a id="planner"></a>
## The planner: never costs more than it saves, improves from feedback

17 records, now: 13 closed, 0 underway, 4 open. At the audit: 13 built, 3 partly built, 1 not started. Origin: 14 a, 1 b, 2 c.

**Invariant.** Planning never costs more than running (0122). Budgets never change results (0334). Plans come from the relations' declared shapes, so a run plans nothing (0361). Statistics and kept plans are blocks (0387).

**Worktrees.** Two-pass-roots-and-cross-file.

**Kept by.** The budget tests (a misplanned run restarts and gives the full budget's results). Nothing counts planning against running, or plans made at run time.

**Check to add.** A run's plans made at run time, counted and pinned at 0 (0361's done-criterion), and planning's work against the run's on the fixture packs.

**To apply it everywhere:**

- 0361: derived relations' estimates switched on again (Q290).
- 0387, all six steps: after 0104 step 6 and 0353.
- 0354: the learned correction as the default pricer.
- 0202: O163's IR forms.

**Closes when.** Both counts hold on the packs, and 0387's statistics are blocks.

**Your words:**

- "take the statistics, but I want a different shape as eventually we will need to run a classifier in Metal in a big batch to calculate these." (0123)
- "please survey the sibling worktrees to identify sibling work and the correct design, and then tell me how we will implement this" (0387)
- "those filters are an insurance policy against over-long joins and pathological cases we need them to be cheap enough to be on all the time" (0488)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0202](decisions/0202-kernels-missing-forms-are-recorded-and-ranked.md) The forms work ran without are recorded per machine, and ranked for … | O163: IR forms of clauses lower refuses and of the logic natives | a |
| [0354](decisions/0354-the-join-estimates-learned-correction-in-shadow.md) The join estimates' learned correction, in shadow | the model as default pricer; samples of orders not kept; step-0 estimates; sample volume; membership correction | a |
| [0361](decisions/0361-relations-declare-their-shapes-and-plans-come-from-them.md) Relations declare their shapes, and plans come from them | derived relations' estimates switched off (Q290); 'plans nothing at run time' done-criterion not shown met | a |
| [0387](decisions/0387-statistics-and-kept-plans-are-blocks-and-a-rounds-checks-run-on-the-device.md) Statistics and kept plans are blocks, and a round's plan checks run … | All six steps: statistics block, device batch into it, sketches in relation blocks, kept-plans block, check kernel, rounds by wave | a |

**The assistant's own choices (c), and the goal each served:**

- [0009](decisions/0009-planner-over-atoms.md) The planner lowers programs, statistics and inputs to the same integers (built). Served: Copy-in-place workplan, Phase 7 (planner over vtab inputs, O13), from "please go with phase 7".
- [0472](decisions/0472-a-lowered-programs-plans-are-its-stores.md) A lowered program's plans are its store's: they do not move across stores (built). Served: work through the workplan items, including Q185's decision, from "Then work through these: … 3. Then Q185's decision.".

<a id="metal-rules"></a>
## Rules on Metal: label walks, device waves, bounded launches, memory budgets

49 records, now: 43 closed, 2 underway, 4 open. At the audit: 42 built, 6 partly built, 1 superseded. Origin: 16 a, 22 b, 11 c.

**Invariant.** The Metal budget is the overall budget (0549). Every launch is counted by one ledger, and past `KERNEL_LAUNCH_LIMIT` a run fails. A device wait is on a running thread, never a poll (0540). A stratum's device work is one launch (0603).

**Worktrees.** Two-pass-roots-and-cross-file (in part), django-bugs.

**Kept by.** The launch ledger. Nothing checks for polling waits or a thread's resident copies.

**Check to add.** The polling waits in kernels and host loops, listed and pinned: today the preprocessor's MSL.

**To apply it everywhere:**

- 0549: each thread's resident copies go (0353 step 4).
- 0540: step 4 (a 16z block decoded by its SIMD group); the resident interpreter's Metal gate; the preprocessor's MSL without polling.
- 0603: 5d's other half (the lattice fold off the host's tables), 5e (device sorts), the closure launch test, rooms for non-recursive heads.
- 0095: aggregates and recursion on the device, text tests, the cross-file part.
- 0098: step 1's data-layer profile at scale.

**Closes when.** No polling wait left, and the Metal budget back down with the resident copies gone.

**Beyond the invariant (your call):** 0098 step 6c (library facts held on the Neural Engine); 0287 §2–§4, which the record defers until a workload shows gain.

**Your words:**

- "figure out why metal is still slow (it should be much faster than the CPU) using profiling / analysis and fix the low-hanging fruit. THis includes organizing the work shape and memory layout to be regular and …" (0015)
- "please take a look at the ast-index-spike design and journal, where this idea was comprehensively explored, and tell me what we should do with it now" (0365)
- "Take a step back and tell me what the optimal launch shape is and how we could do a fair amount of work per thread whilst using the full parllelism and avoiding the overhead of too many launches" (0474)

**Underway:**

| Record | Left | Worktree |
|---|---|---|
| [0540](decisions/0540-a-device-wait-is-on-a-running-thread-and-every-launch-has-a-budget.md) A device wait is on a thread that is running, and every persistent … | Step 4 (a 16z block decoded by its SIMD group together); the resident interpreter's Metal gate; the preprocessor's own MSL still waits by polling | django-bugs |
| [0603](decisions/0603-a-stratums-device-work-is-one-launch-a-work-queue-whose-rounds-are-continuations.md) A stratum's device work is one launch: a work queue whose phases and … | 5d's other half (the lattice fold still reads host tables), 5e (an index sorts claimed rows on the device), the closure launch test, non-recursive heads' rooms. | Two-pass-roots-and-cross-file, django-bugs |

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0095](decisions/0095-relational-work-on-metal.md) The logic engine's relational work on Metal: one plan, device waves, … | Aggregates and recursion on the device, device waves (§5), text tests on device, the cross-file part, planner pricing of tile | a |
| [0098](decisions/0098-acceleration-lanes.md) Accelerating relational work: forms first, then four lanes (CPU, … | step 1's data-layer profile at scale and step 6c (library facts held once on the Neural Engine) not shown built | a |
| [0287](decisions/0287-the-devices-functions-are-one-table-and-its-missing-shapes-are-measured.md) The device's functions are one table, and its missing shapes are … | §2–§4 drafts: node texts read from source on the device, made texts inside recursion, remaining functions (deferred until a workload shows gain). §1 built (2026-11-15); §2–§4 draft, not to be built … | b |
| [0549](decisions/0549-the-metal-budget-is-the-overall-budget.md) The Metal budget is the overall budget | Remove each thread's own resident copies and sorts of the same relations, so the Metal budget can come back down | a |

**The assistant's own choices (c), and the goal each served:**

- [0015](decisions/0015-micro-walks.md) Short label programs run as micro walks, one thread per (pass, start); … (built). Served: Make Metal fast: regular work shape and memory layout, from "figure out why metal is still slow (it should be much faster than the CPU) using profiling / analysis and fix the low-hanging fruit. THis …".
- [0016](decisions/0016-capacity-splits-and-s7.md) Work that outgrows a workspace is split along an axis it is independent on; … (built). Served: Make Metal fast, then complete S7 parity gate, from "figure out why metal is still slow … Then complete S7".
- [0017](decisions/0017-text-slots-and-cpu-roots.md) A label walk's text tests are bits computed once per (value, kind); a Metal … (built). Served: Make Metal fast, then complete S7 parity gate, from "figure out why metal is still slow … Then complete S7".
- [0018](decisions/0018-budgeted-ranges-and-count-walks.md) execution ranges packed to a byte budget; count programs run as walks (built). Served: Make Metal fast, then complete S7 parity gate, from "figure out why metal is still slow … Then complete S7".
- [0019](decisions/0019-four-cursors-and-shared-roots.md) walks carry four cursors; passes of one domain share their roots (built). Served: Make Metal fast, then complete S7 parity gate, from "figure out why metal is still slow … Then complete S7".
- [0020](decisions/0020-cpu-ranges-by-counts.md) CPU execution ranges packed by u32 record counts; program groups from one … (built). Served: Make Metal fast, then complete S7 parity gate, from "figure out why metal is still slow … Then complete S7".
- [0021](decisions/0021-walk-stack-tiers.md) walk stacks by static depth; deep walks stride over a pool with device-memory … (built). Served: Make Metal fast, then complete S7 parity gate, from "figure out why metal is still slow … Then complete S7".
- [0022](decisions/0022-footprint-within-budget.md) the engine's footprint within 500 MB: return freed pages, and size GPU buffers … (built). Served: Make Metal fast, then complete S7 parity gate, from "figure out why metal is still slow … Then complete S7".
- [0023](decisions/0023-metal-speed-by-profile.md) Metal's speed by profile: GPU occupancy, and the host work around the GPU (built). Served: Make Metal fast, then complete S7 parity gate, from "figure out why metal is still slow (it should be much faster than the CPU) using profiling / analysis and fix the low-hanging fruit.".
- [0034](decisions/0034-no-thread-memory.md) The device keeps no per-thread memory; walks run in pools with device stacks (built). Served: Finish leftover work and the O39 memory peak, from "complete the "still to do" work".
- [0359](decisions/0359-a-short-walks-group-is-a-simd-groups.md) A short walk's group of 64 nodes is a SIMD group's; the device's parent stays … (built). Served: fast device label walks without stored parents, from "please spike on this and validate the savings".

<a id="any-gpu"></a>
## Any GPU, and the CPU's SIMD units: kernels written once in UKL

2 records, now: 0 closed, 2 underway, 0 open. At the audit: 0 built, 2 partly built. Origin: 2 a, 0 b, 0 c.

**Invariant.** Kernels are written once in UKL, and every device reads the same blocks (0440). UKL on the CPU is vector code between sync points (0567).

**Worktrees.** CUDA-support, uglykl-CPU-SIMD-parallelization.

**Kept by.** `every_library_header_stands_alone_on_every_backend`, and each backend's conformance tests.

**To apply it everywhere:**

- 0440 step 10: one block directory, coherent across devices (after 0353).
- 0567 step 3: its own register allocator, per-call saves, Cranelift from SSA, the Rust SIMD back end, the 1000x gate.

**Closes when.** Step 10 and step 3 done.

**Beyond the invariant (your call):** 0440 steps 11–12 (partitioned memory on a discrete GPU, multi-GPU), the ROCm/HIP and WebGPU backends; 0567 step 4 (wider types).

**Your words:**

- "use it to bring support for CUDA (and other GPUs via ROCm, OpenCL and WebGPU/WebGL/whatever but starting with CUDA) without rewriting all of the kernels over and over again" (0440)
- "I want to run the same kernels on the GPU and the CPU … Currently they lower to coroutine C++ which is correct but horribly slow" (0567)

**Underway:**

| Record | Left | Worktree |
|---|---|---|
| [0440](decisions/0440-gpus-beyond-metal-through-ukl.md) GPUs beyond Metal: kernels written once in UKL, a device layer under … | step 10 block directory/coherency across domains, 11 partitioned memory on discrete GPU, 12 multi-GPU/host; ROCm/HIP and WebGPU backends | CUDA-support |
| [0567](decisions/0567-ukl-on-the-cpu-is-vector-code-between-sync-points.md) UKL on the CPU is vector code between sync points, made by … | Step 3: own register allocator, per-call saves, Cranelift from SSA, the Rust SIMD back end, and the 1000x gate | uglykl-CPU-SIMD-parallelization |

<a id="preprocessor"></a>
## A C/C++ preprocessor on the CPU and every GPU

17 records, now: 15 closed, 0 underway, 2 open. At the audit: 12 built, 2 partly built, 2 superseded, 1 analysis. Origin: 10 a, 7 b, 0 c.

**Invariant.** One parse a file into the preprocessor's tape, and an evaluator over tapes (0436), whose device passes read in place (0468), on the CPU and every GPU.

**Worktrees.** Go-C-and-C-support.

**Kept by.** 0353's check and the launch ledger, as everywhere.

**To apply it everywhere:**

- Header units: open in both 0327 and 0348, one piece of work.
- 0327: `c_pp_tape` and `c_pp_text` as stored objects; system headers as roots.
- 0348: the words on Metal, past 0408's gate.

**Closes when.** Header units, the stored objects and the Metal words done.

**Your words:**

- "a C preprocessor, in two modes: a) recording the directives as facts and b) applying, streaming mode. On both CPU and Metal" (0327)
- "I want you to work on parallel, recursive macro expansion. Both the GPU-based setup phase, and the GPU-based execution phase." (0448)
- "that way we are not continually expanding gigantic blocks of text but are referring back to work already done" (0520)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0327](decisions/0327-c-preprocessor-facts-and-streaming.md) A C and C++ preprocessor: directives recorded as facts, and applied … | c_pp_tape/c_pp_text as stored objects; header units; system headers as roots; compiled mode (b) moved to 0436/0448/0482 | a |
| [0348](decisions/0348-compiled-preprocessing-as-tier-2-words.md) Compiled preprocessing (mode (b)) is tier 2 words over a re-entrant … | the words on Metal (0408 gate missed); header units; 0436 moved the evaluator onto preprocessor tapes | b |

<a id="rules-parity"></a>
## Main's rules run directly, as Datalog (.rr), with main's findings and W's speed

43 records, now: 36 closed, 0 underway, 7 open. At the audit: 36 built, 7 partly built. Origin: 21 a, 20 b, 2 c.

**Invariant.** Main's rules run as `.rr`, with main's findings (the same finding keys) at W's speed. Text is matched a chunk at a time, never per node per byte (0041). A run starts from seeded facts (0044). A relation no rule fills makes no stratum (0563).

**Worktrees.** Benchmarks-and-parity (in part), wolfeel, Bend-language-investigation.

**Kept by.** The packs' parity tests and the data layer's 142 checks.

**To apply it everywhere:**

- Five of the seven open records are the device form of a CPU form already built, and go with Metal's waves: 0041 (chunked automata on the device), 0045 step 4 (device containers), 0511 (the merge-path form), 0561 (a keyed closure on the device), 0570 (its Metal form).
- 0044: seeds through the store, path facts as rules.
- 0563 A3: guest inputs counted empty once admission has closed.

**Closes when.** Each form runs on every executor (`every_operation_runs_on_every_executor`), and 0044 and 0563 are done.

**Your words:**

- "please take a look at the Final rerail shape branch and adapt to that, and then make any changes needed to the artifacts here" (0046)
- "please address these: … 31 JavaScript, TypeScript, React, Vue, Rust and SQL rules use binds, and 28 of them fail the reader" (0194)
- "How can we recognize and transform it into a fast, specialized algoritm? … The issue is that we're not willing to specialize it." (0511)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0041](decisions/0041-chunked-text-matching.md) Text matching is chunk-at-a-time and per file, never per node per byte | Device chunked automata (per-chunk state maps composed by prefix scan) replacing the per-byte device NFA (O66) | a |
| [0044](decisions/0044-runs-start-from-seeded-facts.md) A run starts from seeded facts; everything else is derived | config_file seeds; corpus runner seeding file facts through the store; path facts as .rr rules; local ids with doc_tape; per-stratum derivations by tape hash | a |
| [0045](decisions/0045-sets-in-chunked-containers.md) Node sets are chunked containers: arrays, bitmaps and runs | Step 4: device containers for Metal set steps and label passes (threadgroup per present chunk) | a |
| [0511](decisions/0511-an-aggregate-bounded-by-its-probe-is-an-as-of-search.md) An aggregate bounded by its probe is an as-of search, not a helper … | §3's binary search through an order on key+value columns; the device's merge-path form (it only refuses the stratum, untested) | b |
| [0561](decisions/0561-a-climbs-readers-are-answered-in-the-climb-each-row-test-asked-once.md) A climb's readers are answered in the climb, each of a row's tests … | Metal: the device refuses a keyed closure. walk's rows are still pushed during the climb | b |
| [0563](decisions/0563-a-relation-no-rule-can-fill-makes-no-stratum-and-a-delta-reaches-by-kind.md) A relation no rule can fill makes no stratum, and an edit's delta … | A3: guest inputs counted empty once admission has closed, for corpora whose file-local part answers requests | a |
| [0570](decisions/0570-a-walk-keyed-by-its-row-runs-as-a-closure-in-one-wave.md) A walk keyed by its row runs as a closure in one wave: the climb W's … | Metal form (t50's segmented scan by key); a per-scope-and-name probe as cheap as W's | c |

**The assistant's own choices (c), and the goal each served:**

- [0010](decisions/0010-waves.md) Waves run on one owner connection; SQL selects each wave's frontier; derived … (built). Served: Complete Phase 7 of the copy-in-place workplan, from "complete the phase".
- [0570](decisions/0570-a-walk-keyed-by-its-row-runs-as-a-closure-in-one-wave.md) A walk keyed by its row runs as a closure in one wave: the climb W's lookup … (partly built). Served: make the rules' scope lookup cost what W's does, from "please confirm/verify this: … Then tell me what we can do about it, including using a sorted hash".

<a id="embedded"></a>
## Embedded languages: strings parsed as their language, no guest documents

9 records, now: 8 closed, 0 underway, 1 open. At the audit: 8 built, 1 partly built. Origin: 5 a, 4 b, 0 c.

**Invariant.** There are no guest documents: a string the rules parse is a tape value, and a program the grammar holds is the file's (0431).

**Worktrees.** cost-classifier (in part).

**Kept by.** `a_policy_reads_in_a_pack_only_the_guests_it_asks_for_alone`, `a_session_drops_the_guest_rules_where_no_document_embeds`.

**To apply it everywhere:**

- 0431: `@parse` of computed strings checked against the packs' use; `embed_value`'s placement; the queued tapes' programs record; `publication.rs`'s program count.

**Closes when.** 0431's four items done. 0597, which removes hiding's copies, is counted under widening.

**Your words:**

- "we will need a way to get a normalized string out of a language, so that (for example) SQL in a string with escaped characters can access a non-escaped version that can be actually parsed" (0058)
- "if the rules say "here is an embedded string, here is its sha and here is its tape", then why can't that be kept in the facts" (0431)
- "why do we have ANNOTATION_KEYS? SHouldn't this be a more generic mechanism, not Python-only? Rules need to know line numbers and text in the host so that they can report properly" (0512)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0431](decisions/0431-guests-are-tape-values-the-rules-make-not-documents.md) No guest documents: strings the rules parse are tape values; … | @parse of computed strings checked against packs' use; embed_value placement; queued-tape programs record; publication.rs program count | a |

<a id="typing"></a>
## Typing and libraries: cross-file facts as ordinary facts, libraries held once

7 records, now: 3 closed, 0 underway, 4 open. At the audit: 1 built, 4 partly built, 2 superseded. Origin: 4 a, 1 b, 2 c.

**Invariant.** Typing's cross-file facts are ordinary facts: one fixpoint, maintained by deltas (0065, 0083). A library is a module, held once per run (0089).

**Worktrees.** none.

**Kept by.** The typing oracle tests.

**To apply it everywhere:**

- 0089 stage 3: the per-unit memo; a file's library rows kept under the library.
- 0083: the run path's step 3 (imports read P1 and P2); the stored fixpoint maintained by deltas, which needs 0090 and 0092's per-definition deltas.
- 0065 §3: instantiation as facts about signatures (`inst_ret`).
- 0047 §2: production calls switched to the typing programs.

**Closes when.** The fixpoint maintained by deltas, and the library rows held once.

**Beyond the invariant (your call):** 0083: Cranelift and Metal for searches.

**Your words:**

- "I would like you to look at how we can improve typing support and accuracy across our main languages. … Please help me to close them." (0047)
- "Currently, we are addressing performance issues that have arisen from the typing work, libraries and cross-file relationships. I want to take a step back and address these in a more holistic way." (0083)
- "common facts across the rules (generated by the libraries) are actually common" (0089)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0047](decisions/0047-typing-rounds-call-graph-type-relations.md) Typing: k = 4 with a pending round; `calls:` moves to the programs … | §2: production calls: switching to the typing programs per language not confirmed in code | b |
| [0065](decisions/0065-typing-is-one-fixpoint-over-stored-derivations.md) Typing's cross-file facts are one fixpoint, recorded by the store, … | §3's instantiation as facts about signatures (inst_ret, widening) not found; §2's stored cross-file derivation replaced by 0090/0092's maintenance | a |
| [0083](decisions/0083-phased-fact-spaces-and-memoised-search.md) Typing and libraries run as phases of one engine in separate fact … | Run-path step 3 (imports read P1/P2); §4 searches (recast as demand rules by 0144); maintenance of stored fixpoint by deltas; Cranelift/Metal for searches | a |
| [0089](decisions/0089-libraries-are-modules-held-once.md) A library is a module, held once per run; its facts are one relation … | Stage 3: the per-unit memo (§6), a file's library rows kept under the library; one library per bound (.guests off) | a |

**The assistant's own choices (c), and the goal each served:**

- [0035](decisions/0035-bounded-cross-file-rounds.md) Cross-file rounds bound file crossings, not iterations (superseded). Served: Complete list step 29 (typing strata), from "then complete step 29".
- [0144](decisions/0144-a-search-is-demand-rules.md) A search is demand rules (built). Served: finish t28: every cache and search through ordinary facts and rules, from "merge the integration branch then complete the work please".

<a id="programs"></a>
## Programs as flat images and versions, passes over them, never text

11 records, now: 9 closed, 0 underway, 2 open. At the audit: 7 built, 2 partly built, 2 superseded. Origin: 6 a, 3 b, 2 c.

**Invariant.** A program is a flat image, written through a writer and read as versions, never as text (0173, 0276). Rules load in one pass (0246). Passes run under a pass manager over maintained analyses (0281).

**Worktrees.** final-shape-rr-align.

**Kept by.** The image round trips (`a_program_reads_back_from_its_image`, `a_stored_program_is_read_from_its_image`). Nothing refuses a pass that goes through text.

**Check to add.** The program paths that print or parse text outside the CLI and tests, listed and pinned.

**To apply it everywhere:**

- 0246 stage 3: linking from module images in place of `combine_read`; lowering writes clauses into the builder; analyses as rules per module.
- 0281 §3: rule passes split across the pool by morsel.

**Closes when.** `combine_read` gone and the text check at 0.

**Your words:**

- "design the actual final shape of the loading with (hopefully) one pass" (0246)
- "tell me what is the archtiecturally-correct version of the replay rebuild" (0269)
- "I want to complete the workplan, we are getting stuck on micro-optimizing every step" (0377)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0246](decisions/0246-rules-load-in-one-pass-as-module-images.md) Rules load in one pass: each module read once into its image, a pack … | stage 3 linking from module images replacing combine_read; lowering writing clauses into the builder; analyses as rules per module | a |
| [0281](decisions/0281-program-passes-run-under-a-pass-manager.md) Program passes run under a pass manager: rule rewriters and program … | §3's parallel-by-morsel rule passes (stateless RulePass split across the pool); analyses as rules for program writers' incrementality | a |

**The assistant's own choices (c), and the goal each served:**

- [0215](decisions/0215-a-maintained-programs-parts-beside-its-version.md) A maintained program's parts are a typed form beside its version (superseded). Served: planner reads every program from a version (t36 item 7), from "okay, go ahead with item 7: plan::run takes a version.".
- [0241](decisions/0241-a-maintained-form-is-a-derivation.md) A maintained stratum's form is a derivation of the run's version (superseded). Served: planner reads every program from a version (t36 item 7), from "merge with the rr-align branch, and then complete item 7".

<a id="widening"></a>
## Work bounded by its readers: nothing widened past what is read, no widened output kept

8 records, now: 6 closed, 1 underway, 1 open. At the audit: 6 built, 2 partly built. Origin: 3 a, 5 b, 0 c.

**Invariant.** Nothing is computed past what its readers read, and nothing widened is kept. Each relation, copy and attribute exists for a reader's demand (0291, 0461, 0099).

**Worktrees.** cost-classifier.

**Kept by.** The width cap per relation (0462) and the product refusal (0461) stop a runaway. Nothing counts the copies or the attributes made for every node.

**Check to add.** On a fixed pack, the program's copy relations (hiding's, the narrow ones, the widened requests `rr_wide_*`) and the attributes made for every node, counted and pinned, down to 0.

**To apply it everywhere:**

- 0597 step 4: remove the annotation range (`HIDDEN_KEYS`), `FoldOwn`, the narrow copies (`rr_narrow_`), hiding's annotation copies (`rr_in_annotation`) and `one_document`; then Q420 and Q426, which turn on them; then steps 5 and 6. Steps 4 to 7 are in the hub (cost-classifier); the references it still has are in the counts above.
- 0174 step 3: long text on request (`rr_long_text_node`, `text_kind_demand`), with Q121. This removes 0353's step 3 copy.
- Q454 (requests by reader), 0458 (documents made when first read), 0115 (digest-only kept relations), 0099 (the factorized store).

**Closes when.** The count at 0, and 0597's measurement recorded.

**Your words** (2026-11-09, which became 0099): "Some of the relations should maybe not be materialized. Something which creates a massive expansion in an intermediate join before narrowing down should maybe be recomputed in the intermediate join and not serialized." Then, on 0575's cost: "we certainly can't afford to lose the performance", and on 0462: "limit the widest a rule can get".

**What widens, and what of it is kept.** Several passes widen a program or a relation past what its readers read, and their output is then computed in full and kept:

- Hiding (0058) writes a copy of each library rule a guest can reach: 616 relations and 1,311 rules, 16% of the program on corpus0 (0597).
- The fold (`FoldOwn`, 0165 §3) widens the originals' bounds to take the copies' rows, so #923's narrow copies write each rule a narrow reader reaches twice, and a twin of a reading rule makes each derivation twice (`pt_vopen`, 0575). The copies are prepared, kept with the program and computed as relations of their own; 0575 computes a narrow copy as its original less the annotations' change, and 0597 step 4 removes the annotation range, the fold and the copies.
- A request widened as a copy of its reader's body (0174, 0291); one that cannot split by reader keeps the attribute for every node (Q454, `on_request`'s "no, every node").
- Text made and interned for every node one unnarrowed read reached (0110, 0213); long text still eager (0174 step 3, Q121).
- Every document of the batch made, read or not (0458).

**Built before the audit, so not counted above:** 0099 (what a relation keeps: recompute in the reader, restrict by demand, or store; its factorized store designed, not built), 0145 (the cone is magic sets), 0167 (computed values wait for narrowing joins), 0213 (intern only what is stored), 0291 (a request asks only for what its readers read), 0575 (a narrow copy is its original less the annotations' change).

**Open elsewhere:** Q121 (`text` on request, 0353 step 3), Q420 (a partial hit's cone that sees annotations only through `FoldOwn`), Q426 (a wildcard node not bounded by hiding), Q454 (requests by reader where the shared one closes a loading cycle), 0458 (documents made when first read, tape-blocks), 0115 (digest-only kept relations, inputs not kept twice, store), 0099 (the factorized store).

**Underway:**

| Record | Left | Worktree |
|---|---|---|
| [0597](decisions/0597-a-guest-context-is-a-pass-of-its-own-rules-reading-its-host-explicitly.md) A guest context is a pass of its own rules, reading its host's … | Step 4: remove hiding's annotation range, FoldOwn, narrow copies and one_document (still in the code) | cost-classifier |

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0174](decisions/0174-per-node-source-attributes-are-answered-on-request.md) A node's source attributes are answered on request, by rules | step 3: long text on request, removing the twins, rr_long_text_node and text_kind_demand; step 4 measure inline text | b |

<a id="hygiene"></a>
## A re-entrant engine with bounded memory and no global state; one shape across the worktrees

6 records, now: 5 closed, 0 underway, 1 open. At the audit: 5 built, 1 partly built. Origin: 3 a, 2 b, 1 c.

**Invariant.** The engine is re-entrant and holds no global state (0442). A run's state is the run's (0441). No thread-local is unlisted (0192). Parallel work runs on the machine's pool (0442 step 5). The worktrees share one shape (0094).

**Worktrees.** Final-shape-quality, oscar (this tree).

**Kept by.** `check_no_global_state.py` (4 `shared` statics left), `check_thread_state.py`, `check_machine_pool.py`, `check_no_relation_maps.py` (0478: none left).

**To apply it everywhere:**

- The 4 `shared` statics onto the shared object (`profile::Shared`): `plan/corpus/owner.rs::SLOTS`, `regexes::TABLE`, `git_access/pool.rs::ROOTS`, `model.rs::SHARED_MODELS`.
- 0094 §5 (searches), §6 (phase 0's stored answers and read record), and the incremental layout-invariance gate.

**Closes when.** No `shared` entry left in `global-state.txt`, and 0094's gate in the tests.

**Your words:**

- "bring together all the work into a single shape and decision for the multi-phase dependency and typing work" (0094)
- "I want to understand fully what guarantees we can and can't rely on before we redesign the filesystem part again." (0400)
- "We should not be running git commands at all in the code that we're bringing, we should use gix to get whatever we need from the repo directly" (0497)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0094](decisions/0094-one-shape-phases-keys-libraries-scope.md) One shape for phases, keys, libraries and scope | §5 searches; §6 phase 0's stored answers and read record; incremental layout-invariance gate | a |

**The assistant's own choices (c), and the goal each served:**

- [0433](decisions/0433-a-listing-names-a-directorys-entries-not-the-directory.md) A listing names a directory's entries, not the directory; a racy answer is … (built). Served: leave no red tests or noted defects unfixed, from "DO NOT LEAVE TURDS SITTING AROUND AND JUST PASSIVELY NOTE THEM IN PASSING".
