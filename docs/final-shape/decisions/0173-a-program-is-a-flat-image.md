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

**A hash table, for keys that are bytes.** It is used for a text to its atom, and a rule's bytes to its rule (§8).
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

The image's own table, the store's atoms (stage 4) and a run's rule pool (§8) are all this one structure.

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

This section is for an image with **one line of history**: each version replaces the one before it, and no other
version is made from it. Most programs in a run are not like that. §7 covers the ones with several versions alive
at once.

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

### 7. Two versions at once: views, overlays and derivations

A survey of every place a program is made, read, copied or changed found these places where two versions of a
program, or of a structure made from one, are alive at once or compared:

| | where | the versions | how much they differ | shape |
|---|---|---|---|---|
| E1 | the prepared program (`corpus.rs` `PREPARED`, `Prepared`), its split (`file_local.rs:815`, `:823`) | the program, its lowered form, and its file-local and cross parts, each a `clone` then `retain` | the parts are complementary subsets | one base, three variants |
| E2 | each run's copy of the split (`corpus.rs:2748`), then the run's edits (cross memo, request cone, run constants) | the shared split, and the run's own | a few relations moved, inputs added, a cone split off; the copy is the whole program | one base, a variant for each run and each concurrent session |
| E3 | a session's cross part (`maintained_cross` `corpus.rs:9182`, `retain::Cut::program` `retain.rs:334`, `maintain::rewrite_with`) | the cross part, its maintained form, and the cut that drops clean strata and adds `__plus`/`__minus` inputs | the cut drops most strata; the maintained form rewrites every relation | one base, three variants |
| E4 | `RERAIL_CROSS_VERIFY` (`corpus.rs:6846`) | the plain and the maintained program, run side by side | the whole maintained form | debug only |
| E5 | the planner's forms (`plan.rs:9015`, the `Forms` arena; `Driver::form` `corpus.rs:7461`) | the cross program, and a one-stratum program for each stratum reached | one stratum | one base, many small variants |
| E6 | `plan::run` (`plan.rs:8260`): normalize, physical, `stated_apart`, lowering, `keeps::choose`, `split_negations`; the chosen form kept in `choices` | each stage's input and output, all held; the chosen one kept for later chunks and runs | normalize rewrites every rule; the others, some relations | a chain, each stage held |
| E7 | `split_memo` (`KEPT = 2`) | the splits before and after a preparation round, compared by per-relation rule digests | a few hundred rules of tens of thousands | linear, old against new |
| E8 | kept strata (`kept_strata.rs`, `plan::identities`) | frames made under the old program, matched to the new by per-relation identity | the relations a rule edit reaches | linear, the old program not held |

And retry loops, which hold a candidate and the original until the candidate is accepted or dropped: `magic_sets`
(`magic.rs:219`, `:424`), `push_local_joins` and `push_with` (`file_local.rs:852`, `:1415`), `factorize_exports`
(`:984`), `export_demand.rs:403`.

So the common case is not a line of history. It is **one base, with several variants alive at once**: E1 to E5,
for every run and every concurrent session. A variant cannot tombstone its base's rows, because the base's other
variants still read them. It cannot copy the base, because that costs the base. So:

- **A shared image is immutable.** An image that more than one version reads (a store's prepared program, a run's
  pool, a split's part) is never changed by a variant. §6's in-place edit is only for an image with one line of
  history and no variant over it.
- **A variant is made in one of three forms, each costing what it changes.**

**A view** is a subset of a base: ranges of its rules, and the relations they need. A view has no rules of its own.

- The views the engine makes are the file-local and cross parts (E1), one stratum's form (E5), and the cut that
  keeps only the dirty strata (E3), wherever they add nothing.
- A view costs its range list, where `clone` then `retain` costs the program today. A base needs no particular
  layout for that: the simulation found the same cost in source order, by head relation, and in an oracle's order
  (question 4).

**An overlay** is a base (or a view) and a small delta. The delta has its own regions, in the same format:

- the rules it adds;
- the base rules it removes, as a sorted `u32` set;
- the relation rows it changes: a rename, an input or declaration added, columns moved.

Reading an overlay walks the base's ranges, skipping the removed set, then the delta's rules. A lookup probes the
delta's index, then the base's, as a two-level table does. So reading costs the base plus the delta, and making one
costs the delta alone.

This covers:

- a run's edits (E2);
- a session's cut and its added inputs (E3);
- the side-by-side verify run (E4);
- each retry loop: the candidate is an overlay, dropped if refused, kept if accepted.

Overlays stack, as a run's delta over the prepared program, and a session's cut over that. A stack is at most a
few deep. When a delta reaches half its base, it is folded into a new image: off the run's path, and paid for
geometrically, as compaction is (§6).

**A derivation** is a new image whose rules all differ from its input's:

- normalize's sentinel columns;
- lowering;
- `keeps::choose`;
- `split_negations`;
- the maintained form's qualifiers (E3).

These are not overlays. They are new images, made once, with `from` (§9) linking each output rule to its sources.
When the input gains an overlay, a local pass maps that overlay alone, and the derivation gains the matching
overlay: pass(base ⊕ Δ) = pass(base) ⊕ pass(Δ).

So the chain from the prepared program through split, run, normalized and lowered (E1, E2, E6) is a chain of
overlays, each costing the change, and none a copy.

**Linear history** (E7, E8, and a stored program edited by a file change) takes §6's edit in place when no variant
is over the image. Otherwise it is an overlay, folded later.

**Digests compose.** Five digests today hash a whole program's `Debug` text, so each costs the program and turns
any edit into a whole miss:

- `memo::program_digest` (`corpus/memo.rs:105`), every run;
- `retain::key` (`retain.rs:133`), up to three times a run;
- `choices::key` (`choices.rs:56`), every `plan::run`, so every chunk;
- `prepared_key` (`corpus.rs:2501`), every run;
- `cross_digest` (`corpus.rs:2841`), every run.

Then there is `program_text::store`'s check, which prints the program, reads it back and compares (`:284`).

In the image:

- A program's digest is a commutative sum of its rules' hashes (each rule's byte hash, already its key in the
  pool, §8) and its relation rows' hashes.
- An overlay's digest is its base's, less what it removes, plus what it adds. It costs the delta.
- A digest by relation, which E7's `split_memo` and E8's identities already compute, is the same sum grouped by
  head relation, kept as a column.
- The store's read-back check runs when an image is built, not when it is edited.

**Concurrent readers** need nothing more. Strands sharing one program, background writes holding an `Arc`, and
sessions all read an immutable base, or a generation of a single-history image.

### 8. A run shares rules by their bytes

Within one image, two equal rules have equal nodes, byte for byte, because every name in them is an id. So a run's
programs can live in one pool image:

- each rule stored once, found by its bytes in the pool's hash table (§4);
- each program a key of a grouped index: program → its rule ids.

This is decision 0172's `Lists` table, found by a hash of a byte range instead of by addresses. A pool can be the
run's (an anonymous mapping, dropped when the run ends) or a store's (a file, kept). There is no process table.

### 9. Passes are edits, or rules, maintained by the edit

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
  relations (§10), and kept up to date by the engine's own delta maintenance (counting, DRed). This is the reason to
  make these passes rules: so they cost the change, not the program, by the same machinery as everything else.

A build from nothing (a cold run, a new pack) runs every pass once over the whole image, as an append of everything.

### 10. The image is the program's relations

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
     - compaction;
     - views, overlays and their stacking and folding (§7);
     - digests that compose (§7).
   - `program_text::store` writes a program's image beside its `.rr` text. The text stays, for people and for the
     converter's tests.
   - `load` maps the image, and builds today's `front::Program` from it only where a caller still needs one.
   - Measured: a warm run's reads.
2. **The warm path, by id and by edit.** The planner takes an image, keyed on relation numbers, slots and node
   positions (`Model`, `JoinSpec`, `Step`; `Run.numbers` by `*const Rule` and the `Forms` arena go).
   Each of E1 to E6 (§7) moves to its form:
   - the split's parts and the planner's per-stratum forms become views;
   - a run's edits and a session's cut become overlays;
   - normalize, physical, `stated_apart` and lowering become derivations, maintained by `from`.
   The five whole-program digests become composed ones.
   - Measured: a warm run's program work, and a one-rule edit's, which should cost the rule.
3. **The prepare passes.** Local passes move one at a time, each an edit, each checked against the tree pass it
   replaces on every pack program (as decision 0162's reader was checked against the statement reader). Non-local
   passes are written as rules (§9, §10).
4. **The store's atoms.** The store's atom table becomes a string table of §4: a mapped file whose header is
   committed through SQLite. A process maps it, and no longer rebuilds a hash table. This touches 0111, 0160 and
   0170's atoms, which other worktrees are changing, so it is planned with them.
5. **The tree goes.** `front::Program` remains only as a debug printer's view, or not at all.

## The open questions, simulated

Every making of one program from another in a dl1 run was traced, with both programs (`RERAIL_VERSION_TRACE=<dir>`,
`logic/version_trace.rs`, hooked at the survey's sites E1 to E6). The trace was then replayed through each form a
program could take, by `crates/logic/examples/versions.rs`. Each form is given the same delta for each making (which
of `from`'s items are kept, and which of `to`'s are new), and the replay measures three things:

- **make:** making `to` from `from`;
- **read:** every node, then each head relation's rules by its index;
- **digest:** the program's digest.

The forms:

| form | what it is |
|---|---|
| tree | today's `front::Program`: `clone`, then `retain` and `push`, as each traced site does |
| flat | one flat image a program: copy it, tombstone, append, rebuild its index |
| pool | a run's rule pool, each program a list of rule ids: copy the list, edit it, rebuild its index |
| v | this record (§7): views, overlays and derivations over the pool, with layouts (source order, by head relation, an oracle that knows every later view), fold policies, and three kinds of removed set |

The trace has 4,593 makings on a cold run (4,458 distinct programs) and 426 on a warm run. Items are rules and rows
of a program's other parts (declarations, facts, inputs, outputs). Averages per making:

| site | cold makings | `from` items | removed | added |
|---|---|---|---|---|
| `plan.form` (E5) | 1,996 | 1,102 | 1,095 | 0 |
| `session.cut` (E3) | 2,049 | 21 | 0 | 0 |
| `split.cross`, `split.local` (E1) | 99 each | 672 | 181, 53 | 21 |
| `run.cross`, `run.local` (E2) | 50 each | 511, 598 | 5, 0 | 2, 0 |
| `plan.normalize` (E6) | 124 | 517 | 1 | 1 |
| `session.maintained` (E3) | 73 | 525 | 29 | 89 |
| `plan.demand`, `plan.inline` (E6) | 43, 10 | 676, 1,068 | 33, 9 | 92, 7 |

In 2,537 of the 4,593 cold makings, and 304 of the 426 warm ones, `from` is made from again later. So changing
`from` in place would be wrong for most makings, which the survey said and the trace confirms. Another 2,211 cold
makings change nothing at all.

Release-equivalent build (`--profile profiling`), whole replay, in millions of instructions:

| form | cold: make | read | warm: make | read | one-rule edit of the prepared program |
|---|---|---|---|---|---|
| tree (today) | 953.7 (279 MB allocated, 2.26 M allocations) | 142.7 | 77.5 | 67.0 | 0.24 |
| flat, copy then edit | 201.4 (404 MB) | 53.1 | 49.4 | 20.5 | 0.41 (1.0 MB) |
| pool, copy then edit | 95.6 (26 MB) | 38.0 | 40.1 | 20.2 | 0.65 (5.2 MB) |
| v: fold at ½, sorted removed set | 75.5 (13 MB, 33 k allocations) | 18.3 | 26.9 | 9.9 | 0.014 |
| v: never fold | 59.5 | 331.3 | 14.5 | 42.6 | |
| v: fold at ¼ | 77.3 | 17.6 | 28.0 | 9.3 | |
| v: fold at ½, at most 2 deep | 77.9 | 16.7 | 28.0 | 9.1 | |

For scale, dl1's process is 50.7 G instructions cold and 23.3 G warm. Today's tree is about 2.2% of a cold run and
0.6% of a warm one; the proposal is 0.2% of either. Of the tree's cold making, 791 M is `plan.form`: it clones a
1,102-item program 1,996 times to keep 6 items each. A view or overlay makes the same thing in 19 M.

Digests: today's `Debug`-text digest, taken once per making, costs 1,589 M cold. That overstates today's real cost,
because the real digests are taken fewer times (`memo::program_digest` once a run, `retain::key` up to three times,
`choices::key` once per `plan::run`, so 124 times cold). Each one is still a walk of the whole program. The composed
digest (§7) costs 0.1 to 0.4 M for the whole replay.

So:

1. **Copy then edit loses, even in a flat form.** A flat image copied per making costs less than the tree
   (201 M against 954 M), but it allocates more (404 MB, against 279 MB), because a copy of flat columns copies
   everything. The tree, after decision 0172, shares its lists, so its copy is mostly counts. The flat form's gain
   comes from views and overlays, not from flatness alone.

2. **Folding (question 5): fold at half, at any depth.**
   - Never folding makes the cheapest versions (59.5 M) and the dearest reads (331 M). `plan.form`'s stacks reach
     reads of 265 M alone.
   - Folding at a quarter, at a half, or by a cap of two deep all land within 5% of each other.
   - At half: 75.5 M to make, 18.3 M to read.
   - A depth cap adds folds (the session cut doubles, 2.0 M to 6.0 M warm) and buys no reads.

   So: fold when the delta reaches half its parent, with no depth cap. A run's overlays are folded the same way.
   Whether to fold is a question of reads, not of lifetime.

3. **Removed sets (question 3).** For an overlay's removed set:
   - sorted `u32`s: 75.5 M to make;
   - a hash set: 94.0 M;
   - a bitmap over the pool's ids: 57.4 M.

   The bitmap is the cheapest at dl1's size, but its size is the pool's, not the edit's: about 4 KB a making here,
   zeroed each time, and 64 KB at sixteen times the pool. The sorted set's cost is its edit's, so the sorted set is
   decided.

   In-place tombstones (a `u32` generation a row) remain only for images with one line of history. The trace has
   2,056 such makings, and each costs below the measurement's resolution.

4. **Rule order (question 4): nothing depends on it.**
   - **Findings.** `RERAIL_RULE_ORDER=reverse|shuffle|head` reorders the prepared program's rules before the split. dl1
     gave the same findings under all three, cold, on fresh stores. Instructions were 50.7, 51.7 and 51.5 G, against
     50.7 G as written (load 70 to 97).
   - **Code.** The positional uses of rules in the code number them as ids (`numbers`, `rule_no`, `plan_key`'s
     inputs). None chooses by position.
   - **Cost.** Layout makes no difference: source order, by head and the oracle are within 2% on every site. Views
     are chosen for 124 of 426 warm makings; the rest add rows (exports, inputs), so they are overlays whatever the
     order.

   So §7's requirement that the base be laid out by (part, stratum) is dropped. A base keeps its rules in any order.

5. **Extents (question 1): extents.** The pool's growth over a cold run is 266 k nodes in 259 appends:

   | | append | allocated | scan | 1 M random reads |
   |---|---|---|---|---|
   | one array, grown by realloc | 2.35 M | 12.6 MB (it moves) | 0.37 M | 8.0 M |
   | doubling extents | 0.69 M | 6.2 MB (7 extents, never moved) | 0.37 M | 22.0 M |
   | reserved in advance | 2.02 M | 3.2 MB | | |

   Extents append for less than either, never move, and scan the same. Random access costs 2.75 times as much:
   about 14 instructions a read, to find the extent. So:
   - a reader walks by extent;
   - a built, compacted or folded image is one extent a region, so its index lookups are plain array reads;
   - reserving in advance would need the final size, which no append knows.

6. **Where the bytes live (question 2): a file beside the database.** Opening an image and reading 64 nodes:

   | image | SQLite blob (read by copying) | mapped file |
   |---|---|---|
   | the largest program, 0.3 MB | 0.47–0.52 M, 0.3 MB copied | 0.27–0.29 M |
   | the run's pool, 3.2 MB | 4.8 M, 3.2 MB copied | 0.6 M |
   | sixteen pools, 51 MB | 75.5–76.4 M, 51 MB copied | 0.9 M |

   A blob costs its whole size at every open. A mapping costs the pages read. The header stays in SQLite (§6).

### Left open

- The replay reads each version once. A real run reads some versions many times (the planner reads a form for each
  plan it prices), which favours the forms with cheaper reads (v, folded) further. The simulation does not count
  that.
- The tree's added rules are rebuilt with new lists, as a pass makes them. The cost of a pass's own work (deciding
  what to keep, computing new rules) is the same for every form and is not measured.

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
  evaluators (§9, §10).
- **The survey of `front::Program`'s passes for this record:** the five kinds of rewrite in §9, the warm path's
  copies in point 4 of "What was measured", and the planner's address-keyed structures in stage 2.
