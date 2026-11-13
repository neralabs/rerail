# 0173: A program is a flat image

Status: draft (2026-11-13), for review. Would replace `front::Program`'s tree as the form a program is stored,
read, planned and rewritten in. Builds on decisions 0074 (rules as facts), 0140 and 0161 (the tape image), 0155
and 0162 (a stored program and its reader), and 0166 and 0172 (names and shared lists).

## Intro

A rule program today is a tree of pointers:

- a `Rule` holds an `Atom` and a list of `Literal`s;
- an `Atom` holds a `Name` and a list of `Term`s;
- a `Term` may hold a list of `Term`s.

Every name is an `Arc<str>`, and every list an `Arc<[T]>`. Decisions 0166 and 0172 made that tree cheaper to read
and to copy, but it is still a tree. Nothing in it can be handed to the GPU, written to disk as it is, or read
without building it. Each pass walks it pointer by pointer.

The engine already has a form with none of these limits: the tape. A tape is a tree laid out as flat columns: each
node's kind, its subtree's size and its span, in preorder. A node's children follow it, stepped by their sizes. It
is written to the store as it is, read in place (`tape_format::View`), and uploaded to Metal as it is.

This record makes a program the same kind of thing: a **program image**. Its nodes are in preorder, in columns, and
its names are the store's atom ids, not strings. Its facts are columns too. A program is read by viewing its image,
rewritten by writing a new image from the old one, and planned from node positions. No step makes a tree.

```
// p(x, y) :- e(x, y), x < 3.
//
//   node  kind   size  val            what
//   0     rule   10    -              the rule: itself, its head, then its body
//   1     atom   3     sym("p")       the head
//   2     var    1     sym("x")
//   3     var    1     sym("y")
//   4     atom   3     sym("e")       the first literal
//   5     var    1     sym("x")
//   6     var    1     sym("y")
//   7     cmp    3     op("<")        the second literal: left, then right
//   8     var    1     sym("x")
//   9     int    1     3              small, so held in val
```

A rule is a contiguous range of nodes, and so is each of its literals and arguments.

## What was measured

The tree is not where a run's time goes today. dl1 (the 44 data-layer policies, run as one pack of 334 rules),
`--profile profiling`, CPU, strict SQL, load 13 to 22:

| | process | the program's passes | of which the tree's own work |
|---|---|---|---|
| cold run (fresh store) | 66.2 G instr, 3.58 s user | `prepare`: 0.051 s | about 0.015 s (0.036 s is compiling text tests) |
| warm run | 20.3 G instr, 1.30 s user | about 0.012 s | reading the 2 stored texts: 0.0013 s |

So the image is not justified by its speed today. Under 1% of either run goes to the tree. It is justified by
what the tree cannot be:

1. **Not storable as it is.** Each read parses a text (0162) and builds a tree (0166, 0172).
2. **Not shareable with the device.** The GPU path has its own flat program tables (`BlockStep`, `MetalProgram`),
   made from the planner's output. Program-level work, such as file-local rules resolved inside a GPU dispatch
   (the target architecture's file-local recursion), has no device form to start from.
3. **Not relational.** Decision 0074 stored rules as facts, and the target architecture holds that everything is
   a fact. The tree is the one form the engine's own rules cannot read.
4. **Copied whole, and rewritten, on every warm run.** The warm path rebuilds the parts (`file_local::assemble`),
   applies the per-run edits, and on every `plan::run` call clones the program for `sql_kernel::normalize` and
   `sql::program::physical`. Each is small now. Each grows with the program, and each is a read of unchanged data.

## Decision (proposed)

### 1. Names are atom ids

Every name in an image is a `u32`:

- relations;
- variables;
- function names;
- string constants and string values;
- declaration columns and types;
- the variables in aggregates, `nearest` and `ast` literals, which are `String`s today.

The ids are the store's atoms (`content_store::atoms`, `Sym = u32`). They are persisted, dense and shared by every
worktree, and the native engine's symbol columns already hold them (`setfix`'s `Interner`). An image is then valid
against its store's atom table. An image made with no store (tests, the converter) carries its own table as a
section: offsets and bytes.

What this buys:

- Two names are equal when their ids are, as `Name`s are equal by address today.
- A rule's bytes are equal exactly when the rules are, so sharing is by a hash of bytes, not by a walk (§5).
- The image needs no string table of its own.

### 2. Nodes are columns, in preorder

An image's rules are one node table, three columns, each 16-byte aligned (the tape's and the slabs' alignment):

| column | width | holds |
|---|---|---|
| `kind` | u8 | the node's kind and flags (below) |
| `size` | u32 | the nodes in its subtree, itself included (the tape's `size`) |
| `val` | u32 | its payload: an atom id, an op, or an index into the ints (below) |

Children follow their parent, each stepped over by its size, exactly as `View::children` steps a tape. A rule is
one node and its subtree. Its head is its first child, and its body literals are the rest.

In the Intro's sample, the rule's size is 10: itself and its nine descendants. The rule after it starts at node 10.

The kinds:

| kind | val | children |
|---|---|---|
| `rule` | the rule's number in the source program, for provenance | head atom, then literals |
| `atom` (flags: negated, gated) | relation | its terms |
| `cmp` | the operator | left term, right term |
| `agg` (op in flags: count, min, max, sum, product, join) | the bound variable | `over` var or `wild`, the `by` vars, a `sep` string, the atom |
| `nearest` | the kind set | the two variables |
| `ast` | the matcher's text | document var, node var, then (name, var) pairs |
| `var`, `str` | the name | none |
| `int` | the value when it fits 31 bits; else, with the flag set, an index into `ints` (i64) | none |
| `wild`, `top` | none | none |
| `call` | the function | its arguments |

These are exactly `front::Literal`'s and `front::Term`'s cases. No case is lost, and none is added.

### 3. The rest of a program is tables

The other parts of a program are small, keyed tables, each a set of columns:

- **declarations:** per relation, a start and count into a columns table of (name, type), types as atom ids;
- **facts:** per relation, its rows column by column (`i64`, or `u32` atom ids for symbol columns). These are
  already a relation's columns, as the native engine and Metal take them (decision 0095 §3);
- **inputs, outputs, nullary relations:** atom id lists;
- **natives, lattices, gates, kind sets, settings, scopes, shapes:** one small table each, as `program_facts`'
  schema already names them (`native_reach(rel, seed, edge)`, `lattice(rel, keys, ty)`, and so on).

That last point is decision 0074's schema, kept. The difference is that its rows are columns of ids, not text, and
the hot part (rules) is the node table, not `rule`, `body`, `lit_atom`, `arg` and `term` rows joined back together.

### 4. Indexes are columns made with the image

Every consumer reaches rules by relation (the survey for this record found no consumer that reaches them by
variable). So an image carries, made as it is written:

- `rules`: each rule's first node (a rule's number is its index here);
- `by_head`: CSR offsets, rules grouped by head relation;
- `reads`: CSR offsets, for each relation the rules whose bodies read it, with the literal's node.

These are what `rules_by`, `front::strata`'s edges, `front::needed` and the planner's stratum maps build from the
tree today. They hold node positions, not addresses. That replaces `Run.numbers` (keyed by `*const Rule`) and the
`Forms` arena of address-stable boxes.

### 5. A run shares rules by their bytes

Two programs that hold the same rule hold the same nodes, byte for byte, because names are ids. A run's images can
therefore share a rule pool: a rule's bytes stored once, and a program a list of rule ids. That is what decision
0172's `Lists` table does, found by a hash of a byte range instead of by addresses. Within a store, a rule's bytes
can be content-addressed like any source.

This is the only sharing. There is no process table, and nothing outlives the run or the store.

### 6. A pass writes a new image

A pass reads one image and writes another. It does not edit in place. Every pass the survey listed does one or
more of five things:

1. drop rules or literals;
2. append rules, literals or declarations;
3. rewrite terms in place (strings to ints, calls to variables, a presence column inserted first);
4. rename relations;
5. move a relation's columns (project away, add a location).

Each is one stream over the node table:

- copy a range: a rule or literal kept as it is, one `memcpy`;
- skip a range;
- write new nodes;
- map `val` through a table: a rename is a `u32 → u32` map, with no string touched.

A pass that changes nothing writes nothing. It hands back its input.

This is the tape's own discipline: a tape is written once, in order, by its producer, and never patched. It is
also the form decision 0074 points at. Each pass is a function of the program's relations, so each can later
become a `.rr` program over the image's relations (§7) without changing what it reads or writes.

### 7. The image is the program's relations

The node table and its indexes are a relational encoding: `rule(r, node)`, `literal(r, k, node)`,
`atom(node, rel, neg)`, `term(node, kind, val)`. They can be exposed as relations, as views over the columns
(the tape vtabs' pattern), with no copy. Then the engine's own rules can read a program:

- prune and reachability (`front::needed`) is a recursive rule over `reads`;
- `merge_and_drop_repeats` is a group-by over rules' byte hashes;
- stratification is the engine's own SCC native (`components`) over `reads`.

That is the "phases, not new evaluators" shape applied to the compiler itself. It is the end state, not this
record's first step.

## Stages

Each stage leaves the engine whole, its tests green, and dl1's findings the same.

1. **The stored form.**
   - `program_text::store` writes the image beside the `.rr` text. The text stays, for people and for the
     converter's tests.
   - `load` views the image, and builds today's `front::Program` from it only where a caller still needs one.
   - Measured: a warm run's two reads.
2. **The warm path's own edits.** `file_local::assemble`, the per-run part edits, `normalize`, `physical` and
   `stated_apart` become image passes (§6), and the planner takes an image. `Step` holds a node position, not a
   cloned literal.
   - Measured: the warm run's program work, which should go to the planner's own.
3. **The prepare passes.** Simplify, magic sets, lowering, the file-local split and the rest move one at a time,
   each an image pass, each checked against the tree pass it replaces on every pack program (as decision 0162's
   reader was checked against the statement reader).
4. **The tree goes.** `front::Program` remains only as a debug printer's view, or not at all.
5. **Passes as rules (§7),** where a pass is a relational question: prune, merge, strata first.

## Open questions

1. **Atom ids against an image-local table.** The store's atoms make an image valid only against its store, and
   decision 0170 (another worktree) maps ids at the store. Should a stored image hold store ids (no mapping, but
   tied to the store) or its own table (portable, a map on load)?
2. **How far stage 2 reaches into the planner.** `Model` keys statistics by relation name strings, and the native
   `JoinSpec` maps variables to slots by name. Both would take ids. Is that in stage 2, or after it?
3. **Passes as streams, or as rules, first.** §6 makes each pass a stream (Rust code, rewritten). §7 makes some of
   them rules. Should stage 3 write streams and stage 5 replace some of them, or should the relational passes
   (prune, merge, strata) go straight to rules?

## Prior work consulted

- **Decision 0074:** rules stored as facts; the relational schema (`program_facts::schema`), kept in §3.
- **Decisions 0140 and 0161:** the tape image (preorder, sizes, 16-byte columns, 2- or 4-byte widths); §2 takes
  its layout and its stepping.
- **Decision 0095 §3, §6 and the Metal slabs:** u32 offsets from a buffer base, start/count ranges, symbols as
  codes into an atom table; §1 and §3 follow them.
- **Decisions 0155, 0162, 0166, 0172:** the stored text, its reader, interned names and shared lists, which this
  replaces in stages.
- **The target architecture:** everything a fact; file-local recursion inside the GPU dispatch; phases, not new
  evaluators (§7).
- **The survey of `front::Program`'s passes for this record:** the five kinds of rewrite in §6, the warm path's
  copies in Intro point 4, and the planner's address-keyed structures in §4.
