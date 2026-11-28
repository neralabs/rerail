# 0666. A made text is looked up as an atom on the device

Status: done (2026-11-24).

## Intro

`ann_private` makes a text and then uses it as a key of an atom: `x = @str_between(q, "", ".setter")`, then
`def_name(f, x)`. Decision 0661 gave the device a form for it. The atom is probed by its other keys (`f`), and each
row of that group is compared with the made text, as texts (`TEXTOP`). Two things were left as they were:

- The group scan reads every row of `f`'s group, where a probe by value reads the one row.
- The CPU drops a row at the function when its text is no atom yet (`PROBE_OUT`: no row can hold it). The device
  dropped it at the atom. Their rows agreed, but their counts agreed only from the atom on, and a test checked them
  only from there.

The user, 2026-11-23, asked for both to be fixed before going on: a hash lookup in the atoms store on the device, with
the store's hash ported to the GPU and names interned during the run handled; and the count check strict again.

## What was built

### The atoms find, written once in CLIF

Each atoms block (decision 0401) has a table from hash to id. A slot holds `(h & 0xFFFF_FFFF_0000_0000) | (id + 1)`,
probed from `h & mask` to the first empty slot. The directory of runs (`AtomsDir`, decision 0478 step 2b) now lists
each run's table and its slot count beside its ends and strings (`DirEntry::table`).

`atoms_find(dir, h, len, k)` is a new CLIF read (`clif::atoms_reads`):

- It goes through the runs in order and probes each run's table from `h`.
- A candidate is a slot with `h`'s high half, an id of the run's own (`first..end`), and a string of `len` bytes.
- It gives the `k`-th candidate, or `u32::MAX` past the last.

The caller compares the candidate's bytes with its own and asks for the next on a difference. The bytes lie where
the caller reads them: for a kernel, its arena is another buffer than the directory. The host's `atoms::find_in` is
the same search in Rust. The JIT's read is compared with it, a block with every atom under one hash included
(`the_jits_bloom_and_atoms_reads_are_the_hosts`).

### The store's hash on the device

The tables are keyed by the store's hash: `content_store::atoms::hash_str`, which is foldhash 0.2's quality hasher
with fixed state and seed `0x5EED71`, over a `str` (its bytes, then `0xff`). `store_hash` in `join_stages.uglykl`
is that function for one thread:

- the per-hasher seed, rotated by the length;
- the short path up to 16 bytes, and the long path's 16-, 32-, 64- and 96-byte tails;
- the 4-lane loop past 128 bytes and the 6-lane loop past 256;
- the `0xff` folded in, then the quality hasher's last fold.

The folded multiply is the 128-bit product's halves xored (`ukl_umulhi64` for the high half, on every backend). Loads
are assembled little-endian from bytes, as the arena's texts are not aligned.

### The find in a made text's stage

A text function or a cat whose value is bound and read only as positive atoms' keys (the CPU's `PROBE_OUT`) has
`ROW_FIND` (bit 10 of the stage's `agg[0]`):

- The kernel makes the text into the arena as before.
- `atom_of_text` then hashes it (lane 0), finds its candidates (`clif_atoms_find`), and compares each one's bytes, a
  byte a lane.
- A hit gives the atom's id as the slot's value: a symbol, not an arena text. A miss gives no row (`counts[row] = 0`).
  The empty text is atom 0.

The compile (`setfix::device`) keeps that slot out of the arena's slots. Its atom is then an ordinary probe by value:
it may be a witness stage (0277), and it needs no `TEXTOP`. A made text read otherwise (compared, in a head, read by
another function) stays an arena text, and 0661's comparison still serves it.

### Names interned during the run

The directory lists every atom the process holds: the store's, and the pending atoms, which are atoms blocks with
tables of their own in anonymous memory (decision 0478 step 2a). It is made again when the atoms' end grows
(`dev_atoms`). A relation a stratum reads is complete before the stratum's batch, so every atom one of its rows holds
is in the directory. A miss is final.

### The CPU's rule, made exact

The CPU interns a text at the function when it is read later as a symbol (a head's, say). A cat of that text found it
held, though no row of a relation the walk reads can hold an atom the walk made. So the CPU passed the row to the atom,
which dropped it; the device, whose directory has no atom the walk made, dropped it at the function.

A walk that makes atoms defers them: during the walk each such text is a `DEFERRED` value (`Interner::id`, decision
0315). The CPU's `PROBE_OUT` lookups (a function's text, a cat, a split's parts) now take a deferred value as no atom
(`Interner::before_walk`). Both drop the row at the function, and every step's count agrees.

### The structured C of a nested loop's exit

The device test hung on the CPU's device (a Linux host, `RERAIL_GPU=cpu`) inside `clif_atoms_find`. The JIT's read was
right. The C that `clif::ukl` writes was not:

- The inner loop over a run's slots has two exits: the next run (the outer loop's header) and the end (the outer
  loop's exit).
- The text after the inner loop took the end as reached by falling through, as for a loop at the top level.
- Inside an outer loop, falling through goes back to the outer loop's header, so the search began again forever.

An inner loop's exit that is the enclosing loop's header or exit now gets that loop's `continue` or `break` (`leave`).
An exit to a loop further out gets the switch form, as a branch to one already did. Of every read header, only
`atoms_find` changes.

## Found on the way

The 10-file django run on the device (M1 Max) ended about one run in four with "sort_init: a launch outside any
evaluation", a hard failure under decision 0601. A batch of joins the GPU lane begins (`GpuBackend::start`) and reads
later (`finish`) opened no evaluation, where `run` does. It ran inside the caller's evaluation when the caller had one.
On the lanes' pool, whose threads have none, the radix sort's first launch failed. Each half now opens one, as `run`
does (`a_started_batch_of_joins_is_charged_on_a_thread_with_no_evaluation`).

## Tests

- `text_functions_on_the_device_are_the_cpus`:
  - Its made-key case compares every step's count again.
  - It gains a rule that finds every name of the relation through a cat with "" (the empty name too).
  - It gains names of 40 to 200 parts, past each of the hash's paths.
  - It passes on Metal (M1 Max) and on the CPU's device (a Linux host). The 55 tests of `setfix::device` and
    `metal_join` pass on both.
- `the_jits_bloom_and_atoms_reads_are_the_hosts`: the find's candidates against the host's `find_in`.
- The atoms directory's test: ids found and not, the empty string, an absent string, a wrong hash.
- `rerail-logic`'s quick tier passes, but for `logical_line_predicates_and_aliases_use_the_vocabulary_batch`. That test
  checks the text passes' vocabulary, which this change does not touch, and was red before it.
- The 10-file django run on the device, repeated on the M1 Max:
  - Every run that ended has the CPU's findings.
  - `ann_private` runs on the device, 3 rounds, never handed back.
