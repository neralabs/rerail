# The transducer IR as it exists, and where it should sit relative to SQL

Sources read:

- **Tempo (Landing):** `~/orca/workspaces/nera-landing-2/ner-1562-scale-test-the-re-railer/tempo`, branch `jbarnes/ner-1562-scale-test-the-re-railer`, tip `e95613625`. Docs read: `transducer-design.md`, `transducer-value-program-design.md`, `sql-driven-transducer-execution.md`, `metal-batch-executor-design.md`, `metal-optimal-execution-shape.md`, `metal-execution-cutover-{plan,handover,phase5}.md`, `cpython-335-seconds-workplan.md`, `ner-1562-full-corpus-production-index.md`, `rerail-observable-requirements.md`, `transducer-native-jit-design.md`. Code read: `native/src/policy_dsl/{tape.rs, transducer.rs, transducer/packed_batch.rs, transducer/metal_batch_abi.rs, transducer/metal_output_pages.rs, transducer/metal_typed_output.rs}`.
- **Rerail N1562:** `~/orca/workspaces/rerail/ner-1562-scale-test-the-re-railer`, worktree HEAD `c674f4b6`, one commit past `8b6576f0`. Docs read: `transducer-design.md`, `packed-executor-architecture.md`, `ner-1562-landing-pad.md`, `rerail-pattern-lowering-design.md`. Code read: `native/src/code_rule_dsl/{packed.rs, packed/**, transducer.rs, transducer/sql_backend.rs, transducer/semantic_facts.rs, query/relational.rs}`.

## 0. Bottom line

1. **What the IR is.** A verified, fixed-width (one 32-bit word per instruction) structural automaton. It is a path-enumerating tree walker over one document, starting from supplied root ordinals. Typed side programs (value, capture, output) handle text, regex, Lua and output directives. It has no registers, no arithmetic, no calls and no host callbacks. Four backends execute the same `Program`: a Rust interpreter, a CPU JIT, a Metal interpreter and a Metal JIT. Tempo's version is the newer and more complete one.
2. **Your suspicion is confirmed.** In N1562's Rerail branch, tree relations (`inside`, `has`, `precedes`, `follows`, `stopBy`) are lowered to **SQL over navigation-edge rows**:
   - The `Navigation` producer emits one row for every parent→child edge in every file. Each row comes from the 35-column `rerail_matches` union virtual table (`transducer/sql_backend.rs:1567-1612`).
   - `query/relational.rs` then lowers relations with a recursive closure in SQL (`lower_navigation`, `:2783`). It also uses a `Universe` terminal that emits one row per node.
   - The transducer itself is given only kind, regex and "navigation descriptor" programs.
   - ast-grep patterns do not lower to transducer words. They run as a separate Rust tree interpreter (`PackedPattern`, `packed.rs:1418-1460`) inside `xFilter`, on the CPU only.

   This is the row-per-node, closure-in-SQL design that ast-index-spike measured as a dead end. It is the opposite of what the IR was built for.
3. **What exceeds the IR, and why.** Almost all of the gap between ast-grep/Rerail rules and the IR comes from requirements the findings contract has now dropped:
   - first-match binding;
   - ordered alternatives;
   - three-valued work accounting;
   - exact capture order and multiplicity.

   The remaining real gaps are small and well defined: an existence/negation sub-program, the missing axes (previous sibling, child position), bounded counts, ast-grep pattern opcodes, utility calls, and side-table reads (section 5).
4. **Where Metal's cost comes from.** Its problems mostly come from reproducing exact Tree-sitter query semantics: capture-containment antichains, ordered output and dynamic quantifier frontiers. They are not inherent to tree walking. The decoded node table the executors need is fully regular (11 × u32 per node), but it is rebuilt from the irregular persisted tape for every batch.

## 1. IR specification (Tempo `transducer-design.md`; implemented in `native/src/policy_dsl/transducer.rs`)

### Input
- An immutable tape, optional source bytes, and an ordered set of starting nodes.
- Each node exposes:
  - kind;
  - incoming field;
  - flags (named, extra, missing, error);
  - byte span;
  - parent;
  - ordered children;
  - optional hidden supertypes (up to 8).
- Node identity is the tape ordinal.

### Control model
Each starting node begins an independent **path**. A path has:
- an instruction position;
- a current node;
- an ordered, append-only capture list;
- active repeat counters;
- cursor slots.

The rules:
- A test keeps or rejects the path.
- A navigation with several results forks one path per result.
- `FORK` defers a branch.
- Only `EMIT` produces output.
- Logical output order is depth-first path order. Parallel backends must reproduce it by sorting on stable (task ordinal, local ordinal).
- Cycles are allowed only if they provably progress, for example a loop over `NEXT` or `FOLLOWING`.

### Encoding
- One word per instruction: `opcode:u8 | mode:u8 | operand:u16`. There are no extension words, so a program has at most 65,536 instructions and 65,536 entries per table.
- Each program has local tables: strings, predicates, labels, and u64 constants.

### Structural opcodes

| Op | Meaning |
|---|---|
| `STOP` | End the path with no output |
| `KIND k` | Test the node's kind |
| `FIELD f` | Test the node's incoming field |
| `FLAGS req/forbid` | Require and forbid flags |
| `TEXT p` | Text predicate (later replaced by `TEXT_EVAL`) |
| `SIZE cmp v` | Compare the span length |
| `SUPERTYPE k` | Test a hidden supertype |
| `CHILD all\|named` | Fork once per direct child |
| `NEXT all\|named` | Move to the next sibling |
| `PARENT` | Move to the parent |
| `DESCEND all\|named` | Fork once per proper descendant, in preorder |
| `FOLLOWING all\|named` | Fork once per later sibling |
| `SAVE_CURSOR s` / `RESTORE_CURSOR s` | Save or restore a node identity |
| `CAPTURE l` | Append the current node under a label |
| `REPEAT_BEGIN` / `BUMP` / `END` | Repetition counters |
| `FORK policy t` | Policy 0 keeps every branch; policy 1 is greedy |
| `GOTO t` | Jump |
| `EMIT l` | Emit one record and end the path |

### Side programs (`transducer-value-program-design.md`, `metal-batch-executor-design.md`)
- **`ValueProgram`** works on text or captures and returns a Boolean.
  - Byte-level instructions: `BYTE_*`, `SPLIT`, `JUMP`, `CALL`, `RETURN` (inside value programs only), `DFA_*`, and Lua capture, backreference and balanced-match ops.
  - Regex lowers through `regex-automata` to a byte DFA.
- **`CaptureProgram`** handles capture predicates: `CAPTURE_EACH`, `CAPTURE_ZIP`, `CAPTURE_TEXT`, `CAPTURE_KIND`, `CAPTURE_PARENT`, `CAPTURE_LINE`, `TEXT_EQUAL`, `TEXT_CONTAINS`, `KIND_IN`, `AGGREGATE all|any`, `SELECT_POLARITY`.
  - Together these cover Tree-sitter `#eq?`, `#any-eq?`, `#match?`, `#has-ancestor?` and same-line tests.
- **`OutputProgram`** handles directives: `#set!`, `#offset!`, `#strip!`, `#select-adjacent!`, `#downcase!` and MIME. Each can drop the output.
- Structural words call these through `TEXT_EVAL`, `ASSERT_EVAL` and `EMIT output_id`.

### Output
An output record holds the start node, a label, and ordered captures (label, node, byte span). Programs that use repeat instructions apply Tree-sitter's capture-containment reduction.

In the packed ABI the output is typed arenas (`metal_batch_abi.rs:256-299`):
- `ResultRecord`: task ordinal, local ordinal, start, label, capture range, property range, dynamic range;
- `FinalCaptureRecord`: label, node, start, end, text range, property range, retained;
- `PropertyRecord`;
- a pooled byte arena;
- a layout table;
- `TaskStatus`.

Durable publication (`NormalizedRowSource`, `content_store.rs:815`) uses evaluations, matches, captures and properties. Tempo's handover now calls for "demand plus six typed output relations over tape v7". I found no document that lists the six exactly; the records above are the candidates.

### Batching
- `execute_batch` takes many programs and many tapes. Results are grouped by file, then program, then ordered outputs.
- Callers can supply root lists per (file, program). An empty list proves there are no candidates; an incomplete index must fall back to unanchored execution.
- On Metal, one invocation covers every (file, program, root) task. Work is packed as blocks of up to 256 tuples (`metal-optimal-execution-shape.md`, "Work representation"). Programs are grouped by execution signature. Output is placed by count, scan and scatter, without atomics.

### Determinism
Output order is fixed by (task, local) ordinals. Errors are written per task. A failed task publishes nothing, and an incomplete result is never reported as a negative.

### Versions compared
- Rerail N1562's `docs/transducer-design.md` is an older copy. It lacks `SUPERTYPE`, `FOLLOWING`, cursor slots, repeat counters and fork policies, and its `FORK` takes no policy.
- N1562's `packed/**` (packed ABI, CPU, regex, Lua, output, scheduler, Metal, `native_ir`) was ported from Landing's v3/v4 eras (`ner-1562-landing-pad.md`, "Source eras").
- **Tempo is the canonical, newer IR.** Its newer parts are value, capture and output programs on Metal, the grid and frontier execution shape, and typed output pages.

## 2. Expressivity

### What lowers to IR words today

| Feature | Lowers to words? | Notes |
|---|---|---|
| kind, field, flags, supertype | Yes | Direct opcodes |
| Node-text regex / equal / prefix / Lua | Yes | `TEXT_EVAL` with a DFA value program |
| Captures | Yes | Ordered and append-only |
| Repetition (`+`, `*`, `?`) | Yes | `REPEAT_*`, `FORK`, `GOTO` |
| Alternatives (`any` over positive patterns) | Yes | `FORK` |
| Child, field-child, descendant, parent, next, later sibling | Yes | `CHILD`, `DESCEND`, `PARENT`, `NEXT`, `FOLLOWING` |
| Capture equality (`$A == $A`) and kind sets | Yes, as a capture predicate | `ASSERT_EVAL` with `TEXT_EQUAL` over `CAPTURE_ZIP` |
| `#has-ancestor` / `#has-parent` | Yes | `CAPTURE_PARENT` loop |
| Output transforms (strip, offset, downcase, set) | Yes | `OutputProgram` |
| ast-grep patterns | **No** | Tempo: exact Smart root-terminal patterns only (`transducer-design.md`, "Implemented ast-grep comparison"). Rerail: a separate `PackedPattern` tree interpreted in Rust, CPU-only, with no Metal or JIT version. The proposed `P_*` instruction set is not implemented (`rerail-pattern-lowering-design.md:382-480`, `:695-703`) |
| `all` (conjunction at one node) | Partly | Sequence of tests plus `SAVE_CURSOR`/`RESTORE_CURSOR`. But a sub-walk that matches several times forks duplicate paths; there is no "exists" barrier |
| `not` | **No** | There is no rule-level negation (`transducer-design.md:241`) |
| `inside` / `has` with `stopBy: end` | Expressible | A `PARENT`/`GOTO` loop and `DESCEND`. They would be filters, but without an exists barrier they multiply paths |
| `stopBy: neighbor` | Expressible | `CHILD`/`PARENT` without a loop |
| `stopBy: <rule>` | **No** | Needs negation ("stop at the first node matching R") |
| `follows` | Expressible | `NEXT`/`FOLLOWING` from the other node |
| `precedes` | **No** | There is no previous-sibling or preceding axis |
| `nthChild`, An+B, `ofRule`, `reverse` | **No** | There is no child-position instruction |
| `count` (relation counts, minimum/maximum/exact, offset) | **No** | There is no aggregation over sub-paths |
| Utilities / `matches` | Only by inlining | There is no `CALL`/`RETURN` in structural words. Recursive utilities can't lower |
| `transform` | Partly | Text transforms via `OutputProgram`; others no |
| `resolves`, `binds`, `calls`, `value`, `arguments`, `parse`, `layout` | **No** | They need side tables. In N1562 they are `SemanticFact`/`ValueFact`/`ParseFact` producers run on the CPU in `xFilter`, with composition in SQL |
| `embedded` / `host` | No | The tape model is one document per file entry. Derived documents would have to be separate file entries in the batch, plus a document map |

### Fundamental limits versus work not yet done
Nothing in the table is fundamental to a tree-walking automaton. The missing pieces are:

- **An existence/negation sub-program.** Run a sub-walk from the current node, keep only accept or reject, and add no paths or captures. This one primitive gives `not`, `has`/`inside` as filters, `stopBy: rule`, and anti-joins along tree axes.
- **A previous-sibling or preceding axis, and a child-position read.**
- **Bounded `COUNT`** of accepting sub-walks along an axis, with a comparison.
- **Pattern opcodes:** Smart terminals, ellipsis, multi-capture.
- **Bounded subroutine calls** for utilities.
- **Typed side-table reads** keyed by node, for scope, binding and reference facts produced upstream.

Tempo's design deliberately left these out because its source language, SCM (Tree-sitter's query language), is a positive pattern language.

### Rerail's own extension design is shaped by exact parity
N1562's `packed-executor-architecture.md` ("Target Program design for Rerail", "Execution transaction model") proposes:
- a `RuleProgram` with environment checkpoints;
- first-branch commit for `Any`;
- first-accepted-candidate binding for relations;
- three-valued unknowns;
- exact work charges.

Under `findings-contract.md` most of that machinery is unnecessary:
- `Any` becomes a union.
- A relation becomes an existential check.
- Bindings matter only where a later equality reads them, which is a join on the captured value.
- Budgets become an `Unknown` status per task.

### Relational operations the IR can host
These are all per document and anchored at a node:
- correlated semi-joins and anti-joins along tree axes (has, inside, not);
- transitive closure along tree axes (ancestor and descendant are native);
- bounded counts along axes;
- capture-value equality within one match.

These are possible but expensive on Metal: within-document joins between two independent matches on a value (a hash table per document), and grouped counts over a whole file.

These belong above the IR, in SQL:
- cross-file joins;
- repository-level counts;
- name, import, hierarchy and reachability graphs;
- allowlists;
- priorities between rules;
- consolidating the findings set.

## 3. Execution: numbers and limits

### CPU
- **Root-terminal patterns only** (8 Python identifier patterns, 5,725 files, 95.7 MB):

  | Path | Median |
  |---|---:|
  | Transducer | 359 ms |
  | ast-grep on tapes | 818 ms |
  | ast-grep on Tree-sitter trees | 12,609 ms |

  Parsing took 8.65 s and tape encode/restore 4.57 s (`transducer-design.md:255-291`).
- **128-comment quantifier stress:** interpreter 37.6 ms, Cranelift JIT 34.6 ms.
- **The Cranelift JIT was later declared "not a JIT for transducer semantics"**, since it was a dispatcher calling Rust, and slated for deletion (`transducer-native-jit-design.md`, "Status"). It is replaced by one shared native IR (`native_ir.rs`) that emits Cranelift code for the CPU and MSL for Metal.

### Metal
- **CPython, 335 programs** (2,079 files, 82.9M tasks, 37.4M outputs, 85.1M normalized rows):
  - 219–275 s end to end;
  - 100.4 s of Metal execution across 68 commands;
  - 100.7 s of SQLite publication (`cpython-335-seconds-workplan.md`).
- **The deleted scheduler** used 1,978,757 kernel launches and 25,000 command buffers.
- **Shape spike:** full production volumes in 34 invocations and 40 ms. The integrated coverage kernel took 106.9 ms. Tuples split 64% direct, 29% bounded, 7% dynamic frontier (`metal-optimal-execution-shape.md`).
- **Pinned 335-pattern differential:**
  - 109,114 matches, equal to the CPU result;
  - 395–451 dispatches;
  - 75–200 s;
  - 142 MB of tracked buffers.
- **Full corpus:** the run on `95b59a9d0` completed 584 dispatches with 1.91 GB tracked. It then failed the terminal-containment comparison guard after 732.6 s; the reduced repro needed 21.7M comparison pairs. The terminal-anchor fix (`e95613625`) completes the reduced case in 72 dispatches, but the full corpus is still unproved (`metal-execution-cutover-phase5.md:1135-1173`).
- **The guards** (600 dispatches, 2 GB of buffers, 4 GiB application RSS) are self-imposed acceptance limits, not hardware limits. The machine's recommended working set is 115 GB.

### Inherent versus implementation costs
- **Implementation and semantics-driven:**
  - capture-containment antichain reduction;
  - ordered output and the ordinal bookkeeping behind it;
  - dynamic quantifier frontiers;
  - per-task arenas;
  - the dispatch count per generation;
  - SQLite publication of 85M rows.
- **Inherent to the IR:** a path-enumeration semantics with no aggregation or existence barrier. Every intermediate path materializes and every successful path is an output, so `has`-style filters and counts multiply work. The existence and count primitives in section 2 fix this.

## 4. Relationship to the tape
- **Tempo's tape is v4 (`TMPTAPE4`, 6-byte records, `policy_dsl/tape.rs:16-24`).** It reuses the final header table for supertypes. Rerail's v4 used the same slot for Blade error ranges, so the two formats are incompatible under one magic number. N1562 forbids reading Landing tape bytes (`ner-1562-landing-pad.md`, "Tape boundary").
- **The executors do not read the persisted tape.** The packed batch decodes each file through `TapeDoc` accessors into a **fully regular** node table (`packed_batch.rs:199-283`):
  - per node: `parent, next_any, next_named, child_start, child_count, descendant_end, source_start, source_end, kind, field, flags`, which is 11 × u32 = 44 B;
  - a child array;
  - line starts, found by scanning the source;
  - supertype entries.

  This table is the input both GPU and CPU use.
- **So the IR depends on regular, fixed-stride, random-access node records, not on the persisted encoding.** The cost is a decode and repack for every batch, about 44+ B per node of host and GPU memory. This feeds directly into the 2 GB buffer guard. A tape whose persisted form is already that regular table, or can be sliced into it with no per-node work, removes both the cost and the irregularity.
- **Derived documents (v7 `FileImage`: Blade/PHP, Vue script, embedded programs) are not modeled.** Each derived document must be its own batch "file" with its own node table, plus a source-map relation for locations.

## 5. Recommendation: the transducer as the concrete IR driver

### Boundary
**Push into the transducer** everything that can be decided per document starting from a root node, by walking tree axes and reading node text or per-node side tables:
- ast-grep patterns (full `PatternNode` lowering);
- kind, regex and text tests;
- `all`, `any`, `not`;
- `inside`, `has`, `precedes`, `follows` with every `stopBy` mode;
- `field`, `nthChild`;
- non-recursive utilities;
- capture constraints and equality;
- relation counts;
- text transforms;
- line and suppression reads;
- semantic tests that read prepared per-node side tables.

**SQL stays above** and owns:
- candidate selection from statistics-backed indexes;
- demand and batch formation;
- cross-file and repository relations (projections, symbol references, imports, hierarchy, reachability);
- counts per file and per repository;
- allowlists;
- priority and exclusion between rules;
- findings consolidation;
- incremental invalidation.

### Interface (fits `architecture.md` principles 1–6 and Tempo's `sql-driven-transducer-execution.md`)
- **In:** a demand relation, `demand(program_id, doc_id, roots)`.
  - `roots` is an interned root list, or "all".
  - One `xFilter` per batch, using `sqlite3_vtab_in`.
  - SQL builds the demand from FTS statistics and the anchor plan.
- **Out:** narrow, typed relations, each its own virtual table, or materialized once per batch:
  - `t_match(program_id, doc_id, root, key_node, line)`;
  - `t_capture(program_id, doc_id, root, label, node, start, end)`, produced only when the consumer needs captures (otherwise payloads come from rehydration);
  - `t_status(program_id, doc_id, complete, reason)`.

  Do not use one 35-column union table with `row_kind`. Never emit navigation-edge or universe rows.
- **Program identity:** canonical digest × grammar identity × IR version. Programs are cached in the plan cache.

### IR extensions needed, in order
1. **Set semantics mode.** No containment reduction, ordering or multiplicity requirement. Output is keyed by (program, doc, root) plus key node. This removes the containment-pair blow-up and the ordinal bookkeeping.
2. **`EXISTS` / `NOT_EXISTS sub`.** An isolated sub-walk that returns accept or reject, short-circuits on the first accept, and leaks no paths or captures. This gives `not`, filter-style `has`/`inside`, `stopBy: rule` and tree anti-joins.
3. **Axes:** `PREV`, `PRECEDING`, `ANCESTOR`. Add `CHILD_INDEX` and `CHILD_COUNT` reads (named or all, forward or reverse) for `nthChild` and An+B.
4. **`COUNT sub axis cmp`.** A bounded count along an axis, so it is also a work bound.
5. **Pattern opcodes** from `rerail-pattern-lowering-design.md` (`P_REQUIRE_KIND`, `P_MATCH_TERMINAL`, `P_BIND_SINGLE`, `P_ENTER_CHILDREN`, `P_NEXT_CHILD`, `P_END_CHILDREN_SMART`, `P_ELLIPSIS`, …), merged into the structural word stream. They replace the CPU-only `PackedPattern` interpreter.
6. **`CALL` / `RETURN`** for utilities, with a verifier-computed depth bound. Alternatively, inline deduplicated by digest. Recursive utilities are rejected or bounded.
7. **`SIDE k`:** a typed read of a per-node side table (scope id, binding id, reference target), produced by an upstream producer relation. This is how `resolves` and `binds` enter without callbacks.
8. **Documents:** derived documents as batch files, plus a `doc_map` relation.
9. **Status:** budgets produce `Unknown` per task. Work accounting is a bound, not a semantic.

### Deletions this implies
- N1562's `Navigation` and `Universe` producers, the recursive-SQL lowering of relations in `query/relational.rs`, and the union `rerail_matches` table. Keep them only as tests or as the pure-SQL fallback binding.
- The CPU-only `PackedPattern` route, once extension 5 lands.
- DLRE's re-encoding of the syntax tree as SQL tables (`authored-sql.md`, category C).
- SSC's grammar→relations SQL pattern compiler as an executor. Keep it as the reference pure-SQL lowering of the same IR for the SQLite-only fallback.

### Sources to merge or port
- **Tempo:** `transducer.rs`, the value, capture and output programs, the verifier, `native_ir.rs` (shared CPU/MSL lowering), the packed ABI v4, and the grid, count, scan and scatter Metal shape. Port them onto v7 `TapeDoc` accessors. Keep SCM only as a test frontend; the 335-pattern corpus stays a parity and throughput benchmark.
- **N1562:** its ported `packed/**`, its pattern-lowering design, and its semantic-fact descriptors, which become the `SIDE` producers.
- **Drop:** the parts of N1562's `RuleProgram` transaction model that only serve exact parity.

### Risks
- **Metal divergence.** Rule-heavy programs with `EXISTS`, `COUNT` and pattern ellipses land in the dynamic-frontier class (Class 4). These need bounded per-thread stacks and a separate kernel class.
- **Per-edit latency.** Launch count per generation still dominates for a single file, so fused or persistent kernels are needed (README "GPU").
- **Rehydration correctness.** Payloads rebuilt by the native matcher must stay consistent with the keys the transducer emitted. A differential test is required.
