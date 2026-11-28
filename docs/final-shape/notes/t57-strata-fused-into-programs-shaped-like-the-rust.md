# t57: Strata fused into programs shaped like the Rust

Status: design for review (2026-11-25). A proposal, not a decision: nothing here is agreed until the user says so.
Follows t50 (the reference graph's shape) and t53 (where the engine runs a shape, not a join).

## Intro

**The problem.** On the `all` corpus (8,890 Python files) the head takes 3.2× W's CPU for the same findings:

| | CPU | wall | instructions | peak |
|---|---|---|---|---|
| W (6970a3e539, Rust) | 54.5 s | 18.5 s | 575 G | 1,042 MB |
| head (c53049388c, rules) | 172.5 s | 38.6 s | 2,120 G | 3,436 MB |

(One host, cold, unprofiled, the two runs back to back, `wvh.sh`.)

**Where the 118 s go.** The run's own profile (`RERAIL_PROFILE`, both runs back to back) puts 79% of the head's
CPU in rule strata, and splits it so:

| Part | Strata | Engine runs | Own CPU | W's counterpart |
|---|---|---|---|---|
| Per-file reference graph (`rf_*` strata run per chunk) | 98 | 6,121 | 33.7 s | W's per-file graph, in Rust: 7.7 s |
| Cross-file reference graph (`rf_*` strata, cross part) | 78 | 539 | 28.8 s | W's link: ~1.3 s (t50; not in this run's spans) |
| Other per-file strata (names, generated exports, demand, the program's own) | 260 | 13,225 | 64.2 s | partly W's 7.7 s (its scope table); the rest W does not do as rules |
| Other cross-file strata, and strata run in phase 1 | 217 | 782 | 10.9 s | W's strata: 12.6 s in all |
| Not strata (parse, frames, writes, measures) | | | 37.5 s | 32.4 s, which holds W's 7.7 s graph and its 11.5 s matchers |

The head's matchers run inside a stratum (`literal_str`); W's run outside any. The shares are from runs on a
loaded host, so their seconds are inflated; the split is what matters. The head's strata take 137.7 s of its 175.2 s
own CPU, W's 12.6 s of 45.0 s: the gap is the strata, and three quarters of the strata's time runs per file.

**Why the gap is structural.** W computes the reference graph in one walk of each file's tree, with a stack of
scopes, writing a compact summary of integers, and then links the summaries by name in hash maps. The head computes
the same thing as 176 strata. Each stratum, for each chunk of files, is one *engine run*: it loads its inputs,
builds indexes, joins row by row through an interpreter (setfix on the CPU, the stage program `join_stages.uglykl`
on the device), deduplicates its rows, settles them into a relation with indexes, and hands the relation to the next
stratum, which does the same. The per-file reference graph alone is 6,121 engine runs. A row of `rf_def(f, x)` is
built, stored, indexed and read again by `rf_qn`, where W's walk holds `x` in a register for the one line that
needs it.

t50 and t53 attacked this one rule shape at a time: a specialized operator per shape (nearest ancestor, the first
of a group, the binding in force at a read, the keyed walk), priced by the planner against the general join. Those
operators are right, and they are kept below. But each still runs inside a stratum, between stored relations, one
engine run per stratum per chunk. t50 estimated the run "in the low 40s of CPU s against W's 38–40" after its
steps; the head is at 3.2× W. The operators fixed the inside of the boxes; the cost is now the boxes.

**What changes.** The strata of a program's per-file part are *fused*: compiled together into one program per
document that runs as straight code over that document's tape, holding every intermediate relation in the form its
readers need (a register, a bit, a column indexed by node, a small table) and storing only what leaves the file.
The cross-file part is fused the same way into a link program over the files' exports. Both are generated from the
rules by the compiler (never written by hand), and both run on executors whose code has the shape of W's Rust: a
loop over a file's nodes, a stack of scopes, arrays indexed by node, tables keyed by interned names, worklists for
fixpoints. No interpreter runs per row; no relation is stored between two rules of the same fused program.

**Status.** Design only. Nothing is built. The first rung of the ladder (§6) is a census that classifies every
stratum of the benchmark program into the classes below; it decides how much of the 100 s the design reaches before
anything is built.

## 1. The target shape, by example

The running example is the qualified name of each definition, from `lib/python/references.rerail`:

```text
// rf_def(f, x): f is a definition (a function or a class) named x.
rf_def(f, x) :- kind(f, k), rf_dc(k), field(f, "name", n), text(n, x).
// rf_qn(f, q): f's qualified name: the module's, then each enclosing class's, then `<locals>` past a function.
rf_qn(f, q) :- rf_def(f, x), !nearest(f, _, rf_dc), docof(f, d), rf_module(d, m), q = @dot(m, x).
rf_qn(f, q) :- rf_def(f, x), nearest(f, s, rf_dc), kind(s, "class_definition"), rf_qn(s, p),
    q = @concat(p, ".", x).
rf_qn(f, q) :- rf_def(f, x), nearest(f, s, rf_dc), kind(s, "function_definition"), rf_qn(s, p),
    q = @concat(p, ".<locals>.", x).
```

**Today.** Two strata. `rf_def` is an engine run per chunk: it enumerates the definitions' kinds, probes `field`
and `text` per row, and stores `(f, x)` with an index. `rf_qn` is recursive: one semi-naive iteration per nesting
level, each joining the last level's rows with `nearest` and with `rf_def`'s index, deduplicating and settling
them. Its rows are stored and indexed again for its 12 reader relations (`rf_names`, `rf_owner_class`,
`rf_same_qn`, …).

**W.** `Extractor::walk` visits the tree once in preorder with a stack of enclosing definitions; at each definition
it reads the name and makes the qualified name from the top of the stack.

**Fused.** The compiler sees that `rf_def` is a function of `f` read only by `rf_qn` (so it is never stored), that
`nearest(f, s, rf_dc)` is the innermost open definition in preorder (a scope stack), and that `rf_qn`'s recursion
runs from an ancestor to its descendant (so one preorder pass computes it). The generated code has this shape
(written as Rust for the reader; the real form is in §2.3):

```text
// One preorder pass over a document's definitions (the kind-code set's nodes, read off the tape in place).
for n in doc.preorder_of(DEF_KINDS) {                  // rf_dc folded into a kind-code set at compile time
    scopes.pop_until_holds(n);                         // nearest(f, s, rf_dc): the open scopes, innermost on top
    let x = doc.text(doc.field(n, NAME));              // rf_def: an attribute in a register, never a row
    let q = match scopes.top() {                       // rf_qn: the enclosing scope was visited before n
        None => dot(module, x),                        //   a top-level definition
        Some(s) if doc.kind(s) == CLASS => concat3(qn[s], ".", x),
        Some(s) => concat3(qn[s], ".<locals>.", x),
    };
    qn[n] = q;                                         // a column by node: rf_ref reads it in a later pass
    scopes.push(n);
}
```

That is W's walk, generated from the rules. Every later rule of the per-file part that reads `rf_qn` reads `qn[n]`.
What the cross-file part needs (each definition's qualified name) is written once, as an export of 32-bit columns.

## 2. The architecture

### 2.1 The two programs

A program's strata are already split into a per-file part and a cross-file part (`file_local`'s split, 0290; t50
step 1). Each part becomes one fused program:

- **The document program** runs once per document, over that document's tape read in place (0353), with the run's
  constants (the program's settings, the corpus's package directories, the matcher wave's captures for the file) as
  read-only inputs. It writes the document's exports: the relations the cross-file part, the findings or the memo
  read. Documents run in parallel; a chunk (0289) is a block of documents the program loops over.
- **The link program** runs once per run (and per delta, later), over every document's exports. Its relations are
  keyed by interned names and definitions, not by node; its joins are probes of tables keyed by name; its fixpoints
  (imports, re-exports, bases breadth first, reachability) are rounds over a frontier.

### 2.2 The fusion compiler

Passes over the program's image (0701: passes that read a program are rules over its image), in order:

1. **Partition** (exists): per-file and cross-file, by the split.
2. **Keys and dependencies.** For each relation of a part: its key (a node of the document, a pair of nodes, the
   document, a name), the functional dependencies its rules prove (from the base relations' own: `kind(n, _)`,
   `field(n, f, _)`, `text(n, _)`, `nearest(n, _, set)`, `docof(n, _)` are functions of their first columns), and
   which relations of the same part read it. Structural only: never a relation's name.
3. **Representation**, per relation, from its key, its dependencies and its readers:

   | Representation | When | Generated as |
   |---|---|---|
   | **elided** | read by one reader in the same pass | the reader's loop computes it in place |
   | **node set** | unary, keyed by node | a bitset over the document's nodes |
   | **attribute** | a function of a node (proved) | a column indexed by node, with a presence bit |
   | **multi-valued by node** | many values per node | an offsets column and a values column (counted, then filled) |
   | **document table** | keyed by a name within the document | an open-addressing table of interned ids in the scratch block |
   | **rows** | none of the above | the document's rows in a scratch run, joined by the general in-document join |
   | **export** | read outside the part | written once, 32-bit columns where values fit, in sorted runs |

4. **Shapes** (t53's operators, now passes of one program instead of strata): a tree axis (`child`, `field`,
   `within`) is a tape read; `nearest` is the scope stack in preorder; a recursion from ancestor to descendant is an
   inherited attribute in preorder; a recursion from descendant to ancestor is a synthesized attribute in postorder
   (fusion research's subtree masks); "first of a group by an order" is a group argmin; "the binding in force at a
   read" is a search in an ordered run; the keyed walk is the scope stack with a binder set per scope.
5. **Schedule.** The part's strata in their order become passes; consecutive passes whose reads are satisfied by the
   same traversal order (flat, preorder, postorder) are merged into one traversal. A negation or aggregate reads only
   relations completed by an earlier pass (the strata's order, kept). A recursion that fits no axis is a semi-naive
   loop over the document's scratch tables, inside the document program.
6. **Room.** Each representation's size is a function of the document's node count and the exports' counts; the
   scratch block is planned before the program runs. Past the room is a hard error with what it needed, never
   a reroute.
7. **Lowering** (§2.3) and caching by the program image's digest; compile time is measured apart from the run.

### 2.3 What the fused program is lowered to

The fused program is an IR of passes over a document: loops over node ranges and traversals, reads of the tape
(`<ukl/reads/...>`, written once), the representations above as typed views of the scratch block, text functions as
IR operations (a document's made texts in a per-document arena, interned at export), and writes of exports.

Its lowering is **generated UKL source, one kernel per fused program** (0440's one kernel language), so the same
program runs on the CPU (rerail-ukl-vec's vector code, its AOT Rust, its JIT) and on Metal from one source, over the
same blocks. The traversal primitives carry their own per-backend lowering, as `nearest` does today (a stack pass on
the CPU, pointer jumping or a segmented scan on the device: 0038, 0040, rerail-ukl-aot's `f16_parent`): the IR says
"the innermost open definition", never "a stack" or "a scan".

This is the design's riskiest choice, so it is stated plainly: UKL's CPU form is vector code across threads between
barriers. A per-node formulation (a thread per node, a pass per barrier) fits it; W's walk is one thread per file
with a stack. Which of the two is faster on the CPU for these programs is measured by an oracle first (§6, rung
O): the per-node form and the walk form of the same fused program, written by hand, against W's walk. If neither
UKL form reaches the walk, that is an architectural clash to bring to the user, not to settle by a fallback.

### 2.4 The run

- The chunk job (0254) runs the document program over its chunk's documents instead of the chunk's strata.
- The per-file memo (0052, 0290) keeps each document's exports under its content, as it keeps its rows today; a
  changed file runs the document program for that file alone (L9, 0677: node ids renumber freely, only exports
  cross files).
- The link program runs where the cross part's strata ran. Its tables are blocks; its fixpoints are
  rounds whose frontier is a run of ids.
- The matcher wave stays the transducer's (0106, 0459); its captures for a file are an input of that file's
  document program, read where the wave wrote them, not a relation stored between them.
- The profile names passes, not strata: `document program/pass k (rf_def, rf_qn, …)`, so the profile still says
  which rules cost what.
- A debug mode writes every fused relation of a document as rows, so any pass can be compared with today's engine
  relation by relation.

## 3. Invariants

1. **One meaning.** A fused program computes the stratified model of the program, for every program: the same rows
   as today's engine for every relation it exports and, in the debug mode, for every relation it holds.
2. **Structural.** Partitions, representations, shapes and schedules follow from the rules' structure and declared
   facts, never from a relation's name (no `rf_` anywhere in the compiler).
3. **One source, every backend.** The CPU and Metal run the same generated kernel over the same blocks. No CPU-only
   structure (no `HashMap`, no `Vec` of pointers) holds a fused relation; scratch and tables are blocks.
4. **Generated, never hand-written.** No Rust function does a rule's work, and generated code calls no Rust.
5. **Document locality.** A document program reads its document's tape, its matcher captures and the run's
   constants, never another document's rows. The split proves it; the program cannot express anything else.
6. **Only what leaves is stored.** A relation is materialized only if the cross part, the findings or the memo read
   it; stored columns are as narrow as their values (32 bits where they fit); exports are written as sorted runs by
   the program (the host never sorts).
7. **Strata order kept.** A negation or an aggregate reads only relations an earlier pass completed; a pass reads a
   relation it is still filling only inside that relation's own recursion.
8. **Deterministic.** Rows do not depend on thread counts, chunk sizes or document order.
9. **Room planned.** A document's scratch is sized before its program runs; overflow is a hard error.
10. **Per-file memo and edits unchanged.** A document's exports are keyed by its content; one edited file reruns
    one document program.
11. **Proved, not assumed.** A functional dependency the representation relies on is proved by the compiler; debug
    builds check it (a second value for a key is a hard error naming the rule).
12. **Nothing silently unfused.** A stratum the compiler cannot place in a fused program is listed with its reason
    in the compiled program and in the profile. During the build-up those strata still run as today; the outcomes
    below require the list to be empty for the benchmark program.

## 4. Required outcomes

Measured on `all`, cold, one host, W and the head back to back (`wvh.sh`), and per stage with
`reference_stage_tests` (t50 §4).

1. **Rows.** Every output relation's rows equal today's head's; `reference_edge` and `reference_open` equal W's
   graph (`reference_edge_is_ws_reference_graph_on_*`); the findings' rows against W's differ only as they do today.
2. **Per-file reference graph** (stages scope, lookup and summary): rules ≤ 1.5× W's CPU per stage.
3. **Cross-file reference graph** (stage link): rules ≤ 1.5× W's CPU.
4. **The run.** Head CPU ≤ 1.25× W's, instructions ≤ 1.25× W's, peak ≤ 1.5× W's (today: 3.2×, 3.7×, 3.3×).
5. **No engine runs in the fused parts.** The benchmark program's per-file part makes zero engine runs (today
   19,346 across the per-file strata); one document program run per document; the cross part one link program.
6. **The code's shape.** The generated code of the per-file reference graph, read beside W's `Extractor::walk`,
   `ScopeTable::build_in` and `Graph::link_references`, has their bones: one traversal per merged pass, intermediates
   in registers or columns by node, no per-row index probe for a tree axis, no stored relation that only the same
   program reads. Judged by reading the code side by side at each rung that changes it, never by a count alone.
7. **Compile apart.** Compile time is recorded apart from the run, cached by the program's digest; a warm run
   compiles nothing.
8. **The device.** The same kernels compile for Metal and give the same rows, once Metal runs are allowed again.

## 5. What this costs, and what can go wrong

- **It replaces the per-file execution model.** For a fused program, setfix's semi-naive loop, settling, indexes and
  view reads are not used at all. That is the point, and it is a large build: a compiler (§2.2), an IR and its UKL
  writer (§2.3), and the run's changes (§2.4).
- **Not every rule fits a shape.** A join between two unrelated nodes of a document by text, or a recursion along no
  axis, becomes the general in-document join or a semi-naive loop inside the document program: still fused, still
  compiled, but slower than a shape. The census (rung 0) measures how much of the 100 s is in such rules before
  anything is built.
- **Functional dependencies can be missed.** A relation the compiler cannot prove functional is multi-valued:
  correct, but an offsets column instead of a column. The census counts these too.
- **Made texts.** `@concat`, `@dot` and `@rx_replace` make a text per definition, as W's walk makes a name. They go
  to a per-document arena and are interned only at export; a text read only inside the program is never interned.
- **The CPU's form is open** (§2.3): the per-node form fits UKL; the walk is W's. The oracle rung decides it, by
  measurement against W, before the generator is written.
- **The cross part's tables.** W links with hash maps and serial breadth-first loops. The link program's tables live
  in blocks and its fixpoints are rounds; reaching 1.5× of W there is a measured outcome, not an assumption.
- **The benchmark is not like for like.** The head runs `idc_dbg`, which outputs two debug relations (1.42 M and
  480 k rows) that W's program does not. Their cost is in the head's figures; it is reported beside every
  measurement and not removed without the user.
- **Debugging.** Without stored relations, a wrong row is found through the debug mode (§2.4), which writes every
  fused relation of a document as rows for comparison with today's engine.

## 6. The test ladder

Each rung is a unit test that runs in milliseconds, over a tiny program and a tiny Python source, written before its
code (ignored with its reason until its step lands). Every rung checks two things: **rows**, the fused program's
against today's engine on the same input (the engine is the oracle for meaning); and **shape**, what the compiler
chose (the representation, the passes, what was stored) and, where the rung changes generated code, the generated
code read beside its hand-written counterpart. The rungs climb from one rule to the benchmark program.

| Rung | Program and input | Rows | Shape | Locks |
|---|---|---|---|---|
| 0 | **Census**: every stratum of `idc_dbg` | — | each stratum's class (a shape, the general in-document join, a loop, unplaceable), with its CPU from the profile | §5's sizes |
| O | **Oracle**: `rf_def`/`rf_qn` and the scope stack, written by hand in the per-node form and the walk form | equal to the engine's | both forms timed beside W's walk on a corpus of 1,000 files | §2.3's choice |
| 1 | `a(n) :- kind(n, "call")` | equal | a node set; one loop over the kind's nodes; nothing stored | I3, I6 |
| 2 | `rf_def`: kind set, `field`, `text` | equal | an attribute; the kind set folded into codes; no index built | I2, I11 |
| 3 | a relation read by one reader | equal | elided: no column, computed in the reader's loop | I6 |
| 4 | negation of an earlier pass's set | equal | a bit test; the earlier pass completes first | I7 |
| 5 | `nearest(n, s, set)` | equal | the scope stack (or its per-node form, after rung O) | §2.2 shapes |
| 6 | `rf_qn`: recursion from ancestor to descendant | equal | one preorder pass; no semi-naive iteration; texts in the arena | §1 |
| 7 | a subtree property (descendant to ancestor) | equal | one postorder pass | §2.2 shapes |
| 8 | many references per definition | equal | offsets and values, counted then filled | §2.2 representations |
| 9 | a name read bound to a name defined in the same file | equal | a document table of interned ids; a probe per read | §2.2 representations |
| 10 | first of a group by an order; the binding in force at a read | equal | a group argmin; a search in an ordered run | t53's operators kept |
| 11 | a recursion along no axis (a chain of `nonlocal` declarations) | equal | a semi-naive loop in the document program; ends | I1 |
| 12 | a run constant (the package directories, a setting) read per file | equal | a read-only input; the relation stays per-file (t50 step 1) | I5 |
| 13 | exports and the memo: one file edited in a 3-file corpus | equal | only the edited file's program runs; exports 32-bit, sorted runs | I6, I10 |
| 14 | room: a document past its planned scratch | — | a hard error naming the representation | I9 |
| 15 | a stratum no class fits | — | listed with its reason; runs as today | I12 |
| 16 | debug mode on rung 6's program | equal, relation by relation | every fused relation written as rows | §2.4 |
| 17 | link: references to definitions by qualified name across 3 files | equal | a table keyed by interned name; a probe per reference | §2.1 |
| 18 | link fixpoints: a re-export chain, bases breadth first, reachability | equal | rounds over a frontier; no row per pair | §2.1 |
| 19 | the whole `python/references.rerail` on the stage fixtures | equal to W's per stage | per stage CPU beside W's | O2, O3 |
| 20 | `idc_dbg` on 10, then 50, then `all` | equal | zero engine runs in fused parts; the unfused list empty | O1, O4, O5 |
| 21 | the generated code of rungs 6, 9, 17 beside W's walk, scope table and link | — | the bones compared, written down | O6 |
| 22 | the same kernels for Metal (when allowed) | equal | compiled for the device from the one source | O8 |

Rungs 1–16 build the document program, 17–18 the link program, 19–22 prove the outcomes. Each rung's commit
un-ignores its test; a red rung is fixed at its root before the next.

## 7. How this relates to what exists

- **t50/t53's operators** (nearest, group argmin, as-of search, keyed walk, reach) are kept as the fused IR's shapes.
  What goes is the stratum around each one: its engine run, its stored rows, its indexes.
- **The split** (0290, t50 step 1) is the partition; this design consumes it unchanged.
- **Chunks** (0254, 0289): a chunk stays the unit of a job and a block; the job runs the document program instead of
  the chunk's strata.
- **Blocks read in place** (0353) and **no CPU-only structures**: the document program reads the tape where it lies;
  its scratch is a block.
- **UKL** (0440, 0567): the lowering target; the traversal primitives' per-backend forms extend what `nearest` and
  `f16_parent` already do.
- **Generated Rust** (0550): any Rust is written by the UKL backends from the generated kernel, never translated by
  hand or from a deprecated lowering.
- **The device stage program** (`join_stages.uglykl`) and **setfix** stay for the strata the census leaves unfused,
  until the list is empty for the programs that matter.

## 8. Questions that are the user's

1. **The lowering target** (§2.3): one generated UKL kernel per fused program for every backend (proposed), or a
   direct Rust writer for the CPU beside UKL for the device.
2. **The build-up** (I12): may unfused strata run as today while the ladder climbs, listed and reported (proposed),
   or must each rung's program be wholly fused?
3. **The bars** (§4): 1.5× W per reference stage and 1.25× W for the run, or tighter.
