# Engine decision records (final shape)

This continues the series in Pulling-it-all-together's `docs/rule-engine-direction/decisions/`
(0001–0007), with the same template and protocol: every record cites the ast-index-spike indexes
(`prior-work/`) and the lessons ledger there, and says whether it agrees or departs.

| # | Decision | Status |
|---|---|---|
| 0008 | Base facts are read through per-wave vtab bindings over core facts; integers only | accepted |
| 0009 | The planner lowers programs, statistics and inputs to the same integers | accepted |
| 0010 | Waves run on one owner connection; SQL selects each wave's frontier; derived relations are fact families | accepted |
| 0011 | Composite matchers lower to node-label passes plus positive walking programs | accepted |
| 0012 | Compilation runs in the background, on demand; the interpreter runs until it is ready | accepted |
| 0013 | Findings are one stratified SQL layer over the match facts, from facts of the tape | accepted |
| 0014 | The slice's entry points keep the wave's rows per scope; a warm run evaluates changed documents only | accepted |
| 0015 | Short label programs run as micro walks, one thread per (pass, start); workspaces are untouched by the host | accepted |
| 0016 | Work that outgrows a workspace is split along an axis it is independent on; S7's gate lists known differences by name | accepted |
| 0017 | A label walk's text tests are bits computed once per (value, kind); a Metal wave runs its roots on the CPU | accepted |
| 0018 | Execution ranges packed to a 64 MB workspace budget per file size; count programs run as micro walks | accepted |
| 0019 | Micro walks carry four cursors; label passes of one domain share their roots and walk items | accepted |
| 0020 | CPU execution ranges packed by u32 record counts (demand-zero workspaces); program groups from one batch's counts; text rows shared across a labelling's runs | accepted |
| 0021 | Micro walk stacks by static depth: 8 frames in thread memory, deeper walks over a 4096-thread pool with device stacks | accepted |
| 0022 | The engine's footprint within 500 MB: freed pages returned at stage ends, SQLite on jemalloc, GPU buffers sized by the run | accepted |
| 0023 | Metal's speed by profile: text rows in one pass, NFA early exit, a 2048×64 loop pool with a deep retry, faster host sweeps, unsorted candidate arms | accepted |
| 0024 | A wave's execution shape: one node space, sets as bitmaps evaluated natively from SQL's plan, labels and roots as device micro walks, integer match records, a RAM working set written back | accepted |
| 0025 | The tape format becomes v9np: random access for anchor indexing (O8's measurements on Rerail's grammars) | accepted |
| 0026 | Nodes live in tapes: no per-node structure beside a tape; how each node attribute is read from the tape or a bare index; enforced by `check-node-structures` | accepted |
| 0027 | Device-side enqueue is a work queue inside a persistent dispatch: workers push the items their work creates and idle workers take them, with no unbounded wait; first used by file-local lattice FIX | accepted |
| 0028 | G's grid executor is superseded by 0024: not ported to the work queue; its digit-ordered terminals, exact state dedupe, bulk closure and exact pages kept for a small general executor if the census's 71 of ~15,600 general programs grow | accepted |
| 0029 | The `py.*` fact families: five integer relations of calls, contexts and arguments, from core facts at the `call` postings, made on a wave's first read, one derivation per document over its core name; demand reads them before the batch | accepted |
| 0030 | Semantic predicates as labels from fact producers: W's predicate with its single-target sub-rules as slot captures makes (node, targets) rows before the passes; the slots are labels; keyed by the query's content; CPU and Metal | accepted |
| 0031 | The findings layer's statement is one typed query: `single` composes the steps' checked `select::Query`s (a materialized CTE per lower relation, reads rebound, checked whole); batch inputs are typed `Source`s; the layer's tables come from its program's `.decl`s; S's `sql_shape` stays test-only (O19) | accepted |
| 0032 | Tapes resident in 64 MiB slabs (one no-copy buffer each), keyed by tape digest, admitted from frames straight into slabs, dropped 2 min after a full execution; format 11 aligns columns; the device reads v9np columns in place through a batch document table (`metal_tape.metal`) | accepted |
| 0033 | A label pass's roots are a rule over each document's codes, not a node list | accepted |
| 0034 | The device keeps no per-thread memory (the driver's scratch, sized for every thread in flight and kept): walks run in pools with device stacks, pattern equality walks preorder runs, pattern kernels hold 8 patterns each | accepted |
| 0035 | Cross-file rounds bound file crossings: a recursive atom whose node may be in another file reads the previous round (`@doc` split); digest cutoff; k = 2 | accepted |
| 0036 | Rules are Datalog (joins, negation, recursion through facts, name sets, string tests, types, options); tree axes are built-ins computed as tapes in the transducer, as `nearest` is; the NFA is optimized and an optimizer may lower tape-shaped rules to tapes | accepted |
| 0037 | Text tests are globs over typed text, run once per distinct string | accepted |
| 0038 | Rule bodies simplified (nearest for inside-not-blocked, identical relations merged, implied atoms dropped); plans reused across chunks and re-priced; search bounded; estimates at least one row; nearest ancestors by pointer jumping on the device | accepted |
| 0039 | Device scratch buffers come from pre-pinned blocks (one retained no-copy buffer each, pages written once, power-of-two sizes, 32 MiB kept) and one kept command queue; the profile splits device time into buffers made, queue, encode, submit, schedule, GPU and wake | accepted |
| 0040 | Nearest ancestors in one dispatch per chunk: a cell's state (target, final bit) is read and advanced in place by its thread until final; one command buffer, one wait, any depth | accepted |
| 0041 | Text matching is chunk-at-a-time and per file: host DFAs and literal prefilters with no work limit, device chunked automata (O66); never a byte-at-a-time NFA per node | accepted (host done) |
| 0042 | One text layer: every text test (regex, glob, affix, name set) is normalized to a canonical matcher and key, so a regex costs what its best rewrite costs; vocabulary text runs once per distinct string, other text by one file scan placed on nodes through the tape | proposed |
| 0043 | Every rule the engine runs is `.rr` (Soufflé Datalog plus `core.rr`): one reader, one printer, the converter writes `.rr`; `.rdl` stays only as T's copies until step 31 | accepted |
| 0044 | Block programs run a batch of files in one launch: file-aligned groups, a file table, persistent workers, a function per program in one kernel (or the interpreter over a program table), one re-launch fallback | accepted |
| 0045 | A batch's files are read in place in residency slabs: sources admitted once by hash, up to four slabs bound, a slab per file row | accepted |
| 0046 | The CPU tape writer: starts and ends a word at a time, a stack per row and level, nodes in preorder, packed by the tape's one packer (`from_written`) | accepted |
| 0047 | Compiled languages keep simdjson's shape, locked by tests: no alignment carries (outputs and fallback at their own delays), `within d` a leaf, byte classes and `prev`s fold, JSON budgets as ratchets | accepted |
| 0048 | Carried state is bits: one per carried op, `k` per looked-back value; Cranelift packs them into fields of a few words; guards count bits and carried parameters | accepted |
| 0049 | Two tiers, as simdjson's two stages: tier 1 (block program) only classes, strings, token starts and lasts, per-byte errors; tier 2 (the tape writer, per token) kinds, keywords, checks, nesting on a stack, roles; the primitive is simdjson's `escaped` | accepted |
| 0050 | Tier 2 on Metal, for JSON first: a dependency analysis; two passes over the bytes and three chained scans over 2 KB entries of four candidates; an 8-byte stack effect; pass B writes the tape; chunks for large files (doc 14) | accepted |
| 0051 | The tree section is a grammar (`node K [nested] = E`, choices with supertypes, fields); one DFA per level with open/leaf/close actions and calls; tier 2 is that state machine over one extracted stream, simdjson's stage 2; rows keep only tokens (doc 18) | accepted |
| 0052 | A token's syntax is tier 2's, as simdjson's `parse_number`: `check KIND = G` is a grammar over the token's bytes, compiled to a byte state machine held as data; JSON's number check leaves tier 1; one word token, keywords retype it | accepted |
| 0053 | Tier 2 is the transducer's own instruction words, with new ones (`TOKEN`, `SWITCH`, `WORD`, `CHECK`, node writes, `PUSH`/`RETURN`); lowered by its interpreter, `native_ir` and Cranelift; one loop, over the stream | accepted |
| 0054 | Tier 2 writes the tape: v9np's columns from the compiled words, codes known at compile time and numbered at first use, widths guessed and fixed by at most one second pass (doc 19 for its cost) | accepted |
