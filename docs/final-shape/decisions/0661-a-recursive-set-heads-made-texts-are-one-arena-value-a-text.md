# 0661. A recursive set head's made texts are one arena value a text

Status: in progress (2026-11-23). Five of the eight strata run on the device; three need the forms listed at the end.

## Intro

Eight recursive strata of the python pack's 10-file django run stayed on the CPU: the device had no form for them.
Seven make a text each round and keep it in a set head's column: a tree climb that builds a dotted name
(`qualname(f, @dot(p, x)) :- …, qualname(s, p)`). One, `literal`, makes a text by a string join.

Decision 0307 let a recursive component's made texts live in one arena shared by its rounds, but only as a
`shortlex_min` lattice's value. A set head compares its rows by their words: in its dedup, its indexes and its
anti-join against its relation. Two equal texts made in different rounds had two arena offsets, so the device would
keep a row the CPU keeps once, twice. 0307 had declined interning on the device each round.

At 10 files none of these strata does enough work a round to be worth the device (0657 hands such a stratum back),
and the first answer was to leave them on the CPU. The user, 2026-11-23: "if they are scheduled on the CPU, then okay
but it needs to be possible to run them on the GPU. With GPUs attached via slow busses and their own memory, much more
will run on the GPU because its data is already there. We will need to be able to run these. There are already
solutions for allocation of memory on GPUs to hold strings."

So placement may still put them on the CPU, but the device has a form for them.

## What was built

### Each made text is one value: a canonical arena

A component whose set heads keep made texts gets a canonical arena (`mj::Arena::canonical`). Beside its bytes it has a
table of its texts (`Canon`): an open-addressed hash table of 32-bit places in a reserved address space, mapped as it
fills, as the arena's bytes are. Its header is `[places - 1, texts, full, 0]`. A place holds a text's offset plus one.
It is 32 bits because Metal has no 64-bit compare-and-swap.

A canonical text is written with its length first: four bytes, little-endian, then its bytes. A kernel that made a
text (`row_text_one`, both of its arena writers) calls `canon_text`:

- The SIMD group's lanes have written the bytes. A barrier and a device fence come next, so another group that finds
  the place reads the bytes.
- Lane 0 hashes the bytes with FNV-1a, 32 bits (the host's `canon_hash` is the same function).
- The group probes the table from the hash. At each place, lane 0 compare-and-swaps 0 for its own offset:
  - An empty place takes the text. Its value is its own.
  - A taken place is compared: the length first, then the bytes, a lane a byte, read coherently. An equal text gives
    that text's value. The bytes just written stay in the arena, unnamed.
- After 64 probes the table counts as full. The kernel sets the table's full word and the program's flag 64, and the
  value is -1. The phase parks, as for a full arena (0603 step 5). `Arena::grow` then places the table again at four
  times the places (also when it is half full), and the phase's texts run again. A text made before the park finds
  its own earlier value.

The host's `Arena::put` (a constant's text) inserts the same way. The table is bound as a row stage's `H` (`desc.h`),
and `P[6]` says the arena is canonical. A launch re-points the descriptors when the table's wrap moved
(`Kit::readdress`, as `retext` did for the arena alone), and makes the table resident.

So equal texts are equal words. A set head's dedup, its indexes and its anti-join, which compare words, compare
texts.

### Which columns hold made texts

`setfix::open` finds a recursive component's text columns before it compiles its rules for the device:

- each rule's head as written (`made_text_columns`): a `cat` term, or a slot that a text function, `cat` or
  `str_concat` binds;
- then the compiled plans' arena columns (`Plan::made_columns`), again until no new column is found. A rule that
  copies a text column's value into another head's column makes that column a text column too.

`compile` takes the head relation (`SolverDevice::compile_join`'s new `head`). In a text column a name is copied into
the arena (`TEXTFN_COPY` of a symbol or a constant), so every value of the column is an arena text. Equal texts are
then one value however they came: made, copied or constant.

### The component's rounds and its end

- `fixpoint_here` gives the component a canonical arena when any set head keeps made texts. Each rule must make or
  name a text in each such column. A lattice still takes made texts in its `shortlex_min` value only.
- At the component's end the set heads' text columns are interned at once, where the rows lie (`intern_texts`,
  shared with the lattice's value). The CPU then reads symbols, including after a hand-back (0657).
- A head with a text column has no reach bound: a text can grow round after round. Before, such a head's room per
  program was twice its rows, and each round past it stopped the program and encoded another. A head with no reach
  now reserves `REACH_BYTES` of rows at least, mapped as its rounds reach them, as a bounded head's is.

### Forms the strata needed besides

- A `cat` in the head of a made text (an arena text or a held cat) is made by a row stage (`ROW_CAT`). Before, only
  a cat of names was. A text function's text may now be a head cat's part.
- A cat of a made text compared with a bound slot is made into a slot of its own (past the rule's slots) and compared
  as texts (`TEXTOP`).
- Two new device text functions: `str_between(x, a, b)` (`TEXTFN_STR_BETWEEN`: no row where `x` lacks the prefix or
  the suffix) and `pkg_up(x, k)` (`TEXTFN_PKG_UP`). Both are in the one table (`device::caps`).
- Placement no longer marks a made text in a recursive stratum's head or comparison as a computed term. A cat of made
  texts in a head (`is_made_cat`) is row code.

## Tests

- `a_recursive_set_heads_made_texts_are_one_value_a_text` (setfix::device):
  - A tree's dotted names are climbed from its root: a name copied in, then one cat a round.
  - The names are copied along alias edges that reach a node in two rounds.
  - A constant goes into the same column.
  - The device's rows are the CPU's, each once. The canonical table grows (`remaps`).
  - With the kernel told the arena is not canonical, it fails.
- `text_functions_on_the_device_are_the_cpus` gained `str_between`, `pkg_up`, a cat of a made text compared with a
  bound name, and a head's cat of a text function's value.
- The 44 device tests pass on Metal (M3) and on the CPU's device (a Linux host, `RERAIL_GPU=cpu`). The planner's 108
  tests pass.

## The real run

The 10-file django run on M3 was run with `RERAIL_DEVICE=always` and `RERAIL_DEVICE_NARROW=0`, so that every stratum
the device can run goes to it and stays there. Its findings are the CPU run's (CSV SAME, 2,108 rows).

| Stratum | Before | Now | GPU time | Rounds |
|---|---|---|---|---|
| `qualname` | CPU | device | 10.4 ms | 4 |
| `dotted_expr` | CPU | device | 7.2 ms | 4 |
| `qual_part` | CPU | device | 12.5 ms | 4 |
| `qn` | CPU | device | 8.9 ms | 4 |
| `class_chain` | CPU | device | 9.0 ms | 2 |
| `ann_private` | CPU | CPU: refused at compile | | |
| `literal` | CPU | CPU: string join | | |
| `ask_c_found_by_bbf…` | CPU | CPU: computed terms | | |

Under the default binding, placement still puts these strata on the CPU at this scale (0657's threshold). That is
the intended outcome: they can run on the device; at 10 files they should not.

## Still to build

- **`ann_private`: a made text as a lookup key.** The planner inverts `q = cat(x, ".setter")` into
  `x = str_between(q, …)` and looks `def_name` up by `x`. The device refuses a made text read by an atom. The made
  text must be found as a symbol. The atoms blocks hold a table from hash to id (decision 0478), so the lookup can read
  them in place.
- **`literal`: a string join on the device.** For each binding, the matching rows are joined in their order, with a
  separator, into the arena.
- **The `ask_c` component**: `@rx_group` (a regex's capture group, which the device's automata do not give), and
  `@concat` and `@substr` in comparisons.
