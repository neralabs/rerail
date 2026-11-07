# Transducer IR: normative specification

Status: draft for review. This document is normative for the transducer IR: what a program is, every operation and its semantics, the side programs (value, text, capture, output), how every rule-language construct lowers, the verifier, the output ABI and the backend obligations. `spec.md` §6 summarises it. Fusion, which combines programs into groups, is specified in `spec-fusion.md`. Tooling (dumps, assembler, tracing, profiling) is in `spec-tooling.md`.

Sections 1–3 (machine model, operations, group tables) were moved here from `spec-fusion.md` with their numbering unchanged, so references of the form "§2.6" in either document mean this one.

Sources:
- Tempo's IR, `tempo/native/src/policy_dsl/transducer.rs` at `e95613625` (opcodes `:184-208`, interpreter `:2099-2300`), value programs in `transducer/value.rs`, Metal kernels in `transducer/metal_*.metal`;
- `research/transducer-ir.md` (expressivity and gaps), `research/sql-vs-transducer.md` (boundary), `research/metal-recursion.md` and `research/metal-enqueue.md` (Metal execution), `research/fusion.md` and `research/fusion-model.md` (measurements);
- the rule language model: `native/src/code_rule_dsl/matcher/mod.rs:254` (`AstMatcher`), `matcher/semantic.rs` (`Semantic`), `model.rs:545` (`Query`, `RegexOptions`, `RegexLines`).

---

## 0. The lowering principle

The transducer is kept **simple**: a small, verifiable instruction set that runs identically on the interpreter, the JIT and Metal. It is not required to express every single-file condition. Each condition goes to one of three places:

| Place | What goes there | Why |
|---|---|---|
| **Transducer program** | Local structure and text: kind, field, pattern, node-text tests, `all`/`any`/`not`, the tree relations with every `stopBy` mode, `nthChild`, bounded counts, non-recursive utilities, monotone single-axis recursion, capture equality within one match, reads of fact columns | Bounded state per path, Metal-friendly, fusable across rules |
| **Named derived fact (SQL)** | Intermediate relations that several rules need, or that are naturally relational: scopes and qualnames, bindings and references, intra-file call graphs, class hierarchies, data flow and other fixpoints over names, joins between independently enumerated sets, aggregates | Computed once and cached, shared across rules, joinable, with statistics for the planner; keeps fixpoint and join machinery out of the transducer |
| **Native fact producer** | Base facts that need a resolver or evaluator (scope binding, literal values, format strings, parsed literals) | Reviewed engine code behind a relation contract |

Rules of thumb:

1. **Local and navigational → transducer.** If a condition is a walk from a node along tree axes with bounded state, it is a transducer program.
2. **Shared or relational → named derived fact.** If two or more rules need the same intermediate relation, or the condition is a join between sets that neither side reaches by navigation, or a fixpoint over names, it is a named derived fact defined separately from the rules (`spec.md` §21) and cached in SQL. Rules and transducer programs join on it. A rule-local join of that kind uses lowering rule L5 (§6.5).
3. **A new transducer operation must earn its place.** It is added only when it is local, cheap on every backend, used by several rules, and not better served by a derived fact. The default answer to "the transducer cannot express this" is a derived fact, not a new opcode.
4. **Facts flow both ways.** Transducer programs produce base facts (exported captures, per-node columns). Derived facts are consumed by transducer programs as fact columns (`SIDE_*`) or demand seeds, or joined in SQL.
5. **Code conditions are still written as AST conditions** where the grammar exposes the structure (§5.1). That is about the source being parsed, not about forcing everything into one IR.
6. **No SQL in rules** (decision 0002). Derived facts are defined separately from rules, named and versioned, and rules reference them by name.
7. **Shared tree-walking analyses are named fact programs** (annotation atoms, receiver chains, binding sites): cached and computed once, but still transducer programs (`spec.md` §21.1).

The boundary test: put the condition where it is simplest to compute once and share. A transducer program should read like a structural pattern; anything that reads like a query belongs in a derived fact.

---

## 1. Machine model in set mode

### 1.1 Documents

A document `D` is one v9np tape (`spec.md` §3) with `n` nodes in preorder, `0..n`. The executor exposes these columns for each node `i`:

| Name | Definition | Source |
|---|---|---|
| `kind(i)` | folded kind code (10 bits; carries named, extra, missing, error and alias distinctions) | `kf` |
| `field(i)` | incoming field code (6 bits; 0 = none) | `kf` |
| `flags(i)` | nibble `named \| extra<<1 \| error<<2 \| missing<<3`, derived from `kind(i)` through the grammar's code table | grammar table |
| `size(i)` | subtree size; the subtree is `[i, i+size(i))` | `size` |
| `end_pre(i)` | `i + size(i)`, the preorder successor that skips the subtree | derived |
| `start(i)`, `end(i)` | byte span (`spec.md` §3 formulas) | `base`, `off`, `tail` |
| `par(i)` | parent, `NONE` for the root | resident sidecar, derived once per tape in a stack pass (`spec.md` §20.6) |
| `cidx(i)`, `cidx_named(i)` | child ordinal among all, and among named, siblings | resident sidecar, derived once per tape on first demand (`NTH`) |
| `side_c(i)` | per-node fact column `c` (`SIDE`) | fact producers (`spec.md` §9, `fact_*`) |

Batches reference resident documents through a batch document table (`spec.md` §20.5); nothing is concatenated. Every node reference is a (document, preorder) pair. No operation crosses a document boundary except the document switch `DOC` (§2.12), which moves only between a file's own documents.

### 1.2 Programs, groups and labels

- **Program group `G`.** The unit of fusion and dispatch. It has an instruction vector `W[0..m)` of 32-bit words with `m ≤ 65,536`, a group header (§3), and local tables.
- **Label `ℓ`.** Identifies one rule fragment's output inside a group. Its label-table entry (§3.2) maps it to (rule, fragment), its kind (`KEY`, `EXPORT`, `COUNT`) and, for exports, a schema.
- **Task.** A triple `(G, D, r)`, where `r` is a root node of `D` supplied by demand or by the group dispatch table (§3.1).

### 1.3 Paths

A **path** is a tuple `π = (pc, x, C, K, Q, σ)`:
- `pc` is the instruction position;
- `x` is the current node;
- `C` is the capture list, a sequence of (capture label, node);
- `K` holds the cursor slots;
- `Q` holds the repeat counters;
- `σ` is the sub-program context stack (§2.3), empty at top level.

A test instruction maps a path to itself or to nothing. A navigation instruction maps a path to a finite set of paths, one per result node. `FORK t` maps π to {π with `pc+1`, π with `pc = t`}.

### 1.4 Output set

`Acc(G, D, r)` is the set of terminal events `(ℓ, π)` produced by `EMIT ℓ` on some finite run from the initial path `(entry(G, kind(r)), r, [], ∅, ∅, [])`. The run is nondeterministic, choosing among the paths produced at every navigation and fork.

The task output is a set:

```
Out(G, D, r) = { (ℓ, D, r)            | ℓ ∈ KEY labels,    ∃π: (ℓ, π) ∈ Acc }
             ∪ { (ℓ, D, r, e(ℓ, π))   | ℓ ∈ EXPORT labels, (ℓ, π) ∈ Acc }      -- distinct tuples
             ∪ { (ℓ, D, r, g, |{...}|) | ℓ ∈ COUNT labels }                     -- per §2.5 / §4.6
```

Here `e(ℓ, π)` is the label's export projection: the typed values of declared captures in `C`.

The batch output is the union over tasks. It does not depend on:
- the order of paths, or the order in which tasks are scheduled;
- how many runs reach an `EMIT`;
- which run is found first.

**Executor freedom.** Because `Out` is a set, an executor may:
1. stop exploring a task's paths for a `KEY` label once that label has been emitted in that task;
2. drop any path whose reachable-label set (§3.3) is contained in the labels already fired in the task;
3. explore paths in any order, interleave tasks arbitrarily, and enumerate navigation results lazily (§2.7).

An `EXPORT` label marked `ALL` (§3.2) must enumerate every distinct export tuple, so freedoms 1 and 2 do not apply to it. Duplicate export tuples may be emitted, and consolidation removes them.

### 1.5 Completeness

Each task ends either `Complete`, when every path terminated, or `Unknown(reason)`. Reasons are: `resource` (an arena, the step limit or the work limit), `verifier`, or `fault`.

- An `Unknown` task's outputs may be partial. They are kept, but the (label, document) outcome is `Unknown` for every label of the task that had not been proved emitted. Consolidation treats them per `findings-contract.md` §2.
- A `KEY` label that was emitted is a confirmed finding even if the task later becomes `Unknown`, because soundness is per key. An `EXPORT ALL` label in an `Unknown` task is `Unknown`.

### 1.6 Determinism

For fixed (G, D, demand), `Out` is a function, and every backend produces the same set. Fused and unfused execution produce the same set (§4). This is the differential-testing invariant (§7).

---

## 2. Operations

### 2.0 Word encoding

The word is `opcode | mode << 8 | operand << 16`, as in Tempo's `word()` at `transducer.rs:1111`. Existing opcodes keep their codes, listed at `transducer.rs:184-208`:

```
0x00 STOP  0x01 KIND  0x02 FIELD  0x03 FLAGS  0x04 TEXT  0x05 SIZE  0x06 ASSERT  0x07 FIRST
0x08 LAST  0x09 ABSENT_FIELD  0x0a SUPERTYPE  0x10 CHILD  0x11 NEXT  0x12 PARENT  0x13 DESCEND
0x14 FOLLOWING  0x15 SAVE_CURSOR  0x16 RESTORE_CURSOR  0x20 CAPTURE  0x21 REPEAT_BEGIN
0x22 REPEAT_BUMP  0x23 REPEAT_END  0x30 FORK  0x31 GOTO  0x40 EMIT
```

`FLAGS` keeps Tempo's semantics: `mode` holds the required values and `operand` the mask over the nibble `named | extra<<1 | error<<2 | missing<<3` (`:2114-2120`).

New opcodes are allocated from free space. The ranges are: tests `0x0b–0x0f` (`NTH`, `TEXT_ID`, `TEXT_DFA`, `TEXT_SET`, `TEXT_ID_EQ`), navigation `0x17–0x1f`, sub-programs and calls `0x32–0x3f`, set stage `0x50–0x57`, documents and side facts `0x58–0x5f`, pattern words `0x60–0x6f`, line-document words `0x70–0x77` (§5.4). Codes not listed here are reserved, and the verifier rejects them.

**Set-mode restrictions.** `REPEAT_*` (Tree-sitter quantifiers) is admitted in set mode only with no capture-containment reduction. The capture-containment reduction itself is not part of set mode.

### 2.0a Base operations (Tempo)

Semantics from Tempo's interpreter (`transducer.rs:2105-2270`), restated over tape columns. "Fork" means one path per result, enumerated lazily (§2.7).

| Op | mode | operand | Semantics at node `x` |
|---|---|---|---|
| `STOP` | — | — | end the path, no output |
| `KIND` | — | folded kind | keep iff `kind(x) = k` |
| `FIELD` | — | field | keep iff `field(x) = f` (the incoming field) |
| `FLAGS` | required values | mask | keep iff `flags(x) & mask = mode & mask` |
| `TEXT` | — | value program | keep iff the value program accepts `text(x)` (§4.1) |
| `SIZE` | 0 `=`, 1 `≥`, 2 `≤`; bit7 long constant | constant or u64-table index | compare the byte length `end(x) − start(x)` |
| `ASSERT` | — | capture program | keep iff the capture program accepts the path's captures (§4.2) |
| `FIRST` / `LAST` | bit0 named | — | keep iff `x` is the first (last) child, or named child, of its parent; a root counts as first and last |
| `ABSENT_FIELD` | — | field | keep iff `x` has no child with field `f` |
| `SUPERTYPE` | — | supertype | keep iff `x` carries the hidden supertype (grammar side table) |
| `CHILD` | bit0 named | — | fork to each child (named if bit0) |
| `NEXT` | bit0 named | — | move to the next sibling (named if bit0), or end |
| `FOLLOWING` | bit0 named | — | fork to each later sibling |
| `PARENT` | — | — | move to `par(x)`, or end at the root |
| `DESCEND` | bit0 named | — | fork to each proper descendant in preorder |
| `SAVE_CURSOR` / `RESTORE_CURSOR` | — | slot | save `x` in slot; move to the saved node |
| `CAPTURE` | — | label | append `(label, x)` to `C` |
| `REPEAT_BEGIN` / `BUMP` / `END` | — | repeat id | quantifier bookkeeping; admitted in set mode only without containment reduction |
| `FORK` | 0 all, 1 greedy | target pc | continue at `pc+1` and at the target. In set mode the greedy policy has no observable effect and is treated as 0 |
| `GOTO` | — | target pc | jump |
| `EMIT` | — | label | §2.10 |

### 2.1 Navigation axes

| Opcode | Mnemonic | mode | operand | Semantics: results from node `x` |
|---|---|---|---|---|
| 0x17 | `ANCESTOR` | bit0 named | 0 | every proper ancestor `a` of `x` (named ones if bit0), nearest first. Verifier: none beyond the encoding |
| 0x1c | `PREV` | bit0 named | 0 | the nearest preceding sibling (named if bit0), or none |
| 0x1d | `PRECEDING` | bit0 named | 0 | every preceding sibling, nearest first |
| 0x18 | `DESCEND_KIND` | bit0 named | folded kind `k` | `{y ∈ (x, end_pre(x)) : kind(y) = k}`. Defined as `DESCEND; KIND k` |
| 0x19 | `CHILD_KIND` | bit0 named | `k` | `CHILD; KIND k` |
| 0x1a | `FOLLOWING_KIND` | bit0 named | `k` | `FOLLOWING; KIND k` |
| 0x1b | `ANCESTOR_KIND` | bit0 named | `k` | `ANCESTOR; KIND k` |
| 0x1e | `PRECEDING_KIND` | bit0 named | `k` | `PRECEDING; KIND k` |
| 0x1f | `CHILD_FIELD` | 0 | field `f` | every child `y` of `x` with `field(y) = f` |

- The `*_KIND` forms are superinstructions. Their semantics equal the two-instruction sequence, and a peephole pass emits them. The executor implements them as a contiguous scan of `kind` over the axis range, filtering before it forks.
- **Verifier:** `k` must be a valid folded code of the program's grammar, and `f` a valid field code. For mode, bit0 is the only defined bit.

### 2.2 Child position

| Opcode | Mnemonic | mode | operand | Semantics |
|---|---|---|---|---|
| 0x0b | `NTH` | bit0 named, bit1 from-end | index into the `nth` table | keep π iff the 1-based position `p` of `x` among its parent's (named, if bit0) children, counted from the end if bit1, satisfies `p = a·t + b` for some integer `t ≥ 0`. The entry is `{a: i16, b: i16}`; `a = 0` means `p = b` |

- This is CSS An+B as ast-grep's `parse_an_b` reads it. `nthChild.ofRule` is not `NTH`: it lowers to `COUNT` over `PRECEDING`/`FOLLOWING` with a sub-program (§2.5).
- **Verifier:** the table index is in range, and `b ≥ 1` when `a ≤ 0`.
- **Executor:** reads `cidx`/`cidx_named`. Those columns are resident sidecars, derived once per tape in one pass over each parent's child chain, when any program in the batch uses `NTH`. The from-end form uses `cidx` together with the parent's child count. The child count is computed by the same pass and stored in the parent's slot of a `nkids` column.

### 2.3 Sub-programs: `EXISTS`, `NOT_EXISTS`, `ACCEPT`

| Opcode | Mnemonic | mode | operand | Semantics |
|---|---|---|---|---|
| 0x32 | `EXISTS` | bit0 must be 0 | sub-program entry pc `s` | keep π (at `pc+1`) iff some run from `(s, x, C, K, ∅, σ·frame)` reaches `ACCEPT` |
| 0x33 | `NOT_EXISTS` | 0 | `s` | keep π iff no run from `s` reaches `ACCEPT`. If the sub-run ends `Unknown`, the outer path ends `Unknown` |
| 0x34 | `ACCEPT` | 0 | 0 | ends the current sub-program run with success. It is illegal at top level |

**Scoping rules.** These are what make `EXISTS` a barrier.
1. The sub-run starts with the outer path's captures `C` and cursors `K`, readable as a snapshot. Its own `CAPTURE`s are local and are discarded when it ends.
2. No capture, cursor or counter flows out. The outer path continues with its own `C`, `K` and `Q` unchanged.
3. `EMIT` inside a sub-program is illegal.
4. The sub-program is a contiguous pc region `[s, s_end)`, recorded in the sub-program table. No jump enters or leaves the region except through `EXISTS`/`NOT_EXISTS` entry and `ACCEPT`/`STOP` exit.

**Short circuit.** `EXISTS` returns on the first `ACCEPT`. `NOT_EXISTS` returns false on the first `ACCEPT`, and true only when every sub-path has ended. The sub-run's paths use lazy enumeration (§2.7), so a sub-run's frontier is bounded by its static nesting (§6.1).

**Verifier:**
- `s` begins a declared sub-program region;
- regions are properly nested;
- sub-program nesting depth is at most `MAX_SUB_DEPTH = 8`;
- a sub-program contains no `EMIT`;
- every `ACCEPT` lies inside a region.

**Metal.**
- **Shallow sub-programs** (nesting ≤ 2, with no `DESCEND`/`ANCESTOR`/`FOLLOWING`/`PRECEDING` inside) run inline in the thread. A sub-run is a nested loop with a cursor register per enumeration level.
- **Deep sub-programs**, or any sub-program containing an unbounded axis, are promoted by the planner to the set stage (§2.6). The outer test then becomes `IN_SET`.
- `EXISTS` therefore never needs a per-thread unbounded stack on Metal.

### 2.4 Utility calls

| Opcode | Mnemonic | mode | operand | Semantics |
|---|---|---|---|---|
| 0x36 | `CALL` | bit0 negate | utility id `u` | like `EXISTS` (or `NOT_EXISTS` if bit0), with entry `utilities[u].entry`. Formal arguments are bound to caller captures through `utilities[u].bindings` |
| 0x37 | `RETURN` | 0 | 0 | `ACCEPT` inside a utility region |

The utility table entry is:

```
{entry: u16, end: u16, params: u8, bindings: u16 (index into binding table),
 group: u16 (recursive group id or NONE), axis: u8 (progress axis for recursion), depth_bound: u8 (static, or 0xFF = data-bounded)}
```

**Recursion.**
- A utility may call itself, or another utility in its recursive group, only when the verifier proves **monotone single-direction progress**. Every recursive `CALL` in the group must be dominated by a navigation along the group's declared `axis`: descendant/child for `has` recursion, ancestor/parent for `inside`. The call must not be under `NOT_EXISTS`, `COUNT` or `CALL` with bit0 set.
- Recursion depth is then bounded by the document's tree depth, which is data-dependent but finite.
- **Execution of a recursive group.** It never uses per-thread recursion:
  - the planner lowers the whole group to a set-stage fixpoint (§2.6, `FIX`), evaluated as depth-level waves on Metal with GPU-driven continuation (C2);
  - the `CALL` sites then become `IN_SET`.
- **Non-recursive utilities** are either inlined at their call sites by the fusion algorithm (§4.5) or kept as `CALL` regions executed like `EXISTS`.

**Verifier:**
- the call graph is acyclic except for recursive groups that pass the progress check;
- bindings are in range;
- `depth_bound` is consistent: static for acyclic call graphs, `0xFF` only for groups lowered to `FIX`.

### 2.5 Counting

| Opcode | Mnemonic | mode | operand | Semantics |
|---|---|---|---|---|
| 0x35 | `COUNT` | bits0-2 cmp (`=`,`≠`,`<`,`≤`,`>`,`≥`); bits3-5 axis (0 child, 1 named child, 2 descendant, 3 following sibling, 4 preceding sibling, 5 ancestor) | index into the count table | let `n = |{y ∈ axis(x) : sub accepts at y, and y is reachable under stop}|`; keep π iff `n cmp c` |

The count table entry is `{sub: u16 (sub-program entry, or NONE = every node), c: u32, stop: u16 (set id, or NONE)}`.

- **Early stop.** Enumeration stops as soon as the comparison is decided. For example, for `≥ c` it stops when `n` reaches `c`, and for `< c` it stops at `n = c` and fails.
- **Bound.** `c` is also a work bound per root.
- **Executor.** For axis descendant with `sub` a pure kind test, the executor uses the prefix-count column of the set stage (`COUNT_PREFIX`, §2.6): `n = pc[end_pre(x)] − pc[x+1]`. In general `sub` runs per candidate as in `EXISTS`.
- **Verifier:** `c ≤ 2^31`; the axis code is valid; `sub` is a declared sub-program; `stop` is a declared set.

### 2.6 Set stage

The set stage is a list of **set definitions** in the group header (§3.4), evaluated before any instruction program of a task that reads them. Each definition produces, for each document in the batch, a node set `S ⊆ [0, n)`. Instruction programs read sets with:

| Opcode | Mnemonic | mode | operand | Semantics |
|---|---|---|---|---|
| 0x50 | `IN_SET` | bit0 negate | set id | keep π iff `x ∈ S` (or `∉` if negated) |
| 0x51 | `COUNT_SET` | bits0-2 cmp; bits3-5 axis as in `COUNT` | index into the count-set table `{set: u16, c: u32}` | keep π iff `|axis(x) ∩ S| cmp c`, computed from `S`'s prefix counts for descendant, and from popcounts over the child or sibling ranges otherwise |

Set definitions, where `n` is the document's node count:

| Type | Parameters | Definition |
|---|---|---|
| `LEAF` | instruction sub-program `p` (no unbounded axis), optional candidate kind set `K` | `{y : kind(y) ∈ K, and p accepts at y}`. Candidates come from the kind postings (the index) or a `kind` scan |
| `HAS` | inner set `S`; depth `NEIGHBOR \| END`; optional stop set `P`; optional field `f`; named flag | `{x : ∃y ∈ S, y a proper descendant of x (a child if NEIGHBOR), and no node strictly between x and y on the path lies in P, and (f = 0 or field(c) = f, where c is the child of x on the path)}` |
| `INSIDE` | inner set `S`; depth; stop `P`; field `f` | `{y : ∃x ∈ S, x a proper ancestor of y (the parent if NEIGHBOR), with no node of P strictly between, and the field condition applied to the child of x on the path}` |
| `PRECEDES` / `FOLLOWS` | inner `S`; `IMMEDIATE \| ANY`; stop `P` | `{x : ∃ a following (preceding) sibling y of x in S, the nearest one if IMMEDIATE, and no sibling of P strictly between}` |
| `AND` / `OR` / `ANDNOT` | `S1, S2` | set algebra |
| `COUNT_PREFIX` | `S` | an auxiliary column `pc[i] = |S ∩ [0, i)|`, used by `COUNT`/`COUNT_SET` |
| `FIX` | a recursive utility group lowered to equations `S_u = F_u(S_*)` with each `F_u` monotone | the least fixpoint, computed by waves (below) |
| `FACT` | fact digest; optional `column = const` | `{y : (doc, y) ∈ f}`, a posting list loaded from the fact cache (`spec.md` §21.10) |

**Evaluation.** Each formula below gives one Metal execution, as in `metal-recursion.md` §Design.
- `HAS(S, END)` with no stop: `x ∈ H ⟺ S ∩ (x, end_pre(x)) ≠ ∅`.
  - Posting form: two binary searches per candidate.
  - Bitset form: a reverse-preorder pass, `sub[par(i)] |= sub[i] | [i ∈ S]`, so that `H = {x : sub[x]}`.
- `HAS` with stop `P`: build `anc_P[i]`, the nearest proper ancestor of `i` that lies in `P`, in one preorder pass. Then `x ∈ H ⟺ ∃y ∈ S ∩ (x, end_pre(x)) : anc_P[y] ≤ x or anc_P[y] = NONE`, restricted to the path strictly between. Evaluate it as a reverse pass that clears the bit at `P` nodes before propagating: `sub[par(i)] |= (i ∈ P ? [i ∈ S] : sub[i] | [i ∈ S])`.
- `INSIDE`: a forward pass, `anc[i] = [par(i) ∈ S] | (par(i) ∉ P ? anc[par(i)] : 0)`.
- `NEIGHBOR` forms: the single-level `par(i)` test.
- `PRECEDES`/`FOLLOWS`: a sibling-chain pass in preorder over each parent's children.
- `FIX`:
  - `has`-direction groups run in levels from the deepest to the shallowest; `inside`-direction groups run from the shallowest to the deepest;
  - one wave per depth level, each a node-parallel evaluate plus a scan;
  - the wave count is the batch's maximum depth;
  - waves are enqueued device-side, with no CPU round trip.

**Representation.** Each set is a bitset (n bits per document, packed u32) or a sorted posting list (u32 preorder ids). The planner chooses from the set's bound (§4.8). `IN_SET` is a bit test on a bitset, or a binary search on a posting list. Both representations are admitted on every backend.

**Multi-target packing.** Up to 32 or 64 sets of the same pass type, depth and stop are evaluated in one pass as bit-planes of one word per node. This is the measured mask pass of `fusion.md`, which handled 42 subtree targets and 30 ancestor targets at 25–54×.

**Strata.** Set definitions form a DAG through their inner sets. A set's stratum is the length of the longest path to a `LEAF`. Every set of stratum `k` is evaluated after stratum `k−1`, and passes in one stratum are packed together.

**Verifier:**
- the definition graph is acyclic apart from `FIX` groups, whose equations must be monotone (no `ANDNOT` and no negated `IN_SET` through a recursive reference);
- every set id read by `IN_SET`/`COUNT_SET` is defined;
- `LEAF` sub-programs contain no unbounded axis;
- the number of strata is at most `MAX_STRATA = 16`.

### 2.7 Lazy enumeration

This is normative for every backend in set mode. `CHILD`, `DESCEND`, `FOLLOWING`, `ANCESTOR`, `PRECEDING` and the `*_KIND` forms must be enumerated through a single resumable cursor per enumeration: (axis, origin, next position, end). They must not push one pending path per result.

The semantics are unchanged (§1.4, freedom 3). The pending set grows with program nesting, not with subtree size. Measured: pushed paths fell from 5.26M to 96k on the fusion benchmark.

### 2.8 Side facts

| Opcode | Mnemonic | mode | operand | Semantics |
|---|---|---|---|---|
| 0x58 | `SIDE_TEST` | fact column `c` (u8) | index into the side-predicate table `{cmp, constant u64 or string id}` | keep π iff `side_c(x) cmp constant` |
| 0x59 | `SIDE_EQ` | fact column `c` | capture label `l` | keep π iff `side_c(x) = side_c(last capture with label l)` |
| 0x5a | `TEXT_EQ` | 0 | capture label `l` | keep π iff `text(x) = text(capture l)`. Executors compare a 64-bit text hash column first (when built), then the bytes |

- Fact columns are produced by native producers, fact programs, or derived facts with key (doc, node) and node → value (`spec.md` §21.10), per document digest (`fact_scope.owner`, `fact_def.qualname_id`, `fact_ref.target_id`, and the text hash) and cached by digest (`spec.md` §9).
- **Verifier:** column `c` must be declared in the group's side-column list. The executor ensures those columns are resident before dispatch.

### 2.9 Document switch

| Opcode | Mnemonic | mode | operand | Semantics |
|---|---|---|---|---|
| 0x5b | `DOC_GUEST` | 0 | via code | the root of each derived document of `x`'s file with that `via` whose host-side segment intersects `[start(x), end(x))` |
| 0x5c | `DOC_HOST` | 0 | 0 | if `x`'s document is derived and `x` maps through the source map to a single contiguous host range `[a, b)`, the deepest host node whose span covers `[a, b)`. Otherwise no result |

- **Verifier:** the via code is valid for the program grammar's `embeds`.
- After a switch, subsequent `KIND` and `FIELD` operands refer to the destination document's grammar. The verifier tracks grammar per pc and rejects a mismatch.

### 2.10 Emission

`EMIT ℓ` is unchanged in encoding (`0x40`, operand = label). Its semantics follow §1.4, by label kind. Exports read the declared captures of `C` and convert them to typed values (text, a span, a side-column value) through the label's export schema.

### 2.11 Order against a capture (E16)

| Opcode | Mnemonic | mode | operand | Semantics |
|---|---|---|---|---|
| 0x53 | `ORDER` | bits0-2 cmp; bit3 byte position instead of preorder | capture label `l` | keep π iff `pos(x) cmp pos(last capture l)` |

- **Verifier:** `ORDER` reads a defined capture.
- **Withdrawn (decision 0005).** Keyed sets (`KEYS`/`IN_KEYS`, E15) and bottom-up folds (`FOLD_UP`, E17) were proposed for within-document name joins, recursion through names and "all branches" properties (`research/heavy-inputs.md` §3). They are withdrawn under §0 rule 3: each added a new execution kind to every backend and to the verifier, while the same work is cheap and shareable as a derived fact or native producer. Joins between independent matches use lowering rule L5; recursion through names uses derived facts such as `py.call_edge_local` and `py.local_flow` with `reaches`; "always raises" uses the native producer `py.flow_exit` read through `SIDE_TEST`. M7's quadratic case is served by L5's indexed join (measurement M10).

---

## 3. Group header and tables

### 3.1 Dispatch table

```
dispatch: u16[CODES]      -- CODES = grammar's folded-code count (≤ 1024); entry pc or 0xFFFF
any_kind: u16[]           -- entry pcs of members with no root-kind anchor (scan every node)
```

A demand row `(G, D)` with no root list starts one task per node `r` of `D` with `dispatch[kind(r)] ≠ 0xFFFF`, plus one per node for each `any_kind` entry. That is fused dispatch without an index. A demand row with roots `(G, D, R)` starts tasks only at `R`, using the same table. Each entry pc begins that kind's trie (§4.3).

### 3.2 Label table

```
LabelEntry { rule: u32, fragment: u16, kind: u8 (KEY=0, EXPORT=1, COUNT=2),
             flags: u8 (bit0 ALL: every distinct export tuple), schema: u16, budget: u32 (per-label work bound) }
```

- **Limit:** `n_labels ≤ 4096` per group.
- **Executor label sets** use bitsets of width `W = 64` (one u64 register), `128` or `256`. A group with more labels than its target backend's register width is split by the planner (§6.2).

### 3.3 Reachable-label masks

For each pc, `R(pc)` is the set of labels whose `EMIT` is reachable from `pc` in the control-flow graph. Sub-program regions are excluded, since they cannot emit. Storage:

```
reach_index: u16[m]       -- per pc, index into reach_pool
reach_pool: u64[W/64][]   -- deduplicated masks
```

The executor keeps `Fired(task)` as a label bitset. At every `FORK`, at every navigation result, and before resuming a cursor, a path is dropped when `R(pc) ⊆ Fired(task) ∪ AllLabels(task)`, where `AllLabels` holds the labels marked `ALL`, which are never considered fired. Measured: without pruning, tries were 2–3× slower at small K.

### 3.4 Other tables

These are all u16-indexed, with at most 65,536 entries each:
- strings, predicates, capture predicates, u64 constants (Tempo's tables);
- `nth`;
- `count`, `count_set`;
- sub-programs `{entry, end}`;
- utilities (§2.4);
- bindings;
- side predicates;
- the side-column list;
- set definitions `{type: u8, params…, repr: u8, bound: u32}`;
- export schemas.

### 3.5 Binary layout and digest

A group serializes as a header, then the tables in a fixed order, then `W`. Its **canonical digest** is SHA-256 over the serialization after normalization (§4.1). The digest keys the plan cache and the dump tools (§7).

---

## 4. Side programs

Structural words call typed side programs through `TEXT` (value programs on the current node's text), `ASSERT` (capture programs over the path's captures) and `EMIT` (output programs). Each side program has its own table in the group; the structural word's operand is the table index. Side programs are pure: they read, return a Boolean (or, for output programs, a rewritten record) and charge work.

### 4.1 Value programs (`TEXT p`)

Tempo's `ValueProgram` (`transducer/value.rs:157`), applied to the bytes `source[start(x), end(x))`:

| Variant | Semantics | Encoding |
|---|---|---|
| `Equal s` | bytes = `strings[s]` | string id |
| `Prefix s` | bytes start with `strings[s]` | string id |
| `Regex` | unanchored search unless the pattern anchors; Rust `regex` syntax | byte NFA: `Bytes(ranges)`, `Look(kind)`, `Split`, `Capture(slot)`, `Fail`, `Match` (`value.rs:222-255`); Metal runs it as an NFA frontier (`metal_regex_frontier.metal`) |
| `Lua` | Lua 5.x patterns (frontier, balanced match, back-references) | `LuaInstruction` stream (`value.rs:330`) |

Set-mode additions (normative):

- `TEXT_ID b` (0x0c, §5.2): the pattern is evaluated once per distinct token text, not per node; `b` is its bit in the result table indexed by the node's interned text id. `TEXT_ID_EQ` (0x0f) compares the text id with one literal.
- `TEXT_DFA p` (0x0d, §5.3): the value program compiled to a dense byte DFA where the pattern admits one (no look-around beyond `^ $ \b`, no captures read downstream, no back-references). Required on Metal where admissible; the NFA form remains the reference.
- `TEXT_SET q` (0x0e, §5.3): a multi-pattern test. `q` names a pattern set; the executor runs one combined automaton over the node text and writes the matching pattern ids into a bitset, which later `TEXT_SET_BIT` reads (mode bit0) without rescanning.

Every value program declares a work bound: bytes × states for NFA, bytes for DFA. A node whose text exceeds the bound yields `Unknown(resource)` for the task.

### 4.2 Capture programs (`ASSERT c`)

Tempo's `CaptureProgram` (`value.rs:17`): `Equal`, `Text`, `AnyOf`, `Kind`, `Contains`, `Relation` (parent/ancestor between captures), `SameLine`, combined with `CAPTURE_EACH`, `CAPTURE_ZIP`, `AGGREGATE all|any` and `SELECT_POLARITY`. They implement Tree-sitter predicates (`#eq?`, `#any-eq?`, `#match?`, `#has-ancestor?`) and Rerail capture constraints.

In set mode the common case, equality of two captures' text, is the dedicated `TEXT_EQ` word (§2.8); `ASSERT` remains for the general forms.

### 4.3 Output programs (`EMIT ℓ` with an output id)

Tempo's `OutputProgram` (`#set!`, `#offset!`, `#strip!`, `#select-adjacent!`, `#downcase!`, MIME). Under the findings contract, presentation is owned by the integration, so output programs are used only where they change the finding key (for example, an offset that moves the reported line). All others are dropped at lowering and recomputed during rehydration.

---

## 5. Text and regular expressions

Evidence: `research/regex.md` (all 1,140 regexes of the data-layer rules, measured on the 9,010-file gate corpus).

### 5.1 Code conditions are AST conditions

Whole-file text regex clauses (`Query::Regex { source: Content }`) exist because 14 data-layer rules were ported from nera-data-layer's own line-scanning linters at exact parity. Their line state machine (`compile/line_state.rs`), comment filters and indentation filters rebuild parser knowledge by hand (string and docstring tracking, "not a comment", "not under `if TYPE_CHECKING:`"). Measured census of the 113 clauses:

| Class | Count | Lowering |
|---|---|---|
| (a) expressible as AST rules now | 107 | ordinary AST rules; comment, docstring and string exclusion is automatic |
| (b) small extension | 2 | layout line facts (`SIDE_TEST` on `line(i)`, end-line of the module), §6.2 |
| (p) path only | 4 | `filePath` at demand time |
| (c) genuinely textual | 0 | — |

Normative consequences:
1. **A text clause over code is rewritten as an AST rule.** The rule compiler rejects a `Content` regex clause on a language with a grammar unless the clause is marked `textual: true` with a reason, and the census tool reports every such clause.
2. **The line state machine, indentation filters and content gates are removed** from the rule language once the 113 clauses are rewritten. Under the findings contract this is a correction: today's filters miss the 18% of raw matches that fall inside string literals.
3. **Inline suppression comments** (`# noqa: …`) are handled by the integration's suppression step (`findings-contract.md`), not by rule-level line filters.

### 5.2 Token text: evaluate once per distinct text (`TEXT_ID`)

Regexes on token nodes (identifier, type_identifier, keyword, operator, `string_start`) are evaluated once per **distinct token text**, never per node. Measured: 6.0M identifier occurrences, 231,862 distinct names; per occurrence costs 33–69 s, per distinct name 0.11 s.

- **Vocabulary.** The tape producer writes a `tid` sidecar: one u32 per token node, an id into a global append-only `text_vocab(id, bytes)` in the content store. Interning costs 0.22 s per corpus on one thread and belongs at ingest.
- **Result table.** At plan time each `TEXT_ID` pattern gets a bit index `b`. For every vocabulary entry the planner computes the pattern bits once:
  - exact literal sets (396 of 444 identifier patterns) by hash lookup; literals absent from the vocabulary match nothing;
  - prefix, suffix and contains sets through Aho-Corasick;
  - the remaining general patterns through one combined dense DFA if its determinized size is within budget, otherwise per-pattern DFAs (measured hybrid: build 0.44 s, 3.0 MB, run 0.11 s for all 444).
  The result `R[tid]` is `W × u32` per entry, or a sparse posting list per pattern when its bits are rare. It is cached by (pattern-set digest, vocabulary length) and extended incrementally as the vocabulary grows.
- **Executor.** `TEXT_ID b` (0x0c) keeps π iff `R[tid(x)·W + b/32] >> (b mod 32) & 1`. One load on every backend; no automaton runs on the GPU. `TEXT_ID_EQ` (0x0f, operand = literal id) compares `tid(x)` directly.
- **Verifier.** `TEXT_ID` only on kinds the grammar marks as single-token; `b` within the group's bit table.

### 5.3 Other node text (`TEXT_DFA`, `TEXT_SET`)

- **Features.** The rule language's regex dialect is Rust `regex` without look-around or back-references (none are used). Capture groups inside a regex are never read downstream.
- **`\b` is ASCII in the rule language.** With that definition all 725 node patterns and 51 value patterns compile to dense DFAs (`regex-automata`); with Unicode `\b`, 15 fail. Making `\b` ASCII is required by "no fallback": a DFA that quits on non-ASCII bytes would need another engine.
- **`TEXT_DFA t`** (0x0d). Table `t`: a header (stride, anchored and unanchored start states, end-of-input class), a 256-entry byte-class map, u16 or u32 transitions, per-state accept bits. Tables are deduplicated by digest. Placement: constant memory when ≤ 64 KB (708 of 725 node patterns), device memory otherwise. Literal probes (minimum and maximum length, one required byte) run before the loop. The loop is bounded by the node's byte length under the label budget; exhaustion is `Unknown(resource)`.
- **`TEXT_SET q`** (0x0e) only for groups of at most 64 patterns on the same node whose combined DFA fits the budget. Determinization blows up past roughly 450 unanchored patterns (639 patterns: 676 s, 201 MB), and the `regex` crate's `RegexSet` falls off its lazy DFA on whole files (92.6 s against 0.52 s for separate patterns). The planner splits groups accordingly. A "single multi-pattern pass per file" is not used.
- **The NFA value program** (`Regex` in §4.1) remains the reference semantics and is used by the CPU interpreter only for differential testing.
- **Rewrites before any regex runs** (lowering rules, §6.5):
  - `^async\b` / `^def\b` on `function_definition` → `KIND` test on the `async` keyword child;
  - `(?i)^[rub]*f` on `string` → `TEXT_ID` on `string_start`;
  - a regex on a large node's text that names a token (`\bCallable\b` on `assignment`) → `has stopBy end` + `TEXT_ID`;
  - `module`-hosted literal alternations → document prefilters through `search_term_fts`, then the AST rule;
  - regexes that partition DLRE's authored-SQL inputs by first letter disappear with the decision 0002 rewrites.

### 5.4 Line documents

For a future clause that is genuinely textual (class c), the file's lines are a derived **line document**: one node per line, with `start`/`end` byte spans, built once per resident source from the `line_starts` sidecar (`spec.md` §20.6) using byte `memchr` (0.49 s per corpus against 3.5 s for per-character splitting). Programs over a line document use `TEXT_DFA` on line nodes and the line-document words:

| Opcode | Mnemonic | Semantics |
|---|---|---|
| 0x70 | `LINE_NEXT` | move to the next line |
| 0x71 | `LINE_PREV` | move to the previous line |
| 0x72 | `LINE_HOST` | `DOC_HOST` from a line to the deepest tree node covering it (so a textual condition can still test that it is not inside a string or comment) |
| 0x73 | `LINE_OF` | from a tree node to its first line |

No data-layer rule needs line documents. They are specified so that class (c) never reintroduces a separate text engine.

### 5.5 Trigram prefilter

For class (c) clauses, and for `TEXT_DFA` programs on very large nodes, a trigram index can prune documents before execution. Measured on the gate corpus: custom postings (u32 trigram → delta-varint file ids) take 0.127 B per source byte (about 0.62 B/node), build in 0.5 s in parallel, answer all 113 clause queries in 0.14 s, keep 6.2% of (clause, file) pairs, and missed none of the 119 files with raw matches. SQLite FTS5 trigram is 25× slower to query and, with `detail=full`, 16× larger.

**Decision:** no trigram index is built for the current rule packs, because after §5.1 nothing in them is textual (this confirms spec-indexes decision DI 4b and answers Q5). If a pack introduces class (c) clauses, the index is the custom posting index above, built at publication, queried with Cox-style boolean trigram queries derived from the regex, and used as a demand-time document prefilter in SQL.

### 5.6 Expected cost

| Run | Interning | Name evaluation | In-transducer lookups | File-text stage | Total |
|---|---|---|---|---|---|
| Cold | 0.22 s, at ingest (M) | 0.11–0.25 s (M/D) | ≈ 6 ms (6M × ~1 ns, A) | 0 (lowered) | ≈ 0.1–0.3 s |
| Warm | 0 | ≈ 0 (new names only) | ≈ 6 ms | 0 | ≈ 0.01 s |

Today's regex stage costs 9–17 s and finds nothing on the gate corpus.

---

## 6. Lowering the rule language

Every `AstMatcher` variant (`matcher/mod.rs:254`) and every `Query` form (`model.rs:545`) has a lowering. "Set stage" means a set definition (§2.6) read by `IN_SET`/`COUNT_SET`; the planner chooses between the inline form and the set-stage form with the cost model (`spec-fusion.md` §4.8).

### 6.1 Structural constructs

| Construct | Inline lowering | Set-stage lowering |
|---|---|---|
| `kind: k` | `KIND k` (root kind goes to the dispatch table) | `LEAF` candidate set |
| `regex: r` (node text) | `TEXT_ID` for token kinds (identifiers, keywords, operators), `TEXT_DFA` otherwise (§5) | `LEAF` with the text test |
| `pattern` | pattern words (§6.3) | `LEAF` whose sub-program is the pattern |
| `all: [...]` | sequence of conjuncts; non-local conjuncts as `EXISTS`; order by `cost/(1−selectivity)` | `AND` |
| `any: [...]` | `FORK` + one branch per disjunct, each ending in a jump to the continuation; set mode stops at the first success | `OR` |
| `not: r` | `NOT_EXISTS r` | `ANDNOT` |
| `has` / `inside` / `precedes` / `follows`, `stopBy: neighbor` | `CHILD`/`PARENT`/`NEXT`/`PREV` then `EXISTS` | `HAS`/`INSIDE`/`FOLLOWS`/`PRECEDES` with `NEIGHBOR` |
| same, `stopBy: end` | lazy `DESCEND_KIND`/`ANCESTOR_KIND`/`FOLLOWING_KIND`/`PRECEDING_KIND` inside `EXISTS` | `END` pass |
| same, `stopBy: <rule>` | axis loop with `NOT_EXISTS stop` guarding each step | pass with stop set `P` |
| `field: f` on `has`/`inside` | `CHILD_FIELD f` on the first step | `field` parameter of the pass |
| `nthChild` (An+B, `reverse`) | `NTH` | — |
| `nthChild.ofRule` | `COUNT` over `PRECEDING`/`FOLLOWING` with the rule as sub-program, compared to the An+B solutions | `COUNT_SET` over the rule's set |
| `matches` (utility, no parameters) | inlined by digest, or `CALL` | the utility's set; recursive groups become `FIX` |
| `matches` with parameters (`UtilCall`) | `CALL` with bindings | — (bindings prevent sharing unless the bound captures are absent) |
| `constraints` (`Constrained`) | after the rule: per constrained capture, `RESTORE` to it and run the constraint as `EXISTS` | — |
| capture equality across matches, same document | `EXISTS` from the reaching side with `TEXT_EQ`/`SIDE_EQ` | L5 + `FACT` when neither side reaches the other (§6.5) |
| "before" / "after" a captured node | `ORDER` | — |
| "all branches" / bottom-up properties | `SIDE_TEST` on `py.flow_exit` (native producer) | — |
| `compare` (`Comparison`) | `TEXT_EQ` for text equality; `ASSERT` capture programs for ordered comparisons and collations; `SIDE_EQ` for semantic readings | — |
| `count` | `COUNT` with early stop, or `COUNT_SET` | `COUNT_PREFIX` |
| `Never` | `STOP` | empty set |
| `TestUnknown` | test-only; lowers to a task status write | — |

### 6.2 Semantic constructs (`Semantic`)

Each family is a **per-document fact producer** that writes node-keyed columns, plus `SIDE_TEST`/`SIDE_EQ` reads in the rule program. Producers are rule-independent, cached by document digest, and listed in the relation catalog (`spec.md` §9).

| Family | Fact columns (per node) | Rule-side lowering | Producer |
|---|---|---|---|
| `resolves` | `scope_id`, `binding_id`, `resolved_target` | `SIDE_TEST resolved_target ∈ …`, `SIDE_EQ` | scope transducer program + native binder |
| `resolves.binding`, `binds` | `binding_site`, `binding_value`, `binding_annotation` (node ids) | `SIDE` read, then a jump to that node and a sub-program (`EXISTS` at the target node) | native binder |
| `value` | `value_kind`, `value_const` (interned) | `SIDE_TEST` | native evaluator over literals |
| `arguments` | `arg_slot` per argument node | `SIDE_TEST arg_slot = s` plus sub-programs at the argument | native signature binder |
| `parse` | derived document per parsed literal | `DOC_GUEST via=parse` then the inner rule | sub-grammar parse; derived document in the file image |
| `host` / `embedded` | — | `DOC_HOST` / `DOC_GUEST` | file image source map |
| `capture` (inside `parse`) | — | `CAPTURE` | — |
| `formatArgs` | `fmt_unused_arg`, `fmt_unfilled_field` flags | `SIDE_TEST` | native format-string parser |
| `layout` | line columns (§5.4): `line(i)`, `first_byte_col(i)`, line-gap to siblings | `SIDE_TEST` on line columns | derived from `start`/`end` and the line index |
| `filePath` | — | evaluated at demand time by SQL scope; never per node | scope |
| `flow_exit` | `exits` per statement or block: never / always / maybe | `SIDE_TEST` | native control-flow summary |
| `calls` | `callee_target` per call node (in-file part) | `SIDE_TEST`; the cross-file closure is an SQL relation joined above the transducer | in-file: native; closure: SQL graph relation |

The `calls` family is the only one whose full meaning needs other files. Its in-file part (which definition a call names locally) is a fact column; the closure through the workspace call graph is composed in SQL over the exported `(file, node, callee)` relation.

### 6.3 Pattern words

ast-grep patterns lower to these words (adapted from N1562 `rerail-pattern-lowering-design.md:382-480`, simplified by set mode). Opcode range `0x60–0x6f`.

| Opcode | Mnemonic | Operand | Semantics |
|---|---|---|---|
| 0x60 | `P_KIND` | kind | the current pattern node's candidate has this kind (alias-folded) |
| 0x61 | `P_TERMINAL` | terminal id `{kind, string, named}` | the candidate is this anonymous or named terminal with this text |
| 0x62 | `P_BIND` | capture id; mode bit0 named-only | bind a single metavariable `$X`; if `$X` is already bound in this match, the word is `TEXT_EQ` against it instead |
| 0x63 | `P_SKIP` | mode bit0 named-only | `$_`: accept any single node |
| 0x64 | `P_ENTER` | frame id | descend into the candidate's children, pushing a child cursor |
| 0x65 | `P_NEXT` | — | advance the child cursor, skipping trivia per the strictness level |
| 0x66 | `P_LEAVE` | strictness | require that no unmatched significant child remains (ast-grep `smart`, `cst`, `ast`, `relaxed`, `signature` levels), then pop |
| 0x67 | `P_ELLIPSIS` | ellipsis id `{capture or none, trailing anonymous terminals, adjacent}` | `$$$` / `$$$X`: consume zero or more children, nondeterministically (one path per split); two adjacent ellipses consume at least one child between them |

Set mode simplifies the N1562 design: no transactional capture environment is needed, because a failed fork discards its own captures and no first-binding order is observable. `$$$X` exported with `ALL` enumerates every split; otherwise the first success ends the task. Agent measurements: of 4.4k pattern atoms in the data-layer rules, only 20 are structural; the rest are single metavariables, which lower to `KIND` + `CAPTURE` (`research/fusion-model.md` §1).

**Verifier:** frame nesting equals the pattern's depth; every `P_ENTER` has a matching `P_LEAVE` on every path; ellipsis ids are in range; strictness is a defined level.

**Metal:** the child cursor is a per-thread register stack whose depth is the pattern depth (static). Ellipsis splits fork through the same lazy cursor as navigation (§2.7).

### 6.4 Query forms

| `Query` | Lowering |
|---|---|
| `Ast` | one or more transducer programs per grammar; findings from `KEY` labels |
| `Regex { source: Content }` | §5.1: rewritten as an AST rule (all 113 data-layer clauses qualify); a clause marked `textual` becomes a line-document program (§5.4), optionally prefiltered by a trigram index (§5.5) |
| `Regex { source: Path }` | evaluated once per file by SQL scope, at demand time |
| `SelectWhen { select, exists }` | per file: one program with `EXISTS`; cross-file: SQL semi-join over two exported relations |
| `RequireEach` | anti-join: per file, `NOT_EXISTS`; cross-file, SQL anti-join |
| `Budget` | `COUNT_SET` per file; repository totals in SQL over `t_count` |
| `Relational` (frozen, decision 0002) | re-expressed as the rows above. Of the 52 clauses, 44 lower fully to the IR, 1 reads a per-document fact column, 3 are IR plus a small cross-file SQL tail, and 4 are genuinely cross-file (`research/heavy-inputs.md` §4) |

### 6.5 Lowering rules

These rewrites apply before fusion and introduce no opcodes:

- **L1, document-invariant hoisting.** Conjuncts that depend only on the document (`filePath`, `inside {module + text regex} stopBy end`) are evaluated once per document as a gate, not once per candidate. Option residue that cannot match (`filePath: '\b\B'`) or always matches (`'(?s).*'`) is constant-folded. This alone removes the 2.59 TB module-text rescan measured in `research/fusion-model.md`.
- **L2, selection drop.** "First definition wins", "latest definition wins" and `row_number`-style pick-first selections are dropped under the findings contract, or restated as `NOT_EXISTS` an earlier sibling with `TEXT_EQ` + `ORDER` where the rule's meaning needs them.
- **L3, literal allowlists.** Path and qualname allowlists written as data compile to constant `filePath` tests or `not inside` chains.
- **L5, relational split.** Capture equality between independently matched nodes, and recursion through names, are split into candidate-exporting programs (rows `(doc, key_atom, node)` for candidates only) plus a derived fact or an SQL semi-join on `(doc, key)`. The result is findings, or a `FACT` set fed back to the outer program. A join that several rules share must use a catalog fact. Worked example and measurements: `research/shape-fixes.md` §3. On the full corpus, legacy_execution_surfaces went from more than 1,290 CPU seconds to 34 with exact fixture parity, by replacing 25 whole-tree inputs with local clauses plus 9 narrow candidate sets.
- **L4, no tree copies.** An input that exports every node (or every node of a kind group) to rebuild the tree in SQL is never admitted. Its consumers are re-expressed over the IR, and the planner rejects any export whose estimated rows exceed the document's node count.

---

## 7. Operation summary

Operations beyond Tempo's original IR (moved from `spec-fusion.md` §8):

| Op | Purpose | Encoding (opcode / mode / operand) | Verifier | CPU semantics | Metal implementation | Needed by |
|---|---|---|---|---|---|---|
| set mode | set-valued output, early stop | group-header flag | no containment reduction | §1.4 | label bitset register | every fusion |
| `ANCESTOR` | `inside` | 0x17 / named / 0 | — | parent chain | `par` column walk, bounded by max depth | relations |
| `PREV`, `PRECEDING` | `follows`, `precedes` | 0x1c, 0x1d / named / 0 | — | sibling chain backwards | sibling skip via `par` + `cidx` | relations |
| `*_KIND` | peephole | 0x18–0x1b, 0x1e / named / kind | valid kind | axis + kind | contiguous `kind` scan | relations, tries |
| `CHILD_FIELD` | field step | 0x1f / 0 / field | valid field | children with field | child scan | patterns |
| `NTH` | `nthChild` | 0x0b / named, from-end / nth index | `b ≥ 1` if `a ≤ 0` | `cidx` test | `cidx` column | patterns |
| `EXISTS`, `NOT_EXISTS`, `ACCEPT` | barriers, `not`, `stopBy: rule` | 0x32, 0x33, 0x34 / 0 / pc | regions, nesting ≤ 8, no `EMIT` | §2.3 | inline loops ≤ 2 levels; otherwise promoted to the set stage | AND, NOT, relations |
| `COUNT` | relation counts, `ofRule` | 0x35 / cmp, axis / count index | `c ≤ 2^31` | §2.5 | early-stop loop, or prefix counts | counts |
| `COUNT_SET` | count against a set | 0x51 / cmp, axis / count-set index | set defined | prefix counts | prefix-count column | counts |
| `CALL`, `RETURN` | utilities | 0x36, 0x37 / negate / utility | acyclic or monotone progress | §2.4 | inline regions; recursive groups become `FIX` | utilities |
| `IN_SET` | read a set | 0x50 / negate / set | set defined | bit test or binary search | bit test or binary search | set stage |
| set definitions | relations as passes, `FIX` | group-header table | acyclic, monotone `FIX`, ≤ 16 strata | §2.6 | segmented passes, GPU-driven waves | relations, utilities |
| `SIDE_TEST`, `SIDE_EQ` | facts (owner, qualname, refs) | 0x58, 0x59 / column / predicate or capture | column declared | column compare | column read | facts |
| `TEXT_EQ` | capture equality within a match | 0x5a / 0 / capture | capture defined | hash, then bytes | hash column, then bytes | patterns |
| `DOC_GUEST`, `DOC_HOST` | embedded and host | 0x5b, 0x5c / 0 / via | grammar tracked per pc | source-map lookup | document table plus binary search over segments | embedded rules |
| group header | dispatch, labels, reach masks | §3 | sizes, ranges | §3 | constant buffers | tries, demultiplexing |
| lazy enumeration | bounded frontier | normative, no encoding | — | resumable cursor | cursor register | every navigation |
| per-label budgets | `Unknown` per label | `LabelEntry.budget` | — | §6.10 | atomic per-label counters in the task's state | `Unknown` handling |



Additional rows for operations defined in this document:

| Op | Purpose | Encoding | Verifier | CPU | Metal | Needed by |
|---|---|---|---|---|---|---|
| `FACT` | read a derived fact as a node set | set definition | fact declared in the group | posting list | posting list | L5, derived facts |
| `ORDER` | position against a capture | 0x53 / cmp, byte bit / capture | capture defined | preorder compare | same | before/after tests |
| `TEXT_ID`, `TEXT_ID_EQ` | token-text tests once per distinct text | 0x0c / 0 / bit index; 0x0f / 0 / literal id | single-token kind; bit in range | result-table lookup | result-table lookup in device memory | identifier regexes |
| `TEXT_DFA` | DFA text test | 0x0d / 0 / DFA table | DFA-admissible pattern | dense DFA | DFA table in constant or device memory | node-text regexes |
| `TEXT_SET`, `TEXT_SET_BIT` | multi-pattern text | 0x0e / bit0 read-bit / set or pattern | set ≤ 64 patterns per bitset word | combined automaton | combined DFA | many regexes on one node |
| pattern words | ast-grep patterns | 0x60–0x67 | §6.3 | §6.3 | register child-cursor stack | `pattern` |
| line-document words | genuinely textual conditions | §5.4 | §5.4 | §5.4 | §5.4 | `Regex { Content }` class c |

---

## 8. Inputs and outputs

**Inputs per batch:**
- tape columns for every document in the batch (`spec.md` §3: `kf`, `size`, `base`, `off`, `tail`), read in place from residency slabs, with `par`, `cidx` and line columns as resident sidecars (`spec.md` §20.6);
- source bytes for documents whose programs read text;
- fact columns declared by the group's side-column list;
- the interned text-id column and text-id result tables (§5.2) when any program uses `TEXT_ID`;
- set-stage inputs: kind postings from the index;
- demand rows `(group, doc[, roots][, exports])` from SQL (`spec.md` §9).

Residency, pinning and page allocation of these inputs follow `spec.md` §20 (tape residency).

**Outputs** are the typed relations of `spec.md` §9:
- `t_match(group, label, doc, root)` for `KEY` labels;
- `t_export_<schema>(group, label, doc, root, values…)` for `EXPORT` labels;
- `t_count(group, label, doc, root, n)` for `COUNT` labels;
- `t_outcome(group, label, doc, status, reason)`.

Output placement on Metal is count → scan → scatter, with no atomics on output order. No output relation carries order, witnesses or spans beyond the declared export schema.

---

## 9. Verifier

A group is admitted only if every rule below holds. Each rejection has a stable error code and a fixture (`spec-fusion.md` §7.5).

1. **Encoding.** Every opcode is defined; operands are within their tables; table sizes are within the encoding limits (65,536 entries, `n_labels ≤ 4096`).
2. **Control flow.** Jumps stay in their region; sub-program and utility regions are properly nested and entered only through `EXISTS`/`NOT_EXISTS`/`CALL`; no `EMIT` inside a sub-program; every `ACCEPT`/`RETURN` is inside a region.
3. **Progress.** Every cycle in the control-flow graph advances along a navigation axis (Tempo's cycle-progress proof, `transducer/cycle_progress.rs`), or is a bounded `REPEAT`.
4. **Recursion and strata.** The rule and set-definition graphs are stratifiable. Recursion is positive within a stratum, monotone, along one declared axis; negation, `COUNT` and `NOT_EXISTS` occur only between strata. At most `MAX_STRATA = 16` strata and `MAX_SUB_DEPTH = 8` nested sub-programs.
5. **Grammar typing.** Kind and field operands are valid for the grammar in force at that pc (tracked through `DOC_GUEST`/`DOC_HOST`).
6. **Set mode.** No capture-containment reduction; `REPEAT_*` only without it; export schemas declared for every `EXPORT` label.
7. **Bounds.** Every loop is bounded by a batch constant (depth, fan-out, range, text length). Every value program has a work bound. Arena and set sizes are bounded at plan time from statistics.
8. **Backends.** Every operation used has an implementation on every backend, including Metal. There is no capability subset and no fallback.
9. **Patterns.** §6.3 rules.
10. **Text.** `TEXT_DFA` only for DFA-admissible patterns; `TEXT_ID` only on kinds whose text is a single token; line-document words only in line-document programs (§5.4).

---

## 10. Backends

| Backend | Role | Obligation |
|---|---|---|
| Rust interpreter | reference semantics and test oracle | implements every operation directly from this document |
| CPU JIT (arm64, from Tempo's `native_ir.rs`) | production on CPU-only deployments | identical output sets to the interpreter |
| Metal (interpreter kernels, then MSL code generation from `native_ir.rs`) | production on Apple GPUs; per-edit target | identical output sets; stratified range execution and GPU-driven waves (`spec.md` §8); resource exhaustion → `Unknown(resource)`, counted by `metal_coverage` |

Every backend charges the same abstract work units per operation, so budgets give the same `Complete`/`Unknown` outcome on every backend for the same input.

---

## 11. Constants

| Constant | Value | Source |
|---|---|---|
| Instruction words per group | ≤ 65,536 | 16-bit operand |
| Labels per group | ≤ 4,096 | §3.2 |
| `MAX_SUB_DEPTH` | 8 | §2.3 |
| `MAX_STRATA` | 16 | §2.6 |
| Steps per path (CPU reference) | 2^20 | Tempo `MAX_STEPS_PER_PATH` |
| Work per task (CPU reference) | 2^26 | Tempo `MAX_WORK_PER_TASK` |
| Patterns per `TEXT_SET` word | 64 | §4.1 |

The step and work limits become per-label budgets (`LabelEntry.budget`) sized by the planner; the constants above are upper bounds.

---

## 12. Open measurements

- Executor speed: M2 passed at 0.26–0.40 ns per program-node fused (`research/m2-m7.md`), but each evaluation costs 13–17 ns against a 3–5 ns target; the interpretive evaluator is 74% of time. A compiled (JIT) evaluation path is the next measurement.
- N1562's packed IR lowers none of transaction-patterns, async-sync-delegation or sync-async-signatures (missing `syntax`/`filePath` keys; all-or-nothing lowering; recursive utilities under negation rejected). E2, E6 and E14 `FIX` close those gaps.
- Rewriting the 113 text clauses as AST rules, with finding sets compared (§5.1).
- Pattern words against the CPU-only `PackedPattern` interpreter, on the 20 structural patterns.
- Metal: pattern child-cursor stacks and `TEXT_DFA` table placement (constant vs device memory).
