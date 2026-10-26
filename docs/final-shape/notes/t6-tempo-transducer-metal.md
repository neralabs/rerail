# T6 notes: Tempo transducer IR, CPU/JIT/Metal execution, batch model, SQL-driven execution

Research track T6. Read-only extraction from Tempo docs at
`/Users/jeremy/orca/workspaces/nera-landing-2/ner-1562-scale-test-the-re-railer/tempo/docs/` (abbreviated `TD/` below)
and PIAT research at `/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/docs/rule-engine-direction/` (abbreviated `PIAT/`).
Citations are `file:line`. Number marks: **M** = measured (the doc reports a run), **D** = design target / limit / estimate,
**A** = asserted or claimed without a run shown in that doc. "not found" means I searched the named sources and the
point is absent.

One source-code check was done (read-only) because the docs disagree with each other on the opcode set:
`tempo/native/src/policy_dsl/transducer.rs:192-222` (opcode constants). Everything else is from docs.

---

## 1. TD/transducer-design.md (339 lines) — the structural IR

Status line: "redesigned language proposal. This replaces the previous VM design." (:3)

### Scope and non-goals
- A transducer reads a syntax tape and produces match records with captures; "not a general-purpose VM" (:7-8).
- No scalar registers, arithmetic, calls, stack ops, mutable memory, or host callbacks; only bounded cursor slots retain node identities (:10-11).
- Storage, SQL rule composition, scheduling, backend implementation are outside language semantics (:12).
- The language does not define SQL operators, rule-level negation, binding environments, rewrites, or a general expression evaluator; it does not claim arbitrary TS queries / ast-grep rules lower directly "without semantic adaptation" (:241-243).

### Input / output machine model
- Input: immutable tape, optional source bytes, ordered set of starting nodes (:17). Tape node exposes kind, incoming field, flags, byte span, parent, ordered children (:18). Node identity = tape identity + ordinal, NOT byte span (:19).
- Each starting node begins an independent path at instruction 0 (:21). A path is logically (instruction position, current node, ordered capture sequence, active repetition counters) — "semantics, not a prescribed runtime data structure" (:22-23).
- Test retains or rejects a path; navigation with several results creates one path per result; alternatives create independent paths with same node+captures; only `EMIT` produces output (:25-28).
- Output record = (starting node, output label, ordered captures); capture = (label, node identity, half-open UTF-8 byte span) (:30-31). Repeated capture labels append, never overwrite (:32). Rejected paths produce nothing (:33).

### Encoding (:35-60)
- Exactly 32 bits per instruction: `word = opcode | (mode << 8) | (operand << 16)`; opcode u8 bits 0-7, mode u8 bits 8-15, operand u16 bits 16-31 (:37-46).
- No extension words / prefixes / variable length; next instruction is always next word; positions count words; little-endian serialization (:48-51).
- 16-bit arithmetic for positions, table indices, labels, small comparisons; checked, overflow rejects (:53-54). Max 65,536 instructions and 65,536 entries per local table; oversize = explicit compile error (:55-56).
- Tape positions/source spans are native width via the tape iterator; >65,535-node files not truncated (:58-60).

### Tables (:62-81)
Four local tables: Strings (arbitrary-length bytes; names UTF-8), Predicates (pure text tests: equality, prefix, regex with string index + declared text semantics), Labels (capture/output names -> string entries), Long integers (u64 constants for numeric tests >16 bits) (:64-71). Strings can contain NUL; no inline string data in instructions (:73-74). Regex entries identify dialect+flags; unsupported dialect = error not failed match (:75). Only `SIZE` has a long mode (:77-80). "Tables contain data, not hidden instruction sequences or arbitrary executable expressions" (:81).

### Opcode table as documented (:89-111)
| op | name | mode | operand | effect |
|---|---|---|---|---|
| 00 | STOP | 0 | 0 | end path, no output |
| 01 | KIND | 0 | kind id | retain if current kind == operand |
| 02 | FIELD | 0 | field id | retain if incoming field == operand (field 0 = none) |
| 03 | FLAGS | required mask | forbidden mask | require all required, no forbidden |
| 04 | TEXT | 0 | predicate idx | source bytes satisfy predicate |
| 05 | SIZE | cmp (+bit7 long) | value / long idx | compare span byte length |
| 0a | SUPERTYPE | 0 | supertype kind id | tape records this hidden supertype for node |
| 10 | CHILD | node selection | 0 | continue once per selected direct child |
| 11 | NEXT | node selection | 0 | move to next selected sibling or reject |
| 12 | PARENT | 0 | 0 | move to parent or reject |
| 13 | DESCEND | node selection | 0 | once per selected proper descendant, preorder |
| 14 | FOLLOWING | node selection | 0 | once per later selected sibling |
| 15 | SAVE_CURSOR | 0 | slot | save current node identity |
| 16 | RESTORE_CURSOR | 0 | slot | restore saved node or reject |
| 20 | CAPTURE | 0 | label idx | append current node under label |
| 21 | REPEAT_BEGIN | 0 | repeat id | start counter |
| 22 | REPEAT_BUMP | 0 | repeat id | increment active counter |
| 23 | REPEAT_END | 0 | repeat id | close active counter |
| 30 | FORK | branch policy | target | continue at next, defer target branch |
| 31 | GOTO | 0 | target | jump, same node+captures |
| 40 | EMIT | 0 | label idx | emit one record, end path |

- Flags bits: 0=named, 1=extra, 2=missing, 3=error; others reserved; a flag can't be both required and forbidden (:113-115).
- Node selection mode 0=all, 1=named; `NEXT named` skips anonymous siblings rather than rejecting at first anonymous (:116-117).
- SIZE modes 0=eq, 1=at least, 2=at most; bit 7 = long-table reference (:119-120). Example `SIZE at-least,long 3` = `00038105` (:211-219). Span length never narrowed before compare (:126).
- FORK policy 0 retains each branch; policy 1 "replaces an equivalent deferred branch with its later greedy state" (:122). Compiler uses policy 1 ONLY for "a terminal root-child repetition whose body adds no captures" (:124).

**Contradiction / omission (opcode set).** The design doc's table omits four opcodes that exist in source and that
`transducer-native-jit-design.md:255-266` lists as test instructions: `ASSERT`=0x06, `FIRST`=0x07, `LAST`=0x08,
`ABSENT_FIELD`=0x09 (source `transducer.rs:198-201`). Source semantics (read-only check): `ASSERT` operand indexes a
capture-predicate table and evaluates against the path's captures (`transducer.rs:1671-1675`, `:2152-2157`);
`FIRST`/`LAST` take mode 0/1 (all/named), operand 0, and retain if the node is the first/last (named) child of its parent
(`:1676-1680`, `:2158-2181`); `ABSENT_FIELD` operand=field id (non-zero), retains if node has no child in that field
(`:1681-1685`, `:2182`). Source also defines interpreter limits `MAX_FINITE_JOB_INSTRUCTIONS=256`,
`MAX_STEPS_PER_PATH=1<<20`, `MAX_WORK_PER_TASK=1<<26` (`transducer.rs:223-225`) [D]. `SIZE` valid modes in source are
`0,1,2,0x80,0x81,0x82` (`:1664-1669`); transducer-jit-design.md:96 calls these "all six SIZE modes".
`transducer-design.md` also never mentions a capture-predicate table (fifth table) although `ASSERT` needs it.

### Alternatives, repetition, ordering (:128-173)
- FORK and navigation enumerate paths; they never change another path's captures; no undo stack/register environment; implementation may share immutable prefixes (:130-132).
- Logical output order is depth-first path order: (1) starting nodes in supplied order, (2) CHILD sibling order, DESCEND preorder, (3) FORK next-instruction path before target path, (4) captures in CAPTURE order along the path (:134-139). "Parallel execution must preserve that observable order" (:141).
- Programs without repeat instructions keep different successful paths as different outputs (:142).
- Programs WITH repeat instructions apply Tree-sitter capture containment within the same matched root: keep incomparable capture lists, remove a capture subset when another contains it, for equal lists keep the earlier path (:144-146).
- In-progress removal only at an equivalent continuation state where the earlier path's captures contain the new path's; "Repeat counters and repeat-node histories do not define equivalence" (:148-149).
- Repeated sibling pattern = FORK + NEXT + backward GOTO; example emits every nonempty prefix of consecutive named identifiers, `[a,b]` before `[a]` (:152-165).
- Cycle validity: must advance through a finite tape domain; a cycle of only tests/captures/branches is invalid; a cycle with NEXT or FOLLOWING progresses; parent/child oscillation invalid ("A changed node alone does not prove progress"); compiler rejects cycles without a monotonic traversal proof; zero-width nodes advance by ordinal (:167-173).

### Worked example (:175-219)
`eval` call: 8 instructions = 32 bytes (`KIND call; CAPTURE call; CHILD all; FIELD function; KIND identifier; TEXT 0; CAPTURE callee; EMIT match`) (:192-204).

### Output directives (:221-232)
- Ordered output directives are data, not callbacks (:223-224). `Set` adds key+optional value; `Offset` applies four signed row/byte-column offsets to every occurrence of one capture label; out-of-source adjusted point rejects the path; output keeps node identity, stores adjusted span (:226-229). Backends that cannot return directives must reject the Program (:231-232).

### Errors (:234-239)
Invalid opcodes/modes/indices/grammar IDs/targets/non-progressing cycles = errors; reachable fallthrough past last instruction is an error; missing source for TEXT = input error not negative; cancellation/exhaustion = incomplete output, "never a complete negative result" (:236-239).

### ast-grep comparison benchmark (:255-291)
- First ast-grep compiler accepts only exact root-terminal patterns with Smart strictness; rejects internal patterns, metavariables, other strictness with `UnsupportedPattern` (:257-259).
- Workload: 8 Python identifier patterns over 5,725 corpus files; all paths produced 12,967 ordered ranges, identical SHA (:267-272) [M].
- 5 measured iterations after 1 warmup; medians [M] (:274-281): ast-grep on Tree-sitter trees 12,608.58 ms (35.10x slower); ast-grep on tapes 817.68 ms (2.28x); transducers on tapes 359.22 ms (baseline). Clock type not stated (likely wall; "not found" for CPU vs wall).
- Preparation separate: Tree-sitter parsing 8,653.04 ms; tape encoding+restoration 4,574.91 ms; corpus 95,697,853 bytes (:283-286) [M].
- Scope disclaimer: applies only to the root-terminal subset; no structural/metavar/relation/full-policy speedup established (:290-291).

### Batch and anchor contract (:293-307)
- `execute_batch` accepts many programs and many tapes in one native op; result shape file -> program -> ordered outputs (:295-296). Without anchors each tape gets one shared leading-kind scan (:297).
- Caller may supply one ordered root-ordinal list per (file, program); empty list proves no candidate roots; executor starts only at supplied ordinals, does not search for replacements; lists must be valid strictly increasing ordinals else `InvalidAnchor` (:299-303).
- Anchor construction outside execution; preorder ordinals from anchor index or other sound source; missing/incomplete index coverage must use unanchored execution, never an empty list (:305-307).

### Backends (:309-339)
- CPU JIT (Cranelift) compiles every verified instruction to native control flow, calling typed runtime primitives for tape access/output allocation (:311-314). Same query-state arena as interpreter, interning captures, cursor states, repeat stacks (:316). [Later declared NOT a real JIT and slated for deletion by transducer-native-jit-design.md:11-13.]
- 128-comment quantifier stress: interpreter median 37.60 ms (5 optimized runs) [M]; Cranelift JIT warm compile median 0.90 ms, execution median 34.57 ms [M] (:321-323). "Runtime primitives dominate this query. Native control-flow compilation gives a small gain" (:325).
- Four backends: CPU interpreter, Cranelift CPU JIT, full Metal interpreter, Metal JIT target; all four accept the complete verified language (:327). Metal JIT generates one MSL switch for the complete Program batch with constant opcodes/modes/operands/targets; not a fixed-shape specialization (:329).
- Metal retains pending paths, captures, cursor states, repeat stacks, greedy alternatives, hashed query states in shared GPU buffers (:331).
- "The current Metal backend submits work per root. It also yields text, predicate, and output events to Rust. This behavior is transitional and does not meet the production batch contract. The required design uses one invocation for the complete batch and no host events." (:333-335)
- Exhaustive parity fixture: every opcode, every SIZE mode, both nav modes, both FORK modes, every valid FLAGS combination, through all four backends (:337).
- Separate Metal anchor scanner is a root-selection optimization, not a transducer JIT; old four-opcode Metal implementation and shape-specific kernel deleted (:339).

---

## 2. TD/transducer-value-program-design.md (435 lines) — value and output programs

Header: final Metal architecture is in metal-batch-executor-design.md, which "replaces the transitional Metal execution sections in this document" (:3-5).

- Purpose: second transducer domain for values; compile regex, Lua patterns, capture predicates into verified `ValueProgram`s; CPU and Metal JITs do not call Rust regex/Lua during execution (:9-13). Later `OutputProgram` applies the same model to directives (:15).
- Requirements (:19-32): preserve byte-oriented text behavior, ordered captures + multiplicity, regex leftmost-first, Lua greed/backrefs/frontier/balanced/position captures, positive+negative aggregate rules, per-path work limit; equal results on all 4 backends; "Execute one complete Metal batch with one kernel invocation"; keep TEXT_EVAL, ASSERT_EVAL, EMIT inside that invocation; evaluator tables in canonical Program identity; reject malformed evaluator control flow before execution; explicit error when an evaluator exceeds work/storage.
- No new user syntax; SCM remains source language (:34).
- Program model tree: `structural_words, value_programs, output_programs, strings, automata, constant_tables` (:40-48).
- Typed subprograms (:56-61): `ValueProgram<NodeText> -> Boolean`, `ValueProgram<CaptureList> -> Boolean`, `ValueProgram<CapturePair> -> Boolean`, `OutputProgram<CompletedMatch> -> KeepOrDrop`. Incompatible input type calls rejected (:63).
- Structural changes: `TEXT_EVAL value_id`, `ASSERT_EVAL value_id`, `EMIT output_id`; same opcode width; operand indexes verified subprogram table; old opcodes may coexist during migration but no mixed old/new evaluation for one operand (:67-77).
- ValueState: pc, input node or capture iterator, byte position+limit, aggregate state, Lua capture slots, Lua balance stack, pending alternatives, work count; indexes into immutable tape/tables, no owned text (:81-95). Every transition increments work; failed branch does not reset (:99).
- Byte instruction set (:105-112): Input `LOAD_NODE_TEXT, LOAD_CAPTURE_TEXT, RESET_BYTES`; Cursor `BYTE_NEXT, BYTE_SAVE, BYTE_RESTORE, BYTE_AT_START, BYTE_AT_END`; Tests `BYTE_EQ, BYTE_RANGE, BYTE_CLASS, BYTE_ANY, BYTE_FRONTIER`; Control `SPLIT, JUMP, CALL, RETURN, ACCEPT, REJECT`; Lua `CAPTURE_OPEN, CAPTURE_CLOSE, CAPTURE_POSITION, BACKREFERENCE, BALANCED`; Automata `DFA_START, DFA_STEP, DFA_ACCEPT, DFA_RESTART`. `SPLIT` stores preferred branch first (preserves greedy/non-greedy) (:114). Verifier rejects open capture at ACCEPT and backrefs to unavailable captures (:118).
- Regex lowering via `regex-automata` byte DFA: UTF-8 mode matching `regex` crate, unanchored unless anchored, leftmost-first, empty matches advance one code point during repeated replacement, deterministic table order (:122-130). Boolean predicates stop at first accept; replacement requests start/end (:134). Canonical identity uses normalized automaton + compile config (:136). [Superseded: native-regex-lua-lowering-design.md uses an ordered `ByteNfa`, not a DFA — see §7.]
- Lua lowering table (:144-155): literal/class = byte test + BYTE_NEXT; optional = preferred SPLIT; greedy `*` = preferred loop; non-greedy `-` = preferred continuation; `+` = one + greedy loop; frontier; balanced; captures; position capture; backreference. Transition order must match the recursive evaluator; differential tests use the old evaluator (:157-159).
- Capture predicate ops (:165-179): `CAPTURE_EACH, CAPTURE_ZIP, CAPTURE_TEXT, CAPTURE_KIND, CAPTURE_PARENT, CAPTURE_LINE, TEXT_EQUAL, TEXT_CONTAINS, KIND_IN, AGGREGATE mode empty_policy, SELECT_POLARITY`. `AGGREGATE` ALL/ANY + explicit empty policy; `SELECT_POLARITY` is not boolean negation — predicates track positive and negative aggregates separately to preserve mixed-list behavior for `#eq?`, `#any-eq?` and negations (:181-185). `#has-parent?` one parent step, `#has-ancestor?` all; same-line uses persisted line-start index (:187-189).
- CPU JIT: Cranelift private function per value program, `(program constants, tape view, structural path, work counter) -> result code`; compile identical value programs once per batch keyed by canonical digest (:193-205).
- One-invocation Metal batch (:207-257): one invocation executes the complete `(file, program, root)` batch; Rust packs all tapes, Programs, anchors, constants, task descriptors with stable batch ordinals (:209-211). Initial implementation: one Metal thread per task, owning the complete structural path stack (:213); "gives parallel work across roots and Programs ... preserves depth-first branch order inside each task" (:215). MetalTask descriptor fields: batch ordinal, file index, program index, root ordinal, pending-path stack, capture arena, cursor arena, repeat arena, query-state table, candidate-output arena, final-output range, error slot; host computes offsets; regions never overlap (:217-235). Phases inside the kernel: structural execution until stack empty -> value functions inline -> raw EMIT candidates in task arena -> capture-containment reduction -> delayed query predicates -> output programs -> append to shared result buffer (:237-245). Each task assigns local emission ordinal; output records (batch ordinal, local emission ordinal); "Atomic append order is not observable. Rust sorts result records by their two stable ordinals after the invocation" (:249-251). Each task writes an error record instead of stopping others; Rust returns the lowest-batch-ordinal error (:255). Empty batch = no invocation; non-empty = exactly one (:257).
- Metal interpreter reads all words from shared buffers; DFA tables packed `u32`; byte classes 256-bit masks or ranges (:261-267). Metal JIT: one MSL function per unique structural/value/output program, one generated batch kernel selects task Program; small DFAs may use switch with a "measured size threshold, not a pattern-shape rule"; JIT cache key = GPU registry ID, Program identity, MSL options, backend version (:269-281).
- OutputState (:287-299): ordered capture array, span overrides, text handles, capture property lists, match property list, dynamic property list, output byte arena, keep/drop. Directive ops (:303-310): `#set!` MATCH/CAPTURE_PROPERTY_SET; `#offset!` SPAN_TO_POINT + signed add + POINT_TO_SPAN; `#strip!` repeated regex + TEXT_COPY_EXCEPT_MATCH; `#select-adjacent!` reverse scan, line-gap test, stable compaction; `#downcase!` ASCII map; `#set-lang-from-mimetype!` MIME trie. Each op can return DROP (replaces `Option` from `apply_directives`) (:312). Line-start index needed in tape (:314). Arena full -> explicit storage error (:316).
- Directive order observable, source order; text op reads current override else span; span op doesn't discard override; capture removal preserves order; property insertion preserves source order and duplicate keys; "Tree-sitter structural reduction still occurs before query predicates and directives" (:322-332).
- Verifier list (:336-347): branch targets, table indices, input types, Lua slot validity on all incoming paths, iterator-loop progress op, DFA total transitions, initialized iterators, checked arena writes; computes max stack and slot counts for workspace allocation.
- Identity marker increments from `tempo-program-v6`; canonical identity includes value/output words, signatures, DFA, byte classes, Lua slot counts, constants, work-accounting version, regex config, directive data; machine code not hashed (:351-365).
- Tests: differential vs regex/Lua/capture_predicate_matches/apply_directives; all 4 backends; invocation counter exactly 1 per non-empty batch; host value event counter must be zero; run pinned 335 Python patterns (:369-399).
- Completion gates (:418-435): no JIT calls Rust regex/Lua; one invocation per batch; zero TEXT/ASSERT host events; all compatible SCM predicates verified; 4-backend equal outputs/errors; "The 128-comment query stays within the production latency and memory limits" [D, no number given].

---

## 3. TD/transducer-jit-design.md (100 lines)

- Header: superseded by metal-batch-executor-design.md for Metal (:3-5) and by unified-packed-runtime-plan.md where it describes collection-based runtime state or nested outputs (:9-11).
- Each Program word becomes one native basic block; direct branches native; constants for op/mode/operand/next/targets (:21-29). CPU JIT doesn't fetch/decode words; Metal JIT selects Program once then enters constant cases (:31).
- Runtime primitives: generated CPU code calls typed Rust primitives for managed data (tape nav, capture alloc, output alloc); "A primitive implements one transducer operation. It does not dispatch on a Program opcode." (:35-39) [This callback boundary is exactly what transducer-native-jit-design.md later condemns.]
- Unsupported verified instruction = compile error, no silent interpreter (:55).
- Query states: one arena shared by interpreter and JIT; paths hold integer IDs; arena interns ordered capture lists, saved-cursor states, active-repeat stacks, deferred greedy branches (:59-66). State key = (Program position, current node, cursor-state ID, repeat-state ID, sibling requirement) (:68). Capture containment uses capture-state ID's ordered list; exact lists use a hash index (:70).
- Greedy FORK policy 1 = deferred greedy fallback; later equivalent fallback replaces earlier; only for terminal root-child repetition with no captures (:74-76).
- Memory: Cranelift module owned by `CompiledPrograms`; CPU frame owns mutable state; Metal equivalent in shared buffers bounded by fixed work budget (:80-84). Current backend yields TEXT/ASSERT/EMIT events to Rust — transitional (:86-88).
- Default SQL terminal backend uses CPU JIT when it has production anchor lists; registry can select Metal interpreter/JIT for anchored batches; unanchored keeps shared leading-kind scan (:94).
- Parity: all opcodes, "all six SIZE modes", both nav modes, both FORK modes, all valid FLAGS; Tree-sitter differential tests remain the semantic oracle (:96-98).

---

## 4. TD/tree-sitter-query-quantifier-design.md (204 lines) — the semantic constraint that shaped the state model

- Reference: `tree-sitter` 0.27.0, `src/query.c` (:7).
- Scope: `?`, `*`, `+` on nodes, groups, alternatives, captured/uncaptured, adjacent quantified forms, forms with required continuations (:11-18). Grammar.js quantifiers out of scope (:20).
- Reference model: TS compiles to `QueryStep`s; quantified step has an alternative transition; cursor keeps concurrent `QueryState`s each with (pattern+step index, matched root depth, ordered capture list, immediate-sibling requirement, status of longer alternatives) (:24-32). After a required occurrence `+` can repeat or continue; `*`/`?` also have zero-occurrence transition; "Tree-sitter keeps each viable transition until it fails or becomes redundant" (:34-39). "This behavior is backtracking through concurrent states. It is not one global allocation of nodes to greedy quantifiers." (:41)
- Example `(module (comment)+ (comment) @c (comment)+)`: 5 comments -> 3 matches, `@c` = comments 1,2,3; N comments -> `max(N-2,0)` matches; uncaptured repetitions may have multiple internal paths per `@c` that do not produce additional observable matches (:45-58).
- Longest-match rule: compare only within same pattern + matched root; compare ordered capture lists of `(capture index, tree node identity)`; containment = superset in tree order; discard contained state when both at same query step and immediate-match conditions permit; equal lists equivalent (:62-72). TS does NOT compare repeat counters, repeat IDs, private allocations of uncaptured nodes, greatest-repetition-count vectors (:74-79). "A completed state remains pending while a longer capture alternative can still complete." (:81)
- Tempo observable state: pc, matched root, current node, saved cursors, ordered captures, immediate-sibling requirement (:87-94). "The executor must not choose one transition before it knows that the other transition cannot produce a distinct match." (:96) Reduction only between states with same Program, root, pc, equivalent cursor state, equivalent sibling requirement; then capture containment; "must not use repeat history as a substitute" (:98-106).
- Three correctness layers: (1) query-state arena interning captures/cursors/repeat stacks; (2) executor removes a new equivalent state when an earlier state contains its captures; (3) completed outputs from the same root reduced by containment (:110-114). Equivalent in-progress state = same pc, current node, saved cursors, active-repeat stack, sibling requirement (:116). [Note: here the active-repeat stack IS part of in-progress equivalence, while :74-79 and transducer-design.md:149 say repeat counters don't define equivalence — the distinction is stack identity vs counter values; the doc does not reconcile explicitly.]
- Compiler rules for empty quantified items with anchors (:120-125). Executor never removes an earlier state contained by a later one ("The earlier state can already have produced descendants") (:127). Greedy FORK only for terminal root-child repetition without captures (:129). Liveness: SCM compiler does not save a last-child cursor unless a later negated-field test reads it, "keeps equivalent repeat states equal" (:131). Repeat-event vectors not part of state or output selection (:133).
- Ordering: finished matches ordered by matched root then first capture position; stable discovery order for ties and captureless matches; nested matched roots are a known exception — TS can order nested-root matches by first capture before root order, Tempo keeps root order first (:137-141).
- TDD plan: differential tests for sibling counts 0..8+ comparing count, order, root spans, capture names/spans/order/multiplicity (:149-170). Implementation gate removes repeat-history selection (:174). Optimization gate: in-progress reduction only after completed-output baseline passes; stress with hundreds of comments recording state count, time, peak retained bytes (:176-180).
- 128-comment test records (debug build) explored states, max pending paths, estimated peak retained bytes, interpreter time, JIT time, with fixed limits on all four; retained bytes sampled every 1,024 states (:188-198). Actual values: not found in this doc.
- Failure: exceeding budget returns `Error::WorkLimit`, never partial/negative; unrepresentable TS transition = compile failure, "must not use an approximate quantifier heuristic" (:202-204).

---

## 5. TD/transducer-capture-run-cpu.md (211 lines) — specialized CPU operator for one execution class

- Status: CPU component executes "a proved class of compiled transducers"; no backend routing change; does not close phase 5; GPU corpus failure remains open (:3-6).
- **Problem statement / root cause (:10-12):** "The frozen corpus run spends 380 dispatches on nineteen successor generations. Deep captured repetition retains millions of states after its repeat arenas become empty. Copying each capture prefix repeats the same values across many states." [M for 380/19; "millions" A]
- Interface: immutable `Program`, `TapeDoc`, one root, work limits -> complete `Output`s + counts; no SCM compiler/anchor/SQL/publication in interface (:14-17).
- Supported class: one normal sibling repetition; finite acyclic body with guards before captures and distinct kind alternatives; initial and repeating bodies identical guard+capture paths; per-navigation named/all mode retained; suffix selects following siblings by kind, forward jumps, deferred assertions, optional cursor restore, emission; prefix may have root guards, captures, save of cursor 0; at most one restore (:19-25). Valid programs outside -> `UnsupportedBackend`; "no automatic CPU fallback in Metal" (:27-29). Unnamed capture event that named repetition would skip excludes a region (:30-31).
- Regex-injection program 277 outer prefix enters the region; saving a lower cursor slot truncates higher slots (:33-37).
- Execution (:41-53): one immutable capture-event arena per sibling domain; fixed record of five `u32` = 20 bytes: half-open capture range, entry position, stop position, inherited path work; positions refer to the sibling domain not tape ids. Admission at repeat header by complete range identity; earliest logical predecessor wins an exact join; every attempted admission consumes task work including duplicates; duplicates retire before successor execution; only arrivals for next eligible node remain active; "retains no historical admission table".
- Exit processing groups equal capture ranges in original logical order; terminal ordering assigns raw ordinals before containment; interval reduction selects maximal capture ranges "under a checked geometry proof"; only retained outputs materialize full capture records; shared finalizer does predicates/directives/values/order (:55-65).
- "CPU reference operator with a GPU-compatible record and stage shape. It is not a GPU memory-ledger implementation." (:67-70)
- Measurements: 1,093 generated mixed sibling domains compared (:79) [M]. 350-interpolation string fixture: 352 captures from 351 shared body events; 123,551 compact exits; 61,777 raw terminal candidates; 31,740,672 semantic task-work units; 1.873737 s debug build (:91-96) [M]. Release interpreter oracle at 350 interpolations: 163.242775 s, 2,324,545,536 bytes peak owned RSS (opt-level 3, no LTO, 16 CGUs) (:98-102) [M].
- Benchmark medians, debug, 3 samples after warmup, host not exclusive (:106-123) [M]: 8 pairs: interpreter 0.003967208 s vs capture-run 0.000240375 s (9 captures); 32: 0.338971500 vs 0.002599042 (33); 64: 3.478668333 vs 0.019535750 (65). Interpreter grows super-linearly (~10x per doubling 32->64); disclaimer: not release throughput/corpus/GPU latency (:122-123). Clock type: not stated.
- Limits restated (:147-148) [D]: 600 dispatches, 2 billion tracked Metal bytes, 4 GiB application RSS, 1 GiB compiler RSS; later target below 100 dispatches open.
- Evidence table (:155-172) [M]: 821 baseline names + 14 added; native selection 808 pass / 0 fail / 27 ignored in 1159.87 s; Linux 371 pass / 22 ignored; pinned workload: both 335-pattern five-file differentials preserve all 109,114 matches; each uses 402 dispatches and 142,098,432 tracked bytes; aggregate owned memory peak 2,731,622,400 bytes; compiler memory peak 185,024,512 bytes; 234 source/executable hashes unchanged.
- Progress: "four accepted phases out of five"; "The true measure of phase-five completion is complete exact GPU execution inside the unchanged limits." (:186-187)
- 2026-10-26 addendum (:190-211): outer entry and streaming continuation; streaming path retains scalar counts and capture envelopes instead of one descriptor per logical exit (:197-198); 25 focused tests; 350-part outer fixture 0.003684167 s, one debug sample (:205) [M]; production routing unchanged.

---

## 6. TD/transducer-native-jit-design.md (751 lines) — native IR shared by Cranelift and MSL

- Status: scheduler design in native-jit-scheduler-completion-plan.md; regex/Lua in native-regex-lua-lowering-design.md (:5-6). Replaces CPU JIT sections in older docs (:8-9).
- **Key verdict (:11-17):** "The current Cranelift implementation is not a JIT for transducer semantics. It generates a dispatcher that calls Rust for each semantic operation. This implementation must be deleted." Current Metal JIT has no host callbacks but uses generic GPU functions for some subprograms.
- Goals (:23-31): `Program` only semantic input; both interpreters direct `Program` interpreters; one ephemeral native IR for both JITs; Cranelift CPU; MSL Metal; common packed ABI for all four backends; no host callback from generated code; compile+execute all 335 pinned Python Programs; exact output/error/capacity/ordering.
- Non-goals: native IR not stored, not executed by interpreters, doesn't replace Program, no SCM/regex/Lua parsing, no general VM (:35-39).
- Program parts (:43-51): structural words, string+integer constants, `ValueProgram`s, `CaptureProgram`s, one `OutputProgram`, stable labels, TS query behavior flags. Word format unchanged (:53-60).
- Pipeline: verified Program -> native module builder (structural, value, capture, reduction, output functions) -> native IR verifier -> Cranelift emitter / MSL emitter (:66-86).
- NativeModule {task, values, captures, reduction, output}; NativeFunction {parameters, result, entry, blocks}; NativeBlock {parameters, operations, terminator}; SSA `ValueId`, `BlockId` stable (:93-118).
- Types: Bool,U8,U16,U32,U64,I32,Offset32; refs ImmutableRef/WorkspaceRef/ResultRef = (offset, region identity), not host pointers (:122-146).
- Ops: Constant, Add/Sub/MultiplyChecked, BitAnd, BitOr, Shl, Shr, Equal, NotEqual, LessThan, LessThanOrEqual, Select; memory LoadField, StoreField, LoadIndexed, StoreIndexed, BoundedAppend, BoundedByteAppend; state-table StateLookup/Insert/Replace comparing complete key ("A hash match alone is not sufficient"); terminators Branch, BranchIf, Switch, CallInternal, Return, Trap; no external call (:150-202).
- Task inputs: PackedBatch view, PackedWorkspace view, PackedResults view, task ordinal; no Rust object, no callback table, no allocator (:206-218).
- Task isolation: disjoint mutable ranges assigned before execution: pending paths, capture links, cursor links, repeat links, query states, value work, raw candidates, final results, final captures, properties, dynamic properties, transformed text, task status; exhausted range -> one stable capacity error; no partial result after error (:222-243).
- Structural lowering: one entry block per instruction; each instruction increments task work, `WorkLimit` on exceed (:247-251). Tests: KIND, FIELD, FLAGS, TEXT, SIZE, ASSERT, FIRST, LAST, ABSENT_FIELD, SUPERTYPE (:255-266). Single-result nav NEXT/PARENT/RESTORE_CURSOR (:277-285). Multi-result nav CHILD/DESCEND/FOLLOWING: enumerate targets in order, store deferred targets in reverse, continue with first (:291-298).
- Links: SAVE_CURSOR appends immutable cursor link (slot, node, prior link offset); RESTORE reads newest link for slot (:302-306). CAPTURE appends capture link (label, node, ordinal, prior offset); text not copied (:310-314). REPEAT_BEGIN appends link count 0, BUMP appends count+1, END selects the link prior to the active repeat; "does not create repetition vectors. Repeat history does not define query-state identity." (:318-323). FORK copies current path into pending range (target pc), current continues at next; greedy mode uses complete query-state key (:327-332). EMIT appends raw candidate (root, label, capture head, local ordinal); delayed predicates/directives do not run in this block (:346-350).
- Scheduler pseudo-code (:356-373): init root path; loop execute; on resume pop pending; then reduce raw candidates, run capture programs, run output program, write final packed records, write task status. Bounded ranges.
- Value lowering: Equal/Prefix byte loops; Regex over `ByteNfa` (ordered NFA state set, epsilon+look transitions in stable order, leftmost-first, stop at first accept); Lua over `LuaProgram` (:379-429).
- CaptureProgram variants: Equal, Text, Any-of, Kind, Contains, Parent relation, Ancestor relation, Same-line; exact any/all/negation/empty-list (:433-448).
- Candidate reduction grouped by matched root; with repetition -> ordered capture containment; without repetition -> keep distinct paths; stable, no hash iteration order (:452-461).
- Output instructions: Match property, Capture property, Offset, Strip (ByteNfa), Select adjacent, Downcase, Language from MIME; only transformed text enters transformed-text range; writes final records directly, no nested output collection (:465-481).
- Cranelift: no imported Tempo function, no Rust function address (:487-495). Metal: one GPU thread per task; one non-empty batch = one kernel invocation, zero host events; generated MSL doesn't call the interpreter (:499-505).
- Cache key: backend kind, native IR version, packed ABI version, target identity, compiler options, ordered canonical Program identities (:517-527).
- Errors: same typed code + task ordinal; stable error order by task ordinal; no partial output after WorkLimit, workspace capacity, result capacity, invalid packed input, invalid instruction, unsupported backend, backend execution failure (:531-542).
- Deletion requirements: delete callback-based `jit.rs`, `JitFrame`, every `extern "C"` helper, every `JITBuilder::symbol`, imported functions, nested `Vec<Vec<Vec<Output>>>` result; "Delete or correct each benchmark claim that used this implementation" (:546-558). [So the 0.90 ms / 34.57 ms JIT numbers in transducer-design.md:323 were produced by the condemned callback JIT.]
- TDD ladder Steps 1-14 (:566-696): includes Step 6 gate "The 128-comment stress test has exact four-backend parity" (:610); Step 11/12 compile and execute all 335 programs, byte-for-byte equal packed outputs (:643-670); Step 13 Metal JIT on Apple, CPU JIT when Metal unavailable, "Do not add fallback after compilation or execution failure" (:672-680); Step 14 measure initial exec time, incremental p50/p95, workspace bytes, RSS, publication time, DB size (:682-696).
- Work plan phases A-F (:700-737); completion criteria (:741-751).

---

## 7. TD/native-regex-lua-lowering-design.md (848 lines)

- One control-flow structure for both targets; emitters differ only in types, address spaces, syntax (:8-9). Completes `ValueProgram::Regex(ByteNfa)` and `ValueProgram::Lua(LuaProgram)` for structural TEXT (:16-22). No new parsers (:24-25).
- Required properties (:31-40): same CFG; no host call; only packed views + scalars; one bounded `value_work` range per task; checked appends; work exhaustion -> `WorkLimit`; range exhaustion -> `BackendCapacity`; invalid data -> `InvalidOperand`; no partial output; exact regex state order / Lua alternative order. No backend may fall back to another backend or to the direct CPU evaluator after an error (:42-43).
- Existing Metal runtime already contains the algorithms (`look_matches, nfa_closure, evaluate_nfa, nfa_ordered_closure, nfa_find, lua_class, lua_match, lua_push, evaluate_lua`); they are the control-flow reference, not long-term semantic authority (:47-60).
- Packed records `ValueRecord, NfaStateRecord, NfaRangeRecord, LuaInstructionRecord, LuaSetMemberRecord` in `metal_batch_abi.rs`, target-neutral (:70-82). Generic value-runtime functions + small Program-specific wrapper binding one `ValueRecord` index (:84-91).
- ValueRecord fields (:95-101): kind 2=regex/3=Lua; operand = unanchored NFA start / Lua flags+capture count; auxiliary = anchored NFA start; state_start; state_count. Lua operand bits: 0 anchored, 1 end-anchored, 2..31 capture slot count (:103-109).
- NfaStateRecord kinds (:113-120): 0 bytes(range start,count), 1 look(kind,next), 2 split(target start,count), 3 capture(next), 4 fail, 5 match. Regex captures are epsilon for boolean evaluation (:122-124).
- LuaInstructionRecord kinds (:128-136): 0 atom, 1 frontier, 2 balanced, 3 capture start, 4 capture end, 5 position capture, 6 backreference.
- Wrapper signature `value(task_view, node_ordinal, path_work) -> ValueOutcome`; `ValueOutcome {Reject, Keep, Invalid, WorkLimit, Capacity}`; "The current Metal helper uses one code for both errors. The shared lowering must remove that ambiguity." (:183-216) [root cause noted: conflated error codes in Metal].
- Error mapping table (:277-285): Keep -> next instruction; Reject -> resume scheduler; Invalid -> stop task InvalidOperand; WorkLimit; Capacity -> BackendCapacity. Errors clear all visible result counts; batch error selector = smallest task ordinal (:287-288).
- Work accounting: path work counter shared, no private limit; increment before each logical step; regex closure pop = 1 step, regex range test = 1, Lua instruction dispatch = 1, balanced byte scan = 1; same rule in CPU evaluator and Metal interpreter (:292-306).
- Regex workspace: `4*n` words for n NFA states: seen, closure_stack, current_states, next_states (:312-323). Closure LIFO, mark on pop not push; split targets appended in reverse so pop visits stored order; first visit wins (preserves ordered NFA semantics after dedup) (:330-338). Byte loop: clear seen, visit current in order, ranges in order, closure, swap; "Do not sort state sets. Do not use a hash set. Do not merge equal byte ranges." (:352-366). Look ops list (:373-383), Unicode via packed word-range table (:385-386); invalid UTF-8 -> scalar zero per Metal contract (:388-390). `nfa_find` for directive spans: anchored start at each candidate byte, ordered closure keeps match states, fallback end logic preserves leftmost-first+greedy (:396-407).
- Lua: 3 words per capture slot (status 0 unset/1 open/2 closed/3 position; start_or_position; end) (:416-431). Alternative record `2 + 3*s` words; capacity formulas (:439-460). Unanchored appends all source positions in reverse incl. `source_end` (:464-471). Repetition table min/max/order; greedy appends min..max so pop evaluates max first; non-greedy reverse (:475-490). Byte classes %a %c %d %g %l %p %s %u %w %x %z, uppercase negates, byte semantics not Unicode (:497-517). Frontier, balanced, captures/backrefs (:521-551). End conditions; first successful alternative -> Keep (:555-563).
- Only target details differ (table :580-587): scalar form, immutable/mutable address qualifiers, local refs, internal call, task dispatch (CPU worker call vs one GPU thread).
- File responsibilities and migration order (:620-669): delete hand-maintained MSL regex/Lua helpers after parity; architecture checks reject semantic state switches in emitters.
- Pinned input composition (:794-799) [M-ish count]: 11 equality programs, 16 Lua programs, 8 regex programs among the 335.

---

## 8. TD/sql-driven-transducer-execution.md (196 lines) — SQL-driven execution as designed in Tempo

Status: "the terminal virtual table exists for the fixed-word executor. Production policy composition does not use it yet." (:3)

### File filter (pre-transducer)
- Persisted conservative source facts, "four-lane bit sets, not exact value postings" (:7-8). `source_anchor_facts` per (source, grammar): kinds present, small exact terminal values present, required text trigrams present (:10-14). `candidate_anchor_facts` per (check, grammar): required facts + possible root kinds, from `AnchorPlan::candidate_facts()` (:16-18).
- Publication SQL compares lanes with bit ops and inserts each possible pair into `workspace_file_checks` (clean snapshots: `snapshot_file_checks`) — "exact materialized file/check eligibility after conservative filtering" (:20-23).
- Indexes: PK `(workspace_id, check_id, path_id)`; `workspace_file_checks_by_file(workspace_id, path_id, check_id)`; snapshot equivalents; PK `(source_id, grammar_id)`; PK `(check_id, grammar_id)`; catalog/completion indexes (:25-32).
- Completion rows in `workspace_candidate_files` / `snapshot_candidate_files` gate visibility: "An incomplete publication cannot prove that a file/check pair is absent." (:34-35)

### SQL -> matcher path
- Runtime reads the complete candidate manifest before worker evaluation in one joined SQL statement; returns stable `CandidateCheck` identities, not SQLite ids (:39-41). `restore_persisted_file()` replaces `file.candidates.ast` with the stored subset; `candidate_dispatch()` builds `Candidates.by_kind` and `Candidates.unrestricted`; `evaluate_ast_file()` builds `FileAnchorIndex` only for remaining checks, one `Selection` per check; "rejected file/check pairs never enter the matcher loop" (:43-53).
- GPU anchor API accepts a per-file program bit set with no 32-program limit; kernel reads group bit before emitting a hit; disabled pairs do no literal work (:55-59).

### Search index (schema 12)
- Replaces file-by-predicate posting tables with three contentless FTS5 tables independent of the policy catalog (:63-64).
- `search_kind_fts`: one kind token per preorder node; FTS token offset = preorder ordinal; `search_kind_instances` exposes exact `(kind, tape, ordinal)` rows for structural roots (:66-68).
- `search_value_fts`: each distinct bounded `(kind, exact value)` token once per tape; gives file eligibility not positions; terminal executor confirms; "removes the file-by-predicate Cartesian product" (:70-73).
- `search_text_fts`: source words, no extra source copy; conservative eligibility for regex words and comment searches; always confirmed (:75-77).
- `search_index_sets`: tape, index version, node count, completion; "A complete FTS miss proves that a bounded exact value is absent from a tape. A missing index set means unknown eligibility. Unknown eligibility widens to terminal scanning." (:79-82)
- Publication in batches of at most 64 tapes; delete+insert by immutable tape identity; GC deletes FTS docs before tape (:84-86).

### Terminal virtual table `tempo_matches(batch)` (:88-120)
- Eponymous-only SQLite vtab; hidden required `batch` column via `xBestIndex`; batch lives in Rust (programs, tapes, optional preorder anchors); JS never gets the token/tape ids/connection (:90-93).
- `xFilter` receives one batch token; cursor calls `execute_batch()` ONCE for all file/program pairs; a `OnceLock` caches terminal rows so a query referencing the relation twice doesn't execute twice (:95-98).
- Columns: `batch, file_index, program_index, match_index, start_ordinal, output_label, capture_index, capture_label, capture_node, start_byte, end_byte, complete, error_code` (:102-116). One row per capture (flattened).
- Complete negative pair = one status row with null `match_index`; failed pair `complete=0` + error code; "SQL must inspect completion before it uses absence." (:118-120)

### SQL as outer rule engine (:122-166)
- Example query: MATERIALIZED terminal CTE; `complete_pairs` = `min(complete)` per (file, program); matches = complete and non-null match_index; join `rule_programs` on program_index; group by rule/file; `HAVING count(DISTINCT match_index) >= r.minimum` (:126-145).
- SQL then implements: joins between captures of different programs; counts/ranges/grouping/thresholds; cross-file relations; rule alternatives with priority columns; finding templates from relational columns; absence checks after a completion join; incremental invalidation joins by tape and catalog identity; stable output order with explicit logical keys (:147-156).
- Scalar functions only for pure per-value ops (hash compare, small posting decode, collation, formatting); "A scalar function must not run one matcher for each file or rule row. That design recreates the nested execution loop." (:158-161)
- Use virtual tables for batched terminal work and posting expansion; future `tempo_postings(batch)` for decoded preorder lists; future Metal-backed `tempo_matches(batch)` selecting the GPU backend; "The SQL schema and query do not change when the backend changes." (:163-166)

### Execution phases (:168-186)
1 SQL selects eligible file/program pairs; 2 SQL loads sparse FTS eligibility and kind positions; 3 Rust converts eligibility to one bit set per file; 4 one GPU dispatch finds exact roots for all eligible pairs; 5 `tempo_matches(batch)` exposes matches + completion rows; 6 SQL joins terminal rows into higher-level findings (:170-177). For exact literals, the GPU anchor kernel proves the complete terminal predicate and the runtime emits terminal rows directly, omitting a second dispatch; structural programs still need terminal verification (:179-182).
- "A dispatch loop is valid only when a hard device limit requires partitioning. The runtime must record that limit and the number of partitions. It must not split work to hide contention or setup costs." (:184-186)

### Correctness boundaries (:188-196)
Fixed-word compiler supports only exact Smart root terminals; vtab cannot add missing compiler semantics; structural patterns, metavariable equality, ellipses, alternatives, relations need compiler support first (:190-192). SQLite must not infer absence from missing index rows, cancellation, exhaustion, or backend failure (`complete=0`) (:194-195).

Later contrast: metal-production-grid-cutover-design.md:217-225 restricts SQL to candidate-file selection, anchor postings, normalized result storage and paging; "SQL does not execute `Program` instructions. It does not expand one row for each `(file, Program, root)` tuple." And metal-execution-cutover-plan.md:10 puts "anchor selection, SQL planning, virtual tables, and durable publication" OUTSIDE the cutover scope.

---

## 9. TD/metal-optimal-execution-shape.md (380 lines)

- Header: cutover plan supersedes older limits/admission policy; temporary hard limits 600 launches per complete evaluation and 2,000,000,000 application Metal-buffer bytes; later optimization must reach <100 launches; active-state reduction uses exact equality; completed candidates get separate exact + containment reduction; non-progressing cycles rejected upstream; "No historical production admission index is permitted." (:4-8) [D]
- Replaces the persistent-lane scheduler in metal-root-streaming-continuation-design.md; production-grid doc is more specific (:12-14). "The executor shape is the primary constraint. Input packing, Program lowering, continuation, and publication must conform to this shape." (:17)
- Hardware [M] (:21-27): Apple M5 Max, 40 GPU cores, SIMD width 32, max threadgroup 1,024, recommended working set 115,448,725,504 bytes.
- **Root cause stated:** "The current production kernel dispatches 64 threads in two SIMD groups. Thus, most GPU cores receive no SIMD group from the command." (:29-30) "A larger persistent lane pool improves time only by replicating large per-lane arenas. This approach uses memory to compensate for the wrong execution shape." (:32-33)
- Target: millions of independent threads per dispatch; 256 threads/threadgroup unless measured otherwise; thousands of threadgroups; no fixed pool of 64 persistent threads; a thread performs one small bounded unit of work and does not own a maximum-size query arena (:37-44).
- Shape spike (synthetic, no query semantics) [M] (:48-57): flat arithmetic + 1 global counter, 82,908,643 threads, 2.34 ms; nine Program probes per root, 9,632,501 threads, 0.35 ms; one count per root 1.31 ms; node+Program loads with one navigation load 6.43 ms; mixed four-step work with 64-step work on 1/64 roots 11.14 ms.
- "The production interpreter takes approximately 98 seconds with 64 persistent lanes. The measured difference is not query count or corpus size. The difference is execution shape." (:59-60) [M, approximate]
- Full-shape spike [M] (:64-80): classification, bounded navigation, 16 frontier waves, vector scans, page boundaries, bounded scatter; 9,632,501 roots, 37,436,359 results, 45,037,455 captures, 1,932,340 properties, 9 bounded output pages, 34 Metal invocations; median command time 40.04 ms; 154,120,016 input bytes, 323,859,045 scratch bytes, 234,881,024 output bytes. Does not implement Program semantics. Artifact `tempo/benchmark/history/ner-1562-metal-full-shape-spike-2026-10-16.json`.
- Integrated coverage kernel over the complete production CPython workload [M] (:85-89): 82,908,643 tuples in 323,894 blocks and 759,585 spans across 68 existing command partitions; kernels 106.91 ms total, slowest partition 5.88 ms; packed representation 358,085,256 bytes vs 1,989,807,432 bytes of `MetalTask` records = 82.00% reduction.
- Shape classification of production tuples [M] (:91-100): 25,618,909 direct; 27,659,506 direct with predicates/output work; 19,122,092 bounded structural; 4,766,624 bounded with predicates/output; 5,741,512 dynamic-frontier. Direct 64.26%, bounded 28.81%, dynamic 6.93%.
- "This result isolates semantic lowering and execution as the remaining Metal problem. The broad dispatch, frontier, scan, continuation, and page-scatter shape is viable." (:102-103)
- Work representation (:107-147): no 24-byte task per tuple; no persistent thread pulling from atomic cursor; `WorkBlock{span_start, span_count, logical_start, logical_count}`, `WorkSpan{evaluation, root_start, root_count, block_lane_start}`; block <= 256 tuples, may mix small evaluations; ~1 block per 256 roots, ~324,000 blocks for 82.9M; thread derives evaluation/file/program/root/stable task ordinal; span references interned root selector; preserves `(file, Program, root)` identity.
- Work classes (:151-194): "One divergent interpreter kernel is not the default execution path." Class 1 direct root tests (one kind / choice of kinds / one token / choice of tokens / one capture / constant output directives; one thread per root, exact output count). Class 2 bounded structural paths (fixed child/field/parent/sibling paths; straight-line Metal per structural signature; registers; no max-capacity tables). Class 3 bounded predicates on compacted structural candidates. Class 4 dynamic query frontiers (quantified siblings, forks); SoA frontier buffers; one thread per frontier state; scan compacts next frontier; "The maximal capture antichain becomes a segmented reduction over equal query-state keys." Class 5 output transforms on compacted accepted matches with exact counts before allocation.
- Program grouping by execution signature (structural opcode shape, root kinds, capture count/shape, predicate class, output class, dynamic-frontier requirement) (:198-207). Measured catalog [M]: 106 exact opcode signatures; largest two contain 70 Programs; largest 20 contain 212; 61 signatures contain one Program each (:209-211). Do not dispatch per signature; sort blocks by signature, keep threadgroups signature-homogeneous, uniform threadgroup-level switch avoids SIMD divergence; 335 Programs must compile into <20 broad classes (:213-220).
- Deterministic output without reservation atomics: count (thread registers -> one aggregate per block: results, final captures, properties, output bytes; no per-tuple global count), segmented prefix scans in semantic order, first tuple exceeding any page capacity = continuation boundary, scatter only before boundary, no atomic reservation; "discovers output exhaustion before any bounded page write fails" (:224-256).
- Dynamic-state memory classes: registers (fixed paths), threadgroup memory (short reductions), compact global frontier buffers, compact match descriptors, fixed output pages; scale with active frontier states not resident threads x worst task (:260-271).
- Input residency: pack complete CPython input as one Metal-resident corpus when limits permit (nodes, child+supertype data, source, file prefixes, Program data, anchor postings/compressed root ranges, work blocks/spans); "Do not rebuild these buffers for each 32-file publication batch. Publication boundaries do not define execution buffers." (:275-290)
- Invocation shape target <40 for measured corpus; 100 = diagnostic threshold, not production limit (:294-295); representative plan totals <=40 (1 classification, 2 counts, 3 scans, 2 structural scatter, <=16 frontier, 4 predicate/output, <=12 output-page continuation) (:297-307); "Production has no fixed invocation limit." (:310) [Contradiction: header :5 and cutover plan impose a hard 600 limit per evaluation.]
- Acceptance [D] (:368-380): <40 invocations normal; >100 fails benchmark; thousands of 256-thread groups; no per-tuple fixed record; no transient arena scaling with worst-task x resident threads; count/scan/scatter; exact continuation; four backends exact; **production Metal execution < 5 seconds on measured machine; host prep+validation < 2 seconds.**

---

## 10. TD/metal-batch-executor-design.md (894 lines) — the one-thread-per-task executor (precedes the grid)

- Status: unified-packed-runtime-plan.md supersedes it for host workspace layout, backend dispatch, output ownership, result storage; remains authoritative for kernel behavior and one-invocation contract (:5-8). Replaces transitional Metal design (:11).
- Claims (:13-27) [A]: Metal interpreter executes complete packed batches; task-probe kernel gone; each non-empty batch = one invocation, no host events; kernel executes structural, value, capture, candidate, output programs; JIT generates native structural branches but still generic value/capture/output functions (Unit 9 incomplete); native library suite and 128-comment release stress pass; all 335 patterns parse and lower; full production execution and perf certification incomplete.
- Completion contract (:31-54): exactly one invocation; every `(file, program, root)` task; TEXT_EVAL/ASSERT_EVAL/EMIT inside kernel; no host events; both Metal backends = CPU outputs and errors; order independent of GPU scheduling; no dependence on query names, source paths, fixed Program positions. Metrics `kernel invocations = 1`, `host events = 0`.
- Non-goals (:58-69): CPU execution followed by no-op GPU invocation; host loop per root; host callbacks for text/capture predicates/directives; source-pattern switch; query-name switch; fixed Program-shape kernel; partial output after error; silent interpreter fallback from JIT.
- Execution model (:73-91): one immutable `MetalBatch` (all tapes, Programs, constants, tasks, workspace ranges); one thread per `MetalTask`, start to finish; no shared mutable query state; seven phases (init root path; execute structural paths; store raw EMIT candidates; structural candidate reduction; delayed capture predicates; output programs; final records); one final status per task.
- GPU format separate from canonical Program format; fixed-width integers; indexes/offsets only; explicit counts; checked u32 conversions (:95-105).
- Records: `MetalBatchHeader` (format_version, task/file/program/node/child/supertype/line_start/string counts, structural/value/capture/output instruction counts, workspace_byte_count, candidate_capacity, result_capacity, output_byte_capacity) (:110-128). `MetalFile` (node_start/count, source_start/length, line_start/count, supertype_start/count) — node indexes global within batch, source offsets global (:135-148). `MetalNode` (parent, next_any, next_named, child_start, child_count, descendant_end (exclusive), source_start, source_end, kind, field, flags) (:153-168). `MetalSupertype(node, kind)` sorted, binary search (:173-180). `MetalString(byte_start, byte_length)` shared byte buffer (:185-191). `MetalProgram` (structural/long/string/value/capture/output/label start+count, maximum_cursor_slot, maximum_lua_capture_slot, flags incl. TS query semantics) (:196-217). `MetalTask` (batch_ordinal, file, program, root, then start+capacity for pending, capture, cursor, repeat, query_state, value_state, candidate, result, output_byte, plus error_slot) (:222-249).
- Workspace bounds (:253-282): verifier computes Program-level static bounds; packer combines with tape/root facts: max pending paths, live capture/cursor/repeat links, query-state entries, value alternatives, Lua capture slots, raw candidates, final results, transformed bytes; "must be conservative. It must never allocate less space than a valid execution can use." Work limit bounds cyclic control flow; root descendant count bounds traversal; CFG bounds simultaneous branches; label count bounds capture slots; directive list bounds property writes. If a safe bound exceeds a Metal buffer limit -> `Error::BackendCapacity`; "must not use a smaller heuristic allocation". Checked sums.
  [Root cause per later docs: this conservative per-task worst-case reservation, multiplied by resident lanes, is what made lanes expensive and limited the pool to 64 — see root-streaming :143-144 and optimal-shape :32-33.]
- `MetalPath` (program_counter, node, match_start, capture_head, cursor_head, repeat_head, work, following_mode, capture_hash, cursor_hash, repeat_hash); pending = task-local LIFO stack preserving DFS; capture/cursor/repeat are immutable links, path stores heads (:286-305).
- Interpreter opcode names here differ from transducer-design.md (:312-336): `FAIL, KIND, FIELD, FLAGS, TEXT_EVAL, SIZE, ASSERT_EVAL, FIRST_CHILD, LAST_CHILD, NEGATED_FIELD, SUPERTYPE, CHILD, NEXT, PARENT, DESCENDANT, FOLLOWING, SAVE, RESTORE, CAPTURE, REPEAT_PUSH, REPEAT_REQUIRE, REPEAT_POP, FORK, JUMP, EMIT` (25 names, same count as source's 25 constants; naming contradiction — `REPEAT_REQUIRE` vs source `REPEAT_BUMP`). Work increments at same transitions as CPU (:340-341).
- Query-state key: pc, current node, following-sibling requirement, capture-state id, cursor-state id, repeat-state id; exact elimination requires full equality after hash; containment uses ordered captures and node identity, not repeat histories/counters; greedy FORK uses same equivalence as CPU (:345-360).
- Value-program format `MetalValueProgram(kind, instruction_start/count, table_start/count, capture_slot_count, flags)`; kinds Equal, Prefix, Regex NFA, Lua byte program (:364-386). Regex: ordered Thompson NFA, two ordered state lists, epsilon closure preserving split order, first-occurrence dedup, all regex-automata look assertions, Unicode via generated tables (no ASCII approximations), leftmost-first, exact start/end, empty-match advance one code point (:390-412). Lua instruction list (:416-441).
- `MetalCaptureProgram(kind, operand_start, operand_count, flags)`; ops: capture equality, text value eval, string-set, kind-set, substring, parent, ancestor, same-line; ALL/ANY, empty-input rules, negative aggregate selection (:445-470).
- `MetalCandidate(local_emission_ordinal, root, match_start, match_end, capture_head, capture_count, assertion_program, output_program, flags)`; ordinal incremented before each candidate write and independent of retention (:474-494).
- Structural candidate reduction for TS-query Programs before predicates/directives; ordered capture containment; distinct zero-width identities stay distinct; nested-root exception remains; "No new ordering exception is permitted." (:498-506)
- `MetalOutputProgram(instruction_start, instruction_count, maximum_property_count, maximum_output_bytes)`; seven directive kinds in source order (:510-530). `MetalOutputState` (:536-558).
- `MetalResult(batch_ordinal, local_emission_ordinal, file, program, root, match_start, match_end, capture_start/count, match_property_start/count, dynamic_property_start/count, output_byte_start/count)`; capture records: node, label, span, text handle, properties; task writes only its range; "No global append order is observable"; Rust sorts by stable ordinals, never evaluates (:562-588).
- `MetalError(code, detail, program_counter, task_ordinal)`; codes 1:1 with public `Error` (work limit, invalid opcode/mode/operand/target/state/anchor, workspace capacity, output capacity); first error only; Rust returns first by task ordinal and "discards all outputs when any task failed" (:592-618). [Contrast: production-grid doc :366-376 invalidates only the failed evaluation, not the whole batch.]
- Interpreter: no Program-specific source; JIT cannot be its fallback (:622-629). JIT: one MSL module per Program batch; private functions per Program structural + unique value/capture/output; batch entry selects Program once per task; cache key GPU registry ID, canonical Program identities, batch Program order, MSL options, backend version (:633-661).
- Host submission: pack, allocate, encode into one command encoder, `dispatch_threads` exactly once, wait, read, reconstruct `Output`s; counter increments only at `dispatch_threads` (:665-679).
- Deletions (:683-698), verifier list (:702-718), Units 1-11 (:722-839), test matrix and metric `(1, 0)` (:843-878), final acceptance (:882-894).

---

## 11. TD/metal-admission-decision-spike.md (347 lines) — 2026-10-20

- Context: user accepted recommendation on 2026-10-20 (:6-7). Later cycle characterization: active-batch + terminal dedup alone did not terminate every program the old validator accepted; user approved upstream rejection 2026-10-21 (:10-13). Later advancing-GREEDY fixture passed validator but produced different capture rows under deferred admission; full terminal dedup doesn't remove extra candidate; hand-built, not a compiled failure; user approved the query GREEDY contract (:15-18).
- **Recommendation:** "Keep active state only. Generate bounded successor batches, then deduplicate those batches in bulk. Keep final candidate reduction as a separate operation. Do not retain every historical state merely to reproduce the current CPU admission rule." (:23-25) Two unresolved semantic requirements: early containment can delete a match native TS retains; raw candidate ordinals encode discarded paths (:27-30).
- Problem shape (:37-54): active state = instruction, node, cursor frame, repeat frame, ordered captures; exact merge prevents duplicate future work; capture containment is stronger and "needs proof that every continuation preserves the containment relation" — disproved; completed candidates vs active states; waiting until all finish accumulates duplicates; "A fixed batch capacity bounds one expansion, but it does not bound the complete evaluation"; logical order can travel as data with a later ordering stage.
- Three modes: Early (historical capture antichain + queued-state merging), Deferred (no admission), DeferredExact (Deferred + full terminal-key sort/unique + ordinal assignment) (:63-67). Terminal equality: root, instruction, node, cursor values, repeat values, ordered capture identities (:69). Limits for experiment: 250,000 instruction visits, 32,768 candidates, 16 MiB candidate payload, 250,000 final comparisons (:75) [D]. Debug CPU timings only (:83-84).
- **Counterexample (compiled):** source `# node`, query `(module [(comment) @x @y (comment) @x]+ @z)`; native returns `[x,y,z]` and `[x,z]`; deferred modes match native; CPU interpreter returned only `[x,y,z]` because before `@z`, `[x,y]` contains `[x]`, but after the continuation the equal-geometry comparison is `y` vs `z` and neither contains the other; "Early admission discarded the shorter prefix before the finalizer could distinguish the matches." "It invalidates the assumption that reproducing CPU pruning always preserves native query meaning." (:90-114)
- Ordinals retain discarded work: one empty candidate via two paths + one captured: Early ordinal 1, Deferred ordinal 2, full terminal dedup restores 1 (:118-121); does not generalize to containment; 8 catalog cases differ only in ordinals (:123-127); scope clarification: experiment calls `execute_roots` once for all roots whereas production calls per root and resets counter, so the 8 don't directly mean 8 persisted differences (:129-133).
- Separate compiler discrepancies [M]: `(module (_) @child)` over two comments — native 2 captures, Tempo 0, because wildcard compiler emits FLAGS excluding EXTRA and comments are extra (:137-140). `((comment) @item)+` under module: native splits at intervening statements; Tempo captures across statements because compiler emits `FOLLOWING` for repetition and interpreter enumerates later named siblings across gaps (:147-151). Nested optional-type: same multiset, different order (:152).
- Candidate growth (8 balanced diamonds) [M] (:159-163): Early 1/1/1; Deferred 256 raw / 256 after exact / 1 final; DeferredExact 256/1/1. "Final deduplication cannot recover the work or memory already spent" (:165). Synchronous active-equal-reduction model: 26 instruction visits, max 2 successor states, no history (:166-168). Unequal depths at depth 16: active reduction emits 17 candidates, 562 visits, max 27 successor states; Early emits 1; final exact dedup still needed (:170-173).
- Real amplification [M] (:179-188): `(comment)+ @comment.around` on CPython `Lib/_markupbase.py`: Early 281 raw candidates, 73 final results, 58,292 payload bytes; Deferred reached 23,225 candidates and 9,239,276 payload bytes before instruction-limit stop (>=82x candidates, 158x payload). Debug timings: Early 20,939 us; Deferred stopped after 100,982 us; DeferredExact stopped after 92,131 us; diamonds 118/270/226 us.
- Coverage tables [M] (:193-225): structural matrix 57 cases per mode — stops 2/3/3; full CPU equality 55/33/35; CPU diffs 0/21/19; diffs excluding ordinals 0/5/5; native ordered-capture equality 37/41/41; native diffs 18/13/13. Catalog: 335 patterns x 3 sources = 1,005 cases per mode (synthetic Python file + two CPython files) — stops 1/2/2; CPU equality 1,004/995/995; CPU diffs 0/8/8; excluding ordinals 0/0/0; native equality 751/751/751; native diffs 22/21/21; 232 (Early) / 233 (Deferred) cases without native comparison (host predicates/transforms QueryCursor doesn't implement). Deferred modes retain 12 native capture-multiset discrepancies + 1 order-only (:210).
- Operator-alternative pattern hits the instruction limit in all three modes on `_markupbase.py`; Early's structural stops are in quadratic final comparison work (:227-230).
- Validation: red commit `14efefeb2`; 137 tests passed, two ignored; a reducer test assertion overflowed on unsigned subtraction of process-wide allocation totals (async Metal buffer release) — "exact asynchronous release responsible ... remains untraced" (:264-269). Production runner still fails cross-wave regression with `UnsupportedFeature` at structural-seed boundary (:287-288).
- Decision boundary (:324-336): disproves "final-only dedup has modest growth" and "current CPU containment is always a correct native oracle"; production waves still need canonical frames, successor counts, scans, scatter, GPU-owned frontier lengths; must satisfy 100-launch and 2,000,000,000-byte limits; "The current reducer costs 11–17 launches per call, so repeated standalone calls do not establish an acceptable complete execution shape." [M]
- Follow-up corrections in phase3: unsafe CPU admission, capture ordering, repetition loop-back, wildcard flags, missing-node flags (:344-345).

---

## 12. TD/metal-closure-composition-decision.md (56 lines) — 2026-10-21

- Approvals: composed execution with strongly bound validation results and combined passes; isolated 200-launch experiment; then production hard limit 200; later temporary 600 with required return below 100; memory 2,000,000,000 bytes (:3-7).
- Observed failure [M] (:12-21): four-state regression completes ordering, range preparation, closure, continuation reconstruction, fresh readiness in 77 Metal launches with 8,404,992 peak tracked bytes; then normal successor execution under same guard is refused with `BackendLaunchLimit` before `REPEAT_END`; first 77 launches pass all 24 physical arrival permutations against a serial first-visit oracle.
- **Root cause:** "Each standalone operator owns validation, initialization, scans, boundary checks, and publication. Composition repeats checks over the same immutable input. Generic successor execution then performs its full preparation and admission pipeline. The failure concerns dispatch cost. The measured component chain does not exhaust GPU memory. Small capacity bounds alone do not remove these fixed dispatch stages." (:25-28)
- Proposed: composed execution unit owning stages: validate once and bind result to resident input/buffers/generation/command dependency; carry binding through ordering/range/closure/reconstruction, reject stale bindings; combine compatible init/count/publication passes, keep barriers where GPU results are consumed; preserve normal successor semantics at proved closure exit (no implicit `REPEAT_END`); publish new frontier only after checks, then fresh readiness (:30-36). Must not reset guard, introduce CPU fallback, or restrict accepted Programs (:41).
- Checkpoint `90a516d52` [M]: handoff 114/148/155 -> 100/138/145 launches at 1/128/4,096 tasks; at 100-limit only small handoff fit; 200-launch experiment completed every handoff; "No consistent latency improvement is established." (:43-46)

---

## 13. TD/metal-dynamic-generation-composition-proposal.md (91 lines)

- Status: bounded generation work implemented; phase-three prerequisite repair complete; must reach <100 launches later (:3-4). User directed the agent to resolve routine choices without repeated approval; formal promotion process was the agent's own over-application (:6-8).
- Prerequisite audit found omitted optional-query and wide-frame coverage under 600; "Private-array widening failed. Device scratch, canonical regions, bounded jobs, and shared scans provide the measured repair." (:10-12)
- Measured blocker at former 200 cap [M] (:24-44): red `b1301eb25` fails at 56 dispatches at unsupported dynamic-continuation boundary; red `f776ef2a0` (one task, two sibling nodes, no GREEDY loop; oracle = one result, raw ordinal 1): generation 1 -> 56 cumulative launches, 2 -> 100, 3 -> 144, 4 -> 188; generation 5 prep reaches 200 and is refused; 225 attempts, 200 encoded, 200 completed; peak 8,404,992 bytes; 0.79 s; no terminal candidate yet.
- **Root cause:** "The query prefix compiler stops after each instruction whenever the Program requires repetition admission. Each generation then repeats prefix preparation, value checks, exact reduction, control resolution, and readiness. The measured fixture adds 44 launches per generation after the initial generation." Closure composition doesn't help (no eligible GREEDY closure). "The component checks establish local correctness. They do not establish that their combined dispatch cost fits a complete evaluation." (:49-55)
- Proposal (:59-73): extend composition to a complete dynamic generation; permit proved bounded query regions instead of mandatory single-instruction query waves; immutable region proofs bounding instructions, edits, expansion; preserve admission boundaries at control joins, observable GREEDY transitions, predicate suspension points unless proof permits fusion; deterministic region work in registers; variable fan-out in shared count/scan/scatter/exact-reduction stages; generation owner retains producer buffers and checked bindings; no history table, CPU fallback, private unbounded interpreter, or budget reset.
- Completion evidence: 355 tests pass, 3 ignored (:77) [M]; requirements list (:83-89).

---

## 14. TD/metal-full-state-reduction-proposal.md (78 lines) — approved 2026-10-21

- Integration problem [M components, D sum] (:11-29): frame canonicalization component 11 launches (small forced-collision case); counted active reduction 12 launches; successor stages >=3; straight composition >=26 launches per frame-changing wave; four such waves >=104, exceeding the then-hard 100 limit. "The component costs are measured. The combined cost is an integration estimate." "It disproves straightforward repetition of the current components as the complete production algorithm."
- Approved boundary (:36-54): replace separately canonicalized frame identities with complete frame references (typed cursor/repeat descriptor in current live arena); equal refs sufficient for equality, different refs don't imply different values; full values decide equality together with task, instruction, node, sibling requirement, Program, continuation kind, ordered captures; compare cursor, repeat, capture entries in one parallel comparison domain; hashing only selects candidate pairs; explicit typed boundary, no silent reinterpretation; completed-candidate identity/containment separate; "Retain only frames referenced by live states or retained terminal candidates; never retain an admitted-state index."
- Implementation checkpoint: typed comparison and seed integration pass; actual successor execution, output parity, whole-evaluation budgets unproved (:74-78).

---

## 15. TD/metal-query-greedy-contract.md (54 lines) — 2026-10-21

- Decision: "Query-mode GREEDY exits must belong to capture-free terminal repetitions with controlled entry and exit. Construction rejects unsupported forms with `InvalidGreedyRegion`, before execution backend selection." Non-query GREEDY unchanged (:11-14).
- Eight properties (:16-25): (1) loop header follows a repetition bump and starts with an immediate sibling advance; (2) returning edge follows a bump of the same repetition, returns to header; (3) greedy exit closes the same repetition; (4) header entry only from preceding instruction and returning edge; (5) body entries from region, body edges inside region except returning edge; (6) no nesting/overlap, body cannot emit; (7) neither body nor reachable later continuation captures before termination; (8) no path from exit returns to retired header, including through preceding bump.
- Proof uses predecessor edges and reverse capture reachability; checks actual control flow, bounded visits; iterative control-flow component pass for indirect re-entry; "The earlier direct-predecessor check missed returns through the preceding bump." (:27-32)
- Establishes: former hand-built counterexample can't form a valid query program (later branch re-enters retired header and captures); production expectation changes to construction-time rejection (:36-39). Does not prove final dedup alone implements every accepted query (:41-43).

---

## 16. TD/metal-root-streaming-continuation-design.md (250 lines) — SUPERSEDED

- Status: superseded by metal-optimal-execution-shape.md; persistent-lane and two-tier models remain only as measured investigation history (:5-6).
- Objective: measure dispatch granularity on CPython 335-Program workload; no host task record per tuple; command stops when output page full, restart at first incomplete semantic boundary (:10-15).
- **Problem [M]** (:19-29): scheduler creates 82,899,054 `MetalTask` records for 2,079 files and 335 Programs, mapping to only 9,632,501 distinct (file, root) pairs; 8.61 Programs per pair on average; task work distribution: 41.19% use 1-4 work units, 46.85% 5-16, 10.41% 17-64, 1.55% >64. "The current scheduler repeats root fetch, lane initialization, quota reset, and atomic scheduling for each Program." [Contradiction: 82,899,054 here vs 82,908,643 tuples in optimal-shape :53/:86 and production-grid :23; difference 9,589, unexplained.]
- Command model: packed files/tapes, Programs, a Program-dispatch index per grammar kind, complex anchor filters, stable ordinal prefixes, one bounded output page, one continuation record; host partitions by ABI/scratch capacity, not by output-page capacity (:33-43).
- Root-streaming scheduler: one persistent lane pool claims node-ordinal ranges from a global cursor; per node: read kind once, read Program range for kind, apply anchor filters, execute each admitted Program against the same loaded root, emit with stable ordinals; wildcard Programs separate range (:49-62).
- Stable identity `(file ordinal, Program ordinal, root ordinal)`; records carry file/Program/root/local output ordinal; compaction sorts (:66-79).
- Output-page exhaustion = continuation event; root is restart boundary; failed reservation atomically lowers `first_incomplete_root`; lanes finish claimed roots; host discards records at/after boundary; restart there (:83-94). Semantic runtime error removes all records for that (file, Program) evaluation including earlier pages (:103-104). One root must fit an empty page else `root-output-capacity-exceeded` (:111-114).
- Continuation record fields and reasons (:120-139).
- **Two-tier lane pool [M]** (:143-150): "The current executor assigns the largest transient task capacity to every physical lane. This rule limits the executor to 64 lanes for practical workspace sizes." 4,096-lane spike with 256 records per arena: Metal time 97.79 s -> 13.72 s; logical workspace 2.69 GB -> 877 MB; fast tier completed 99.07% of tasks, 0.93% overflowed. Design: fast lanes + full-capacity lanes rerunning overflowed evaluations (:152-162).
- Invocation budget: <=34 primary input partitions, each fast + full = <=68 invocations; 32 more for output continuation (:166-171).
- Test-only flag `TEMPO_BENCHMARK_SKIP_NORMALIZED_RESULT_PUBLICATION=1` (:196-202).
- Why abandoned: superseded because a persistent-lane pool still serializes and replicates arenas (optimal-shape :32-33: "uses memory to compensate for the wrong execution shape").

---

## 17. TD/metal-production-grid-cutover-design.md (637 lines) — authoritative production design

- Header: same 600/2e9/<100/exact-active/containment-on-completed/no-history rules as optimal-shape (:4-8). Authoritative; corrects ambiguous parts of optimal-shape; spike = performance model; packed interpreter = semantic oracle (:12-16).
- **"The important correction"** (:20-31): spike allocates one count record per distinct root (9,632,501), not per tuple; corpus has 82,908,643 `(file, Program, root)` tuples but 9,632,501 distinct (file, root) pairs, 8.61 Programs per root; must not expand into 82.9M host records nor allocate one global count per tuple. "The failed prototypes made both mistakes. They changed the thread count but retained the old task-shaped data model."
- Spike proves (:35-42): broad 256-thread dispatch fast; hierarchical `uint4` scans fast; 16 compact frontier waves fast; exact multi-arena page boundaries fast; bounded scatter fast; 34 invocations viable. Does not prove (:44-51): complete semantics, tape upload time, semantic ordering across Programs, selector compression, dynamic capture-antichain semantics, predicate/transform costs.
- Failed prototypes [M] (:57-85): per-result atomic reservations — 25,618,909 direct tuples in 47.22 s vs count/scan/scatter 3.05 s, so "a broad kernel cannot use durable output reservation atomics"; per-tuple direct task indexes preserve parity but follow expanded model and 68 partitions; evaluation waves (one thread per evaluation over all its roots) serialize roots, 65,536-record scratch quotas; at 256 capacity lanes 25 partitions required 456 waves and 193.27 s; at 1,024 lanes scheduler offsets exceeded 32-bit ABI. "Evaluation waves are not a viable execution model. More scratch lanes cannot correct serialized semantic work."
- Disposition: keep evaluation-wave code as profiling evidence; don't extend; next starts with private grid contract and shared selectors (:89-93).
- Required shape (:97-109): one bounded semantic unit per thread; 256-thread groups; one exact signature per threadgroup; global buffers hold spans/blocks/frontiers/matches/pages, never per-tuple records; count/scan/boundary/scatter output; compact global frontiers; publication batches don't define partitions; <40 normal invocations; input resident.
- Root universe `RootRecord{file, node, semantic_root_ordinal}`, 9,632,501 entries (:115-127). Shared root selectors (contiguous range, kind posting, run list, sparse bit set, shared explicit list), interned across Programs and files with full-content equality (:131-144).
- `EvaluationSpan{evaluation, file, program, signature, class, route, selector, root_count, semantic_start}`, `BlockSpan{evaluation_span, execution_block, selector_offset, root_count, block_lane_start, semantic_start}`; ~681,725 production evaluations (:150-175). `ExecutionBlock{block_span_start, block_span_count, dispatch_signature, route, logical_count}`; <=256 tuple positions; ~324,000 blocks; ~760,000 block spans (:179-201).
- Two work orders: execution order by initial route + exact signature; semantic order by `semantic_start` for scans/page boundaries; "Physical allocation order never defines result order." (:205-213)
- SQL/execution boundary (:217-225): SQL selects candidate files, reads anchor postings, stores normalized results and serves result pages; does not execute instructions or expand tuple rows; returns compact references to posting sets. "Metal owns tree navigation, capture state, predicates, output, and continuation boundaries. This rule keeps one semantic runtime for all `Program` behavior."
- Stage compiler (:229-263): one `StructuralStage{instruction_start, instruction_count, terminal_pc, terminal_kind, terminal_mode, terminal_operand, flags}` per possible pc: deterministic instruction range + one terminal op (stop/emit; child/descendant/following expansion; fork/goto; repeat-state transition). Frontier state's pc selects next descriptor, "a mixed Program does not need a mixed-Program kernel"; route describes the next operation, not the Program; signatures are optimization metadata, not separate runtimes.
- Class 1 direct: one thread per root evaluates all direct Programs of the block's signature; registers; one `uint4` aggregate per block span (result, capture, property, byte counts); segmented threadgroup scan; generated predicates (equal, prefix, small regex, bounded Lua in registers; large automata -> predicate frontier) (:265-280).
- Class 2 staged structural: thread runs one stage's deterministic range in registers; terminal op selects next count/scatter kernel; expansion creates compact frontier; scatter reruns the stage with local offsets (:282-294).
- Class 3 predicates: compact candidate descriptors (semantic ordinal + compact captures) grouped by predicate signature, one thread per candidate; large regex/Lua use compact predicate frontiers, not per-thread max arenas; count+scan compaction (:296-308).
- Class 4 dynamic frontiers: seed kernel creates one compact state per active dynamic tuple; `FrontierState{semantic_ordinal, program, pc, node, start, capture_head, cursor_head, repeat_head, local_ordinal}`; wave = process (1 thread/state) -> exact next-state + candidate counts -> scan -> scatter next frontier + candidates -> reduce equal query-state segments; no inactive slots; buffers scale with active states (:310-340). Antichain reduction: sort/bucket by complete query-state key; segmented maximal-capture-antichain per equal key preserving multiplicity and ordered identity; hash selects, equality confirms (:342-351). Termination: GPU counter for next frontier length read via indirect dispatch; encode checked wave bound before submission, unused waves zero work; resubmit if work remains and advance generation (:353-362).
- Failures: `FailureRecord{evaluation, semantic ordinal, error, pc}`; reduce by evaluation keeping smallest failing ordinal; final count kernels suppress outputs of failed evaluations; compact buffer, no per-tuple status (:366-379).
- Count hierarchy: thread registers -> 256-thread scan -> one aggregate per block span -> hierarchical scan in semantic span order; `[result_count, capture_count, property_count, output_byte_count]`; ~760,000 spans x uint4 ~12.2 MB vs tuple-count buffer ~1.33 GB [D]; semantic scan overwrites counts in place; scatter reruns semantics so no second count buffer (:383-406).
- Exact page boundaries: scan block-span aggregates, find first span over capacity, rerun containing block with segmented scan to find first non-fitting tuple; `Continuation{semantic_span, block_lane, local_output_ordinal, frontier_generation}`; one tuple > empty page -> invalid-capacity error, no retry (:410-431).
- Scatter: reruns direct/bounded generated code; threadgroup-local offsets + scanned semantic base; no global reservation atomic; dynamic/predicate stages scatter from compact accepted-match descriptors; typed arenas: results, final captures, properties, transformed bytes (:435-449).
- Residency: one Metal execution input, not per 32/64-file publication batch; upload once: files+nodes, children+supertypes, source+line starts, Programs + signature tables, root universe + selectors, evaluation spans + execution blocks; publication transaction <=64 files independently; private Metal storage for large immutable buffers, bounded staging buffers released after copy; report private Metal bytes and RSS separately; incremental builds same structures for changed files only, reusing resident Program/unchanged corpus buffers (:453-476).
- Dispatch plan (:480-498): classification+selector resolution; direct+bounded counts; dynamic seeds; <=16 frontier waves; candidate/predicate compaction; failure reduction; aggregate scans; page-boundary selection; scatter. <40 invocations; "The host does not wait between normal stages. All dependent kernels use buffers and indirect dispatch inside one command buffer."
- Memory budget includes/excludes (:502-519): excludes 82.9M MetalTasks, 82.9M global counts, per-thread scratch, per-evaluation scratch, per-batch corpus copy.
- Integration: corpus execution coordinator above `WorkspaceApplyTask`; returns typed pages keyed by file and Program; boundaries kept independent (discovery/tape loading, execution input, output page continuation, SQLite transactions); no schema change (:523-537).
- ABI: packed result ABI stays v4; executor-private ephemeral `tempo-metal-grid-v1` for blocks/spans/selectors/frontiers; no v3 adapter; not persisted (:541-551).
- Steps 1-10 with gates (:555-619) including <16 MB count+scan metadata (Step 4) and <40 invocations (Step 9). Acceptance gates (:623-637): exactly 9,632,501 roots, exactly 82,908,643 tuples, no per-tuple record, <16 MB metadata, <40 invocations, no host wait between stages, no per-record atomic, exact continuation, CPU=Metal, canonical CPython hash unchanged, **Metal < 5 s, host prep+validation < 2 s**.

---

## 18. TD/metal-execution-cutover-plan.md (481 lines) — the controlling plan, with checkpoint log

### Scope (latest, :5-12)
"The user's latest direction limits the carry-forward scope to transducers and their execution machinery." Execution begins with compiled transducers, value/output programs, a tape view, selected roots/selectors; ends with complete typed results + status. Retain interpreter, JIT, Metal backends, codegen, and the v7 tape input adapter. Optimize dispatches per generation, single-file latency, corpus throughput. "SCM lowering, rule creation, anchor selection, SQL planning, virtual tables, and durable publication are outside this work."

### Status (:14-22)
Phases 1-4 complete within recorded scopes; phase 5 in progress, full-corpus acceptance incomplete. "The frozen full run on `24ef0c4de` clears both terminal classes, then refuses dispatch 601. Nineteen successor generations consume 380 dispatches under the unchanged 600-dispatch limit." [M] CPU capture-run operator addresses deep captured repetition (:20-21).

### Deleted scheduler root cause (:26-28)
"The deleted scheduler did not implement the agreed execution shape. Its last corpus run used 1,978,757 kernel launches and 25,000 command buffers. Correct output and acceptable buffer memory do not satisfy the architecture requirement." [M]

### Checkpoint numbers [M] (:30-82)
- Phase 2 accepted; two historical combined native crashes deferred; CPU JIT distant-call relocation defect proved/corrected (:36-39).
- Phase 3 prerequisite repair: 358 tests, 355 pass, 3 ignored (:42-44). Phase-4 components: 359 tests (:48); typed output 368 (:51); adjacency/MIME 375 (:54); capture text 384, combined optional/quantified-injection fixture passes in 581 launches after removing an unnecessary deferred-assertion generation (:57-58); query-regex 392 tests, finite structure in registers when predicate needs shared frontier: small batch 168 -> 28 launches (:61-62); capture operators 400 tests (:70); ordinary inline 414 tests; 14 malformed argument cases; largest production fixture 598 launches and 278,528,000 tracked Metal bytes; max logged allocation 342,720,512 bytes (:76-81).

### Accepted admission decision 2026-10-20 (:85-107)
- Production retains active states only, no index of previously admitted states; each bounded successor batch gets bulk exact dedup before further expansion; complete equality includes ordered capture identities and multiplicity; hashes never establish equality; final candidate dedup and capture reduction separate (:88-92).
- "This decision replaces intermediate capture-containment pruning with exact active-state deduplication. The spike disproved the continuation safety of the existing containment relation." (:94-95) Does not permit unrestricted generation + final dedup (:98).
- Raw-candidate ordinal contract: "Each root task numbers unique complete candidates in deterministic logical order before final filtering. Final sorting preserves these numbers, including gaps left by filtered candidates. Exact identity includes the complete terminal state, not only the published capture rows." (:101-104)
- Cycle contract: validation must reject nonprogressing cycles incl. cursor restoration cancelling navigation and branches bypassing progress (:109-114). GREEDY contract (:116-121). CPU-first bulk-closure experiment approved; GPU range-closure core, shared eligibility proof, complete-value membership, resident range preparation (:123-130).
- Architecture sources: `benchmark/metal-optimal-shape-spike/src/main.rs`, optimal-shape doc, production-grid doc (:132-136).
- Limits: temporary 600 per complete evaluation replaces approved 200 (400 proposed and superseded); later <100; normal target <40; spike used 34, matching not required (:138-143).

### Deletion-first contract (:145-164)
Delete incorrect architecture in first chunk; replacement is sole production path from first chunk; incomplete stages return explicit errors; temporary loss of workload support acceptable; "Compatibility dispatch, feature flags, dual schedulers, and old-runtime adapters are prohibited. Renaming or relocating an incorrect scheduler does not satisfy deletion." Keep CPU oracles, fixtures, spike, stashes, launch guard, memory ledger.

### Launch guard (:166-187)
Every direct, threadgroup, and indirect dispatch consumes one reservation before encoding; 601st attempt doesn't reach Metal; indirect dispatches count even with zero work; nested scopes share one budget across commands, ranges, retries, replay, output pages; new evaluation gets new budget; no restore on completion; no env override; over-budget command doesn't submit its prefix; blit-only uploads/readbacks don't count; 2e9-byte buffer limit separate.

### Required shape (:189-211)
Shared root universe, compressed selectors, semantic spans, signature blocks (<=256 tuples, one signature); broad submission by execution class; root count changes threadgroup count; direct/bounded generated register code; dynamic = compact global SoA frontiers; wave = count, scan, scatter, reduce equal full-key segments; "Active reduction uses exact complete query-state identity, ordered captures, and capture multiplicity. Capture-containment reduction applies to completed candidates, after their continuation can no longer change capture identity." Lua/NFA in shared predicate frontiers; typed output via exact counts/scans/pages/scatter; publication <=64 files/transaction; memory partitions share launch budget.

### Target structure and owners (:213-244)
`execute_production_corpus_pages` -> resident input / execution plan / guarded executor (direct+bounded register stages; dynamic frontier waves with segmented full-key reduction; shared predicate frontiers; counts, scans, failure reduction, typed page scatter) / publication consumer. Owner table with prohibited responsibilities (e.g. Typed output must not "Restart semantic execution for each output page"; Dynamic frontier must not do "Historical admission scans or metadata-only host phase loops").

### Chunks 1-5 (:246-391)
- Chunk 1 deletes named modules: `metal_grid_frontier_runtime.rs` (host phase scheduler), `metal_grid_coordinator.rs`, `metal_grid_admission_prepare.metal` (historical liveness scans and phase-advance kernels), `metal_grid_admission_compare.metal`, `metal_grid_admission.metal` (old admission index), `metal_grid_admission_oracle.metal`, `metal_grid_history_runtime.rs`, `metal_grid_transform_runtime.rs`, output capture/group runtimes, `metal_grid_failure_runtime.rs`; plus `encode_waves`, `encode_value_cost`, `encode_late_seed`, `checked_range`, `bounded_execution_pages`, "adaptive metadata wave batches and separate comparison, prefix, capture, and value-cost scheduling" (:259-281).
- Chunk 2 resolution: "bounded means statically bounded work"; spike bounded kernel does four navigation steps per root thread; "A finite instruction sequence does not prove a bound on child fan-out. Do not run variable expansion inside a private root interpreter to avoid frontier states." (:306-313) Test: increase independent roots 100x with constant launch counts (:321).
- Chunk 3 tests: 1/128/4,096 equal-depth segments with fixed stage count; long ancestor histories without host loop proportional to history (:341-342).
- Chunk 4: retain optional-body and regex-injection regressions "that previously lost 879 matches" (:363); no host loop per candidate/capture/history link/NFA instruction; tiny-page vs normal-page; continuation without semantic replay.
- Chunk 5: exact normalized CPU parity across all six persisted evaluation and result tables (:383).

### Later checkpoints [M] (:394-481)
- Complete-generation blocker: 200 launches exhausted before any terminal, one task two siblings (:407-412). Temporary 600 cap: routing diagnostic reaches terminal barrier in 364 launches (:418).
- Phase-3 completion 2026-10-21: convergence 410/488/600 launches, closure 405/430/595 at 1/128/4,096 tasks; 312-test selection (:424-430) — later superseded by corrected acceptance.
- Corrected phase-3 acceptance 2026-10-22: five-match optional query 234 launches; captured repetitions at 2/33/130 siblings use 13/18/20 launches; max measured production count 596 (:441-442).
- Phase-4 completion 2026-10-23: "Dynamic seeds now exclude direct tuples. Successor storage follows completed GPU job counts and stable input/variant offsets." 743 native tests, 721 pass, 22 ignored; transducer selection 570 tests; five-file differential agrees with all 109,114 CPU matches; completes in 557 launches with 387,776,512 tracked Metal bytes peak (:449-464).
- 2026-10-25 successor/readiness fusion: saved 350-interpolation dispatch failure closes, returns every value in 599 dispatches; 796 normal native tests pass (:473-475). Remaining: frozen CPython workload exact parity, compile-separated single-file latency, throughput, v7 input binding, <100 dispatches (:478-481).
