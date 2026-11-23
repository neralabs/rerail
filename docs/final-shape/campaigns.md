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

## The campaigns

| Campaign | Records | Open | a / b / c | Priority |
|---|---:|---:|---|---|
| [Generated parsers in the shape of simdjson and the lean parsers](#parsers-shape) | 88 | 8 | 28 / 56 / 4 | |
| [Fast parsers for every product language, on every host](#lean-parsers) | 8 | 0 | 5 / 3 / 0 | |
| [One tape format and one block layout, read in place by CPU and GPU (never copied)](#tape-blocks) | 26 | 6 | 17 / 8 / 1 | |
| [An edit costs what it changes](#edit-cost) | 22 | 8 | 7 / 14 / 1 | |
| [The store: caches are facts, writes keep up, growth is bounded](#store) | 29 | 7 | 21 / 6 / 2 | |
| [Cold runs fast: every core used, the critical path short](#cold-run) | 32 | 2 | 12 / 17 / 3 | |
| [The planner: never costs more than it saves, improves from feedback](#planner) | 17 | 4 | 14 / 1 / 2 | |
| [Rules on Metal: label walks, device waves, bounded launches, memory budgets](#metal-rules) | 49 | 6 | 16 / 22 / 11 | |
| [Any GPU, and the CPU's SIMD units: kernels written once in UKL](#any-gpu) | 2 | 2 | 2 / 0 / 0 | |
| [A C/C++ preprocessor on the CPU and every GPU](#preprocessor) | 17 | 2 | 10 / 7 / 0 | |
| [Main's rules run directly, as Datalog (.rr), with main's findings and W's speed](#rules-parity) | 43 | 7 | 21 / 20 / 2 | |
| [Embedded languages: strings parsed as their language, no guest documents](#embedded) | 9 | 1 | 5 / 4 / 0 | |
| [Typing and libraries: cross-file facts as ordinary facts, libraries held once](#typing) | 7 | 4 | 4 / 1 / 2 | |
| [Programs as flat images and versions, passes over them, never text](#programs) | 11 | 2 | 6 / 3 / 2 | |
| [Work bounded by its readers: nothing widened past what is read, no widened output kept](#widening) | 8 | 2 | 3 / 5 / 0 | |
| [A re-entrant engine with bounded memory and no global state; one shape across the worktrees](#hygiene) | 6 | 1 | 3 / 2 / 1 | |

<a id="parsers-shape"></a>
## Generated parsers in the shape of simdjson and the lean parsers

88 records: 74 built, 8 partly built, 6 superseded. Origin: 28 a, 56 b, 4 c.

**Your words:**

- "okay so implement the tier 2 lowering, and then implement the necessary improvements and fixes so that the tier 2 code is identical in structure and close in shape to simdjson" (0087)
- "The issue is the structure of the generated code vs simdjson, not the lack of micro-optimizatoins. Start by comparing the Rust code only with the Rust JSON parser and SIMDJSON, and get the structure right." (0338)
- "so go ahead and remove the back fills. … Then make the fixes and transformations to make the grammar generate the same general structure and peeks as the lean parser" (0407)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0177](decisions/0177-tier1-on-the-cpu-is-a-token-loop.md) Tier 1 on the CPU is a token loop, as the lean lexer is | marks and checks made at the token as its bits, so the CPU makes no mask but the loop's | a |
| [0332](decisions/0332-kotlin-swift-scala-in-bitlex.md) Kotlin, Swift and Scala in Bitlex: ladders against the lean parsers, … | Swift and Scala Bitlex versions not started; Kotlin's open gaps (lost block comments, Q198) | a |
| [0414](decisions/0414-a-grammars-proofs-are-answers-by-content.md) A grammar's proofs are answers by content | Answers read from and written to the engine build's content store for every grammar compiled for a run (Q234), not only Kotlin's shipped seed | a |
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
## Fast parsers for every product language, on every host

8 records: 8 built, 0 partly built. Origin: 5 a, 3 b, 0 c.

**Your words:**

- "I would like you to work on getting the fast parsers (SIMDJSON, the lean Rerail parser) from the parser generators branch and hooked up into the product" (0134)
- "please ensure that the platform-specific parts are in a separate crate and that we have a parser router" (0148)
- "figure out what can be done (even as a second path when we detect a syntax error caused by a comment)" (0158)

<a id="tape-blocks"></a>
## One tape format and one block layout, read in place by CPU and GPU (never copied)

26 records: 19 built, 6 partly built, 1 superseded. Origin: 17 a, 8 b, 1 c.

**Your words:**

- "It's particularly worrying that we pay such a hefty "upload" cost for a unified-memory device where "uploaded" data is in the same address space and resident in the same RAM!" (0289)
- "I want the kernels to operate over the tape directly, either compressed or decompressed, and format 15 or 16. There must be no intermediate 'decompress to an array of nodes' step." (0324)
- "The invariants are that the actual stored tape format is directly written and read, the same on CPU and GPU, with no decoding/translation to a different format." (0336)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0156](decisions/0156-tape-format-16-draft.md) Tape format 16, a byte tape read in place, in fixed blocks at rest … | §6 text as a stream of pieces (load without source); §5 algorithm-tagged digests and a second hash; tradeoffs T1-T9 still open | a |
| [0260](decisions/0260-blocks-one-slab-relative-offsets-cpu-and-gpu.md) Everything lives in blocks: linear, relative offsets, the same bytes … | facts handed between waves as base+delta blocks; scheduler and scoreboard over kernels; remaining 0353 copies | a |
| [0266](decisions/0266-a-block-is-a-big-thing-with-everything-nested-inside.md) A block is one of the engine's few big things; everything it holds … | step 4: setfix relations as RelationW/RelationV blocks, maintained runs as deltas, kernels reading relations in place | a |
| [0336](decisions/0336-the-tape-reaches-every-reader-as-stored.md) The tape reaches every reader as stored: blocks decompressed on the … | the planner's choice between per-block and whole decompression (Q65); whether the stored header is the view (axis A) | a |
| [0353](decisions/0353-blocks-are-never-copied-every-reader-reads-in-place.md) Blocks are never copied: every reader reads the data where it lives | the marked copies of steps 3, 4/4a, 6, 7 and 8 (30 sites) are still to be removed | a |
| [0458](decisions/0458-the-cross-part-reads-the-base-relations-off-every-files-tape.md) The cross part reads the base relations off every file's tape | documents made when first read (batch still makes every document); phase-run old sources read without SQL | b |

**The assistant's own choices (c), and the goal each served:**

- [0161](decisions/0161-the-tape-format-from-11-to-15.md) The tape format from 11 to 15: named-only tapes, and no stored digest (built). Served: tapes store only named nodes (RER-131), from "Please help me to figure out if we need all of the treesitter node types, or if we can remove some from the tapes".

<a id="edit-cost"></a>
## An edit costs what it changes

22 records: 13 built, 8 partly built, 1 superseded. Origin: 7 a, 14 b, 1 c.

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

29 records: 19 built, 7 partly built, 3 superseded. Origin: 21 a, 6 b, 2 c.

**Your words:**

- "let's start by looking at what's happening in the store writer … tell me about all the crazy amounts of inefficiency inherent in how it works today and how we should fix it" (0240)
- "SQLite writeback as rows needs to happen, but as a bulk insert and as a kind of "archive" operation for the packfile; this is essential an application WAL" (0295)
- "We are moving away from sqlite as bulk storage and towards it having metadata pointing to memory mapped files with the data accessed through virtual tables." (0456)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0053](decisions/0053-background-store-collection.md) The store collects in the background, keeping recent runs and a … | §7: the atom epoch (empty the arena and clear atom-keyed tables past 64 MB) is not built | a |
| [0103](decisions/0103-a-cache-is-two-rules.md) A cache is two rules | move the remaining pre-0103 caches (tape/index tables, run_record, stat_blob, derivation graph) to two rules | a |
| [0104](decisions/0104-everything-kept-is-facts.md) Everything kept is facts, on the same fact path | move the remaining 31 tables and 6 maps (steps 1-8) to facts; load a stored fact space across processes | a |
| [0115](decisions/0115-what-a-run-keeps-for-the-next.md) What a run keeps for the next: rows where a maintained reader needs … | digest-only kept relations; derived frames split by document granule; inputs referenced from the memo, not kept twice | b |
| [0142](decisions/0142-artifacts-cached-by-what-they-depend-on.md) Run artifacts: rules first, then entries kept by what they depend on | per-entry artifact stamps and Unknown places, digests from phase 0, stamps stored by digest, entry inputs as import closure | a |
| [0164](decisions/0164-one-order-written-once-probed-by-density.md) One sorted order a key set, written once, kept in the frames, probed … | orders kept in frames (full/delta, compaction), density probing (dense merge, node-key interpolation), readers building nothing | a |
| [0190](decisions/0190-upkeep-jobs-run-when-the-machine-is-idle.md) Upkeep jobs run when the machine is idle, at the lowest priority, … | re-sorting stored rows (§5, O160, waits on 0164's frames); a launchd agent for machines without a daemon | a |

**The assistant's own choices (c), and the goal each served:**

- [0167](decisions/0167-computed-values-wait-for-narrowing-joins.md) A computed value waits for the joins that narrow its arguments (built). Served: stop long texts bloating the store past READ_WHOLE, from "tell me about how to address this: … One hogql run leaves 485 k strings, 77 MB, in the store".
- [0138](decisions/0138-a-frames-use-apart-and-checkpoints-beside-the-writer.md) A frame's use is kept apart from its rows, and the WAL is checkpointed beside … (built). Served: finish the edit-cost profile work without new caches, from "4. then complete the profile work".

<a id="cold-run"></a>
## Cold runs fast: every core used, the critical path short

32 records: 28 built, 2 partly built, 1 superseded, 1 analysis. Origin: 12 a, 17 b, 3 c.

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

17 records: 13 built, 3 partly built, 1 not started. Origin: 14 a, 1 b, 2 c.

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

49 records: 42 built, 6 partly built, 1 superseded. Origin: 16 a, 22 b, 11 c.

**Your words:**

- "figure out why metal is still slow (it should be much faster than the CPU) using profiling / analysis and fix the low-hanging fruit. THis includes organizing the work shape and memory layout to be regular and …" (0015)
- "please take a look at the ast-index-spike design and journal, where this idea was comprehensively explored, and tell me what we should do with it now" (0365)
- "Take a step back and tell me what the optimal launch shape is and how we could do a fair amount of work per thread whilst using the full parllelism and avoiding the overhead of too many launches" (0474)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0095](decisions/0095-relational-work-on-metal.md) The logic engine's relational work on Metal: one plan, device waves, … | Aggregates and recursion on the device, device waves (§5), text tests on device, the cross-file part, planner pricing of tile | a |
| [0098](decisions/0098-acceleration-lanes.md) Accelerating relational work: forms first, then four lanes (CPU, … | step 1's data-layer profile at scale and step 6c (library facts held once on the Neural Engine) not shown built | a |
| [0287](decisions/0287-the-devices-functions-are-one-table-and-its-missing-shapes-are-measured.md) The device's functions are one table, and its missing shapes are … | §2–§4 drafts: node texts read from source on the device, made texts inside recursion, remaining functions (deferred until a workload shows gain) | b |
| [0540](decisions/0540-a-device-wait-is-on-a-running-thread-and-every-launch-has-a-budget.md) A device wait is on a thread that is running, and every persistent … | Step 4 (a 16z block decoded by its SIMD group together); the resident interpreter's Metal gate; the preprocessor's own MSL still waits by polling. | a |
| [0549](decisions/0549-the-metal-budget-is-the-overall-budget.md) The Metal budget is the overall budget | Remove each thread's own resident copies and sorts of the same relations, so the Metal budget can come back down. | a |
| [0603](decisions/0603-a-stratums-device-work-is-one-launch-a-work-queue-whose-rounds-are-continuations.md) A stratum's device work is one launch: a work queue whose phases and … | 5d's other half (the lattice fold still reads host tables), 5e (an index sorts claimed rows on the device), the closure launch test, non-recursive heads' rooms. | a |

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

2 records: 0 built, 2 partly built. Origin: 2 a, 0 b, 0 c.

**Your words:**

- "use it to bring support for CUDA (and other GPUs via ROCm, OpenCL and WebGPU/WebGL/whatever but starting with CUDA) without rewriting all of the kernels over and over again" (0440)
- "I want to run the same kernels on the GPU and the CPU … Currently they lower to coroutine C++ which is correct but horribly slow" (0567)

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0440](decisions/0440-gpus-beyond-metal-through-ukl.md) GPUs beyond Metal: kernels written once in UKL, a device layer under … | step 10 block directory/coherency across domains, 11 partitioned memory on discrete GPU, 12 multi-GPU/host; ROCm/HIP and WebGPU backends | a |
| [0567](decisions/0567-ukl-on-the-cpu-is-vector-code-between-sync-points.md) UKL on the CPU is vector code between sync points, made by … | Step 3: own register allocator, per-call saves, Cranelift from SSA, the Rust SIMD back end, and the 1000x gate. Step 4: wider types. | a |

<a id="preprocessor"></a>
## A C/C++ preprocessor on the CPU and every GPU

17 records: 12 built, 2 partly built, 2 superseded, 1 analysis. Origin: 10 a, 7 b, 0 c.

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

43 records: 36 built, 7 partly built. Origin: 21 a, 20 b, 2 c.

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
| [0561](decisions/0561-a-climbs-readers-are-answered-in-the-climb-each-row-test-asked-once.md) A climb's readers are answered in the climb, each of a row's tests … | Metal: the device refuses a keyed closure. walk's rows are still pushed during the climb. Cheaper probes are the probe structure's job. | b |
| [0563](decisions/0563-a-relation-no-rule-can-fill-makes-no-stratum-and-a-delta-reaches-by-kind.md) A relation no rule can fill makes no stratum, and an edit's delta … | A3: guest inputs counted empty once admission has closed, for corpora whose file-local part answers requests. | a |
| [0570](decisions/0570-a-walk-keyed-by-its-row-runs-as-a-closure-in-one-wave.md) A walk keyed by its row runs as a closure in one wave: the climb W's … | Metal form (t50's segmented scan by key); a per-scope-and-name probe as cheap as W's. | c |

**The assistant's own choices (c), and the goal each served:**

- [0010](decisions/0010-waves.md) Waves run on one owner connection; SQL selects each wave's frontier; derived … (built). Served: Complete Phase 7 of the copy-in-place workplan, from "complete the phase".
- [0570](decisions/0570-a-walk-keyed-by-its-row-runs-as-a-closure-in-one-wave.md) A walk keyed by its row runs as a closure in one wave: the climb W's lookup … (partly built). Served: make the rules' scope lookup cost what W's does, from "please confirm/verify this: … Then tell me what we can do about it, including using a sorted hash".

<a id="embedded"></a>
## Embedded languages: strings parsed as their language, no guest documents

9 records: 8 built, 1 partly built. Origin: 5 a, 4 b, 0 c.

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

7 records: 1 built, 4 partly built, 2 superseded. Origin: 4 a, 1 b, 2 c.

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

11 records: 7 built, 2 partly built, 2 superseded. Origin: 6 a, 3 b, 2 c.

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

8 records: 6 built, 2 partly built. Origin: 3 a, 5 b, 0 c.

**Your words** (2026-11-09, which became 0099): "Some of the relations should maybe not be materialized. Something which creates a massive expansion in an intermediate join before narrowing down should maybe be recomputed in the intermediate join and not serialized." Then, on 0575's cost: "we certainly can't afford to lose the performance", and on 0462: "limit the widest a rule can get".

**What widens, and what of it is kept.** Several passes widen a program or a relation past what its readers read, and their output is then computed in full and kept:

- Hiding (0058) writes a copy of each library rule a guest can reach: 616 relations and 1,311 rules, 16% of the program on corpus0 (0597).
- The fold (`FoldOwn`, 0165 §3) widens the originals' bounds to take the copies' rows, so #923's narrow copies write each rule a narrow reader reaches twice, and a twin of a reading rule makes each derivation twice (`pt_vopen`, 0575). The copies are prepared, kept with the program and computed as relations of their own; 0575 computes a narrow copy as its original less the annotations' change, and 0597 step 4 removes the annotation range, the fold and the copies.
- A request widened as a copy of its reader's body (0174, 0291); one that cannot split by reader keeps the attribute for every node (Q454, `on_request`'s "no, every node").
- Text made and interned for every node one unnarrowed read reached (0110, 0213); long text still eager (0174 step 3, Q121).
- Every document of the batch made, read or not (0458).

**Built before the audit, so not counted above:** 0099 (what a relation keeps: recompute in the reader, restrict by demand, or store; its factorized store designed, not built), 0145 (the cone is magic sets), 0167 (computed values wait for narrowing joins), 0213 (intern only what is stored), 0291 (a request asks only for what its readers read), 0575 (a narrow copy is its original less the annotations' change).

**Open elsewhere:** Q121 (`text` on request, 0353 step 3), Q420 (a partial hit's cone that sees annotations only through `FoldOwn`), Q426 (a wildcard node not bounded by hiding), Q454 (requests by reader where the shared one closes a loading cycle), 0458 (documents made when first read, tape-blocks), 0115 (digest-only kept relations, inputs not kept twice, store), 0099 (the factorized store).

**Open:**

| Record | Left | Origin |
|---|---|---|
| [0174](decisions/0174-per-node-source-attributes-are-answered-on-request.md) A node's source attributes are answered on request, by rules | step 3: long text on request, removing the twins, rr_long_text_node and text_kind_demand; step 4 measure inline text | b |
| [0597](decisions/0597-a-guest-context-is-a-pass-of-its-own-rules-reading-its-host-explicitly.md) A guest context is a pass of its own rules, reading its host's … | Step 4: remove hiding's annotation range, FoldOwn, narrow copies and one_document (still in the code). Step 5: f-strings and Vue. Step 6: measure. | a |

<a id="hygiene"></a>
## A re-entrant engine with bounded memory and no global state; one shape across the worktrees

6 records: 5 built, 1 partly built. Origin: 3 a, 2 b, 1 c.

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
