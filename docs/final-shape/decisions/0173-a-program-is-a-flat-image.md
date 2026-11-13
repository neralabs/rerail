# 0173: A program is a flat image, mapped and edited in place

Status: draft (2026-11-13), for review. Would replace `front::Program`'s tree as the form a program is stored,
read, planned and changed in. Builds on decisions 0074 (rules as facts), 0140 and 0161 (the tape image), 0032 (slabs
read in place by the device), 0111, 0160 and 0170 (the store's atoms), 0155 and 0162 (a stored program and its
reader), and 0166 and 0172 (names and shared lists).

## Intro

A rule program today is a tree of pointers:

- a `Rule` holds an `Atom` and a list of `Literal`s;
- an `Atom` holds a `Name` and a list of `Term`s;
- a `Term` may hold a list of `Term`s.

Every name is an `Arc<str>`, and every list an `Arc<[T]>`. Decisions 0166 and 0172 made that tree cheaper to read
and to copy, but it is still a tree. Nothing in it can be mapped from disk, handed to the GPU, or read without
building it. Each pass walks it pointer by pointer, and a change to one rule is a copy of the program.

This record makes a program a **program image**, in the shape the tape already has:

- **Flat.** Its nodes are in preorder, in columns. A node's children follow it, stepped by their sizes.
- **Keyed on ids.** Texts are atoms, relations are numbers, and variables are slots. Nothing is keyed by a string
  or an address.
- **Linear.** The image is a fixed set of regions, and so are its string table and its indexes. Every reference is
  an offset into a region, so the whole image can be mapped from a file, or bound to the device, as it is.
- **Edited in place.** Rules and rows are appended, and are removed by a tombstone. An edit costs what it adds or
  removes, never the size of the image it changes.

```
// p(x, y) :- e(x, y), x < 3.        relations: p = 0, e = 1   variables: x = slot 0, y = slot 1
//
//   node  kind   size  val          what
//   0     rule   10    -            the rule: itself, its head, then its body
//   1     atom   3     rel 0        the head, p
//   2     var    1     slot 0       x
//   3     var    1     slot 1       y
//   4     atom   3     rel 1        the first literal, e
//   5     var    1     slot 0
//   6     var    1     slot 1
//   7     cmp    3     op <         the second literal: left, then right
//   8     var    1     slot 0
//   9     int    1     3            small, so held in val
```

A rule is a contiguous range of nodes, and so is each of its literals and arguments.

## What was measured

The tree is not where a run's time goes today. dl1 (the 44 data-layer policies, run as one pack of 334 rules),
`--profile profiling`, CPU, strict SQL, load 13 to 22:

| | process | the program's passes | of which the tree's own work |
|---|---|---|---|
| cold run (fresh store) | 66.2 G instr, 3.58 s user | `prepare`: 0.051 s | about 0.015 s (0.036 s is compiling text tests) |
| warm run | 20.3 G instr, 1.30 s user | about 0.012 s | reading the 2 stored texts: 0.0013 s |

So the image is not justified by its speed today. Under 1% of either run goes to the tree. It is justified by what
the tree cannot be:

1. **Not mappable.** Each read parses a text (0162) and builds a tree (0166, 0172). The store's atoms have the same
   problem: each process rebuilds its hash table from the segments it reads (0111).
2. **Not shareable with the device.** The GPU path has its own flat tables (`BlockStep`, `MetalProgram`), made from
   the planner's output. Program-level work has no device form to start from, such as file-local rules resolved
   inside a GPU dispatch (the target architecture's file-local recursion).
3. **Not relational.** Decision 0074 stored rules as facts, and the target architecture holds that everything is a
   fact. The tree is the one form the engine's own rules cannot read.
4. **Not editable by the edit.** The warm path rebuilds the parts (`file_local::assemble`) and applies the per-run
   edits. Each `plan::run` call clones the whole program for `sql_kernel::normalize` and `sql::program::physical`.
   A one-rule change costs the program. Each of these is small now. Each grows with the program, and each is a read
   of unchanged data.

## Decision (proposed)

### 1. Everything is keyed on ids

A name has one of three kinds of id, and only the first is ever a string:

| what | id | from |
|---|---|---|
| texts: string constants, function names, matcher texts, kind names, setting values | atom (`u32`) | the store's atoms (`content_store::atoms`, `Sym`) |
| relations | relation number (`u32`), dense in the image | the image's `relations` table: number → atom |
| variables | slot (`u16` in a `u32` val), dense in the rule | the rule's `vars` range: slot → atom |

What each buys:

- **Relation numbers.** Every per-relation table (declarations, facts, `by_head`, `reads`, the planner's statistics)
  is a dense array indexed by relation, not a map. A rename changes one row of `relations` and no node.
- **Slots.** A variable is already the column the planner binds it to. The native `JoinSpec` maps variable names to
  slots today; here it reads them. Names are kept only for printing and for errors.
- **Atoms.** The image holds no string of its own, and two texts are equal exactly when their ids are.

The planner follows: `Model`'s statistics are keyed by relation number, and `Step`s by node position. This is in
stage 2, not after it.

Atoms are the store's. A process that has forked from the store's atoms (decision 0170) holds its own ids in its
images, and they are mapped where a stored image is written, like any row that holds atoms (`write_back::Syms::Mapped`).
An image made with no store (tests, the converter) carries its own string table (§3), of the same form.

### 2. Nodes are columns, in preorder

An image's rules are one node table, three columns:

| column | width | holds |
|---|---|---|
| `kind` | u8 | the node's kind and flags (below) |
| `size` | u32 | the nodes in its subtree, itself included (the tape's `size`) |
| `val` | u32 | its payload: a relation, a slot, an atom, an op, or an index into `ints` |

Children follow their parent, each stepped over by its size, exactly as `View::children` steps a tape. A rule is one
node and its subtree. Its head is its first child, and its body literals are the rest. In the Intro's sample, the
rule's size is 10: itself and its nine descendants.

| kind | val | children |
|---|---|---|
| `rule` | its row in `rules` | head atom, then literals |
| `atom` (flags: negated, gated) | relation | its terms |
| `cmp` | the operator | left term, right term |
| `agg` (op in flags: count, min, max, sum, product, join) | the bound slot | `over` var or `wild`, the `by` vars, a `sep` string, the atom |
| `nearest` | the kind set's atom | the two variables |
| `ast` | the matcher's atom | document var, node var, then (name, var) pairs |
| `var` | slot | none |
| `str` | atom | none |
| `int` | the value when it fits 31 bits; else, with the flag set, an index into `ints` (i64) | none |
| `wild`, `top` | none | none |
| `call` | the function's atom | its arguments |

These are exactly `front::Literal`'s and `front::Term`'s cases. No case is lost, and none is added.

### 3. An image is linear: a fixed set of regions

An image is a file (or an anonymous mapping, for a run's own images) holding:

- a **header** of fixed size;
- a **fixed number of regions**, set by the image's kind. A program image has about twenty: `kind`, `size`, `val`,
  `ints`, `rules`, `relations`, `vars`, the declarations, facts and other tables (§5), and the indexes (§4).

A region is a typed array: `u8`, `u32` or `u64` items, or fixed-size records. Every reference between regions is an
item index, never an address, so the image means the same wherever it is mapped.

**How a region grows without moving.** A region is held in at most 32 **extents**:

- Extent 0 is the region as it was built, exactly its length.
- Each later extent is twice the size of the one before it (the first is the next power of two past extent 0's
  length, at least one page).

An item index is found in its extent with one compare and one count of leading zeros. Nothing is chased:
the extent table is in the header, at most 32 offsets a region. An extent, once made, never moves, so a reader's
mapping of it stays valid while the region grows.

- A built or compacted image has one extent a region: each region is one contiguous array, as a tape's columns are.
- An edited image gains extents as it grows: at most 32, and, by doubling, about log₂(growth) of them.

Extents start at page offsets (16 KiB on Apple silicon). Each can be bound to Metal as a no-copy buffer over the
mapping, the route decision 0032 takes for slabs. A kernel takes a region as its extent table and translates an
index the same way. A one-extent region, the common case, is a plain array.

A new extent is allocated at the file's end, at its full size. The file is sparse, so pages are used only as they
are written.

The header, per region:

```
// per region:  item width, committed length (u64), extent count, extent offsets[32] (u64, page offsets)
// and once:    magic, format, kind, generation, region count (fixed by kind), checksum
```

That is under 8 KiB for 30 regions: one page.

### 4. String tables and indexes are regions too

Every table that maps a key to rows, the string table included, is built from three linear forms.

**Dense arrays**, where the key is already a number (an id, a relation, a row):

- the atoms' end offsets;
- `rules`: each rule's first node, its `vars` range and its `dead` word (§6);
- `relations`: its atom and its columns;
- the planner's statistics by relation.

Appending is appending.

**A hash table, for keys that are bytes.** It is used for a text to its atom, and a rule's bytes to its rule (§7).
It is a *linear hash table*:

- The table is a region of 64-byte buckets. Each bucket holds 7 (tag, id) pairs, a count and an overflow link into
  an overflow region.
- A key's bucket is its hash's low bits, `level` bits or `level + 1`, by whether it is below the split point, both
  of them in the header.
- When the load passes its bound, the table grows by one bucket. The bucket at the split point is split into it,
  and the split point moves on by one.

So an insert splits at most one bucket. A lookup reads one bucket, plus overflow only past the load bound, and then
one string to confirm a tag. No insert rebuilds the table, so the cost of growing is paid one bucket at a time. This
is the form the store's atoms take in stage 4, in place of the hash table each process rebuilds (0111).

**A grouped index, for one key to many rows.** It is used for `by_head` (relation → its rules), `reads`
(relation → the literals that read it) and a program's rules (program → rule ids). A key's entries are in two
parts:

- the **base**: a start and count into one packed values region, the CSR form, as built or compacted;
- the **tail**: entries appended since, in 16-entry blocks in a blocks region, chained from the key's last block.

So an appended entry fills the key's tail block or starts one. A key's entries are its base slice, then its tail
blocks, in order. A built or compacted index has no tails: it is exactly CSR.

A string table is these forms together:

- the bytes, one `u8` region;
- the end offsets, a dense array;
- the hash table.

The image's own table, the store's atoms (stage 4) and a run's rule pool (§7) are all this one structure.

### 5. The rest of a program is tables

The other parts of a program are small tables, each a set of regions:

- **declarations:** per relation, a range of (name atom, type atom) columns;
- **facts:** per relation, its rows column by column (`i64`, or `u32` atoms for symbol columns), with a `dead` word
  a row (§6). These are already a relation's columns, as the native engine and Metal take them (decision 0095 §3);
- **inputs, outputs, nullary relations:** relation lists;
- **natives, lattices, gates, kind sets, settings, scopes, shapes:** one small table each, as `program_facts`'
  schema already names them (`native_reach(rel, seed, edge)`, `lattice(rel, keys, ty)`, and so on).

That last point is decision 0074's schema, kept. The difference is that its rows are columns of ids, not text, and
the hot part (rules) is the node table, not `rule`, `body`, `lit_atom`, `arg` and `term` rows joined back together.

### 6. An edit costs its own size

An image has one writer at a time (a lease, as the store's atoms have) and any number of readers, each reading the
generation it opened. Every edit is one of three kinds.

**Append.** An edit writes rules, rows, atoms and index entries past the committed lengths, into the last extent of
each region it grows, or a new one. Readers do not see them until the commit.

**Tombstone.** A removed rule or fact row has its `dead` word set to the generation that removed it (0 = live). A
reader at generation `g` sees a row as removed when its `dead` is nonzero and at most `g`. Setting it is one aligned
`u32` store, so a reader sees the old value or the new one, and either is right for its generation.

Index entries are not removed. A reader skips those whose row is dead. Each removal is also appended to a
`removed` log region, as (row, generation) pairs.

**Change.** A change is a removal and an append: a changed rule is the old one tombstoned and the new one appended.
A rule is the unit, so a change costs the rule, not the program. A rename is one row of `relations` (§1).

**Commit.**

- The writer flushes what it wrote, then writes the new header, generation `g + 1`, as one row of SQLite, in the
  same transaction as the rows that depend on it. SQLite stays the source of truth (memory may run ahead, as
  decided for the store).
- The file holds the bytes; the header in SQLite says how many of them are committed.
- After a crash, bytes past the committed lengths are ignored, and overwritten by the next edit. Tombstones written
  for a generation that never committed are found in the `removed` log past its committed length, and cleared. That
  costs the lost edit, not the image.

So the cost of an edit:

| step | cost |
|---|---|
| append k nodes, rows or entries | O(k), plus at most one new extent a region |
| insert k strings or rules into a hash table | O(k), at most one bucket split each |
| remove k rules or rows | O(k) |
| commit | the header (one page), and one flush of what was written |

**Compaction** is the one step that costs the image's size. It writes a new image of the live rows, each region one
extent, each index pure CSR, and readers move to it at their next generation. It is triggered geometrically: when
the dead rows, or the tails, reach half the live size. So its cost, spread over the edits that caused it, is O(1) an
item. It runs off the edit's path: no edit waits for one.

Atoms are never removed (0111: an atom keeps its id). Compaction keeps them, so ids are stable across it.

### 7. A run shares rules by their bytes

Within one image, two equal rules have equal nodes, byte for byte, because every name in them is an id. So a run's
programs can live in one pool image:

- each rule stored once, found by its bytes in the pool's hash table (§4);
- each program a key of a grouped index: program → its rule ids.

This is decision 0172's `Lists` table, found by a hash of a byte range instead of by addresses. A pool can be the
run's (an anonymous mapping, dropped when the run ends) or a store's (a file, kept). There is no process table.

### 8. Passes are edits, or rules, maintained by the edit

A pass either keeps a program's image up to date (an edit, §6) or writes a different program from it (a
derivation). Every pass surveyed for this record does some of five things:

1. drop rules or literals;
2. append rules, literals or declarations;
3. rewrite terms (strings to ints, calls to variables, a presence column inserted first);
4. rename relations;
5. move a relation's columns (project away, add a location).

For the warm path to cost the change, a derivation cannot be rewritten whole when its input changes by one rule.
So each derived rule records the source rules it came from (`from`, a grouped index: source rule → derived rules).
Then:

- **A pass that is local to a rule** (normalize, physical, the per-run part edits, lowering) is maintained by the
  edit. A source rule removed removes what came from it. A source rule appended runs the pass on that rule alone,
  and appends what it gives.
- **A pass that is not local** (prune and reachability, magic sets, strata, merging repeats) depends on the whole
  program. It is maintained as the engine maintains any derived relation: written as rules over the image's
  relations (§9), and kept up to date by the engine's own delta maintenance (counting, DRed). This is the reason to
  make these passes rules: so they cost the change, not the program, by the same machinery as everything else.

A build from nothing (a cold run, a new pack) runs every pass once over the whole image, as an append of everything.

### 9. The image is the program's relations

The node table and its indexes are a relational encoding: `rule(r, node)`, `literal(r, k, node)`,
`atom(node, rel, neg)`, `term(node, kind, val)`, `reads(rel, r, node)`. They can be exposed as relations, as views over
the regions (the tape vtabs' pattern), with no copy. Then the engine's own rules can read and maintain a program:

- prune and reachability (`front::needed`) is a recursive rule over `reads`;
- merging repeats is a group-by over rules' byte hashes;
- stratification is the engine's own SCC native (`components`) over `reads`.

That is the "phases, not new evaluators" shape applied to the compiler itself.

## Stages

Each stage leaves the engine whole, its tests green, and dl1's findings the same.

1. **The container and the stored form.**
   - Build the linear image: regions, extents, the three index forms and the commit, with its own tests:
     - growth across extents;
     - a reader at an old generation;
     - recovery from a lost commit;
     - compaction.
   - `program_text::store` writes a program's image beside its `.rr` text. The text stays, for people and for the
     converter's tests.
   - `load` maps the image, and builds today's `front::Program` from it only where a caller still needs one.
   - Measured: a warm run's reads.
2. **The warm path, by id and by edit.** The planner takes an image, keyed on relation numbers, slots and node
   positions (`Model`, `JoinSpec`, `Step`; `Run.numbers` by `*const Rule` and the `Forms` arena go).
   `file_local::assemble`, the per-run part edits, `normalize`, `physical` and `stated_apart` become edits of a
   run's pool image, maintained by `from`.
   - Measured: a warm run's program work, and a one-rule edit's.
3. **The prepare passes.** Local passes move one at a time, each an edit, each checked against the tree pass it
   replaces on every pack program (as decision 0162's reader was checked against the statement reader). Non-local
   passes are written as rules (§8, §9).
4. **The store's atoms.** The store's atom table becomes a string table of §4: a mapped file whose header is
   committed through SQLite. A process maps it, and no longer rebuilds a hash table. This touches 0111, 0160 and
   0170's atoms, which other worktrees are changing, so it is planned with them.
5. **The tree goes.** `front::Program` remains only as a debug printer's view, or not at all.

## Open questions

1. **Extents or reserved space.** §3 grows a region by doubling extents, at most 32 of them, so a region is not
   always one array. The other way is to reserve each region's whole virtual range in a sparse file at creation.
   Then a region is always one array, but its capacity is fixed when the image is made. Extents are proposed. Is a
   bounded extent table on the device acceptable?
2. **Where an image's bytes live.** §6 keeps the bytes in a file beside the database and commits the header through
   SQLite. A blob inside SQLite is not mappable (it spans overflow pages). Is a file beside the database, under the
   store's reclamation, acceptable?
3. **Tombstones as generations.** §6 spends a `u32` a row, so that old readers need no copy. A bitmap is 32 times
   smaller, but readers would have to stop at each commit. Is a word a row right for rules and fact rows?

## Prior work consulted

- **Decision 0074:** rules stored as facts, and the relational schema (`program_facts::schema`), kept in §5.
- **Decisions 0140 and 0161:** the tape image (preorder, sizes, 16-byte columns). §2 takes its layout and its
  stepping.
- **Decision 0032:** slabs read in place by the device, bound as no-copy buffers. §3's extents take the same route.
- **Decision 0095 §3, §6 and the Metal slabs:** u32 offsets from a buffer base, start/count ranges, symbols as codes
  into an atom table. §3 and §5 follow them.
- **Decisions 0111, 0160 and 0170:** the store's atoms, held in segments, given out without a lease, and mapped at
  the store when a process forks. §1 keeps their ids, and stage 4 changes their table's form only.
- **Decisions 0155, 0162, 0166 and 0172:** the stored text, its reader, interned names and shared lists, which this
  replaces in stages.
- **The store's durability (SQLite the source of truth, memory ahead):** §6's commit.
- **Linear hashing (Litwin, 1980):** the hash table's growth, one bucket at a time, in §4.
- **The target architecture:** everything a fact; file-local recursion inside the GPU dispatch; phases, not new
  evaluators (§8, §9).
- **The survey of `front::Program`'s passes for this record:** the five kinds of rewrite in §8, the warm path's
  copies in point 4 of "What was measured", and the planner's address-keyed structures in stage 2.
