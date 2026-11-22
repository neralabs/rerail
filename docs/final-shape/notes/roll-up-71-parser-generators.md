# Roll-up 71: Parser-generators-final-shape (62187a8c9a) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 6238759cd1. Theirs: Parser-generators-final-shape at 62187a8c9a. Git's base is
virtual: the merge bases are d558cc0919 (theirs' pin in roll-up 55) and 15a575d133 (the hub, which theirs merged in
f1793ae112). Theirs brings 29 commits of its own (decisions 0602 and 0604, page 40: JSON in simdjson's structure):
- the six differences after the twelve fixes: JSON's string and token masks in `index`'s terms, a value's
  primitives as one method, and tier 2 storing its nodes through a cursor it holds, as `walk` does;
- the generated Rust in the lean parser's shape:
  - tier 1's records are `index`'s `structural_start`, through one write a block behind;
  - the toggle is `in_string`, and a string open at the end is its carry;
  - tier 2's peek is the record's byte, and its returns are one loop;
  - strings, `\u` pairs, numbers and checks are read as `escaped_string`, `number_end` and `walk` read them;
  - the root's calls test no depth;
- JSON keeps accepting numbers past simdjson's bounds (the user's choice); the red 16z device-read test is 0540's
  recorded M3 failure;
- `json_lean`: the three JSON parsers phase by phase, and the simdjson form's JIT and generated Rust measured the
  same way.

## Items

1. **Theirs' merge of the hub is read as a merge, and stays out.** Its inputs are in ours: d558cc0919 by roll-up 55,
   and the hub. Its own text puts theirs' removal of the Rust lowerings (0565, 0569) on the hub's crate split. That
   means `native_cpu_stream` over the hub's `stream_facts` and `stream_analysis`, `native_cpu_facts` removed, and the
   crate roots and 0565's text to match. Roll-up 55 (Q476, item 2) chose the other layout: with the Rust lowering
   gone, the JIT is the facts' only reader, and they are its own (`native_cpu_facts`). Ours keeps that layout.
   - So all 75 files theirs changed only through merges take ours. That includes `native_cpu_facts.rs` and
     `native_cpu_tape.rs`.
   - The same goes for theirs' renumbered decisions. Theirs' merge named its two decisions 0605 and 0606; ours names
     them 0562 and 0569 (roll-up 55). Theirs' 0559 is ours' 0579, and ours' 0577 and 0578 stand. Theirs' 0605, 0606
     and 0559 files are not taken.
2. **Theirs' own commits on ours' files** (`/tmp/obs/byinvariant.sh`). All applied but five hunks:
   - `slabs.rs` (07eeaa34ef): the trace of a refused slab is already ours. It is roll-up 66's one `RERAIL_POOL_DEBUG`
     print, the same print theirs' merge chose.
   - The decisions index's rows for 0602 and 0604: theirs' last text of each.
3. **The decisions index had no rows for 0576, 0597, 0601 or 0603**, and no feeder's index has them. Each gets a row
   from its title and status, in number order with 0602 and 0604.
4. **The generated JSON Rust** (`json_aot`'s tiers) is theirs' generator's output. `json_rust_is_current` checks it
   against the merged generator.

## Audit

`scripts/check/merge_audit.py` reports 721 findings:
- 716 on theirs' side: its hub merge's text, which item 1 leaves out. That covers `native_cpu_stream.rs` (655), the
  crate roots of rerail-rr-parser, rerail-cpu-stream and rerail-compiler, 0565's text, the 0605 and 0606 files,
  `AGENTS.md`, the members, and item 2's index rows.
- 5 on ours' side: short lines theirs' own commits wrote again in new code, for example `one_loop.rs`' returns loop
  (ac5098583f).

## Checks

Locally under Relax, CPU:
- rerail-parser-tests' `json_rust_is_current` passes.
- rerail-tape-format's 42 tests pass (`f16.rs`, `writer16.rs`).
- rerail-json-lean does not build under Relax: its `stage1.rs`, which this merge does not change, calls the NEON
  intrinsic `vqtbx1q_u8`, which Relax's std lacks. Its tests were not run.
- The crate DAG, global-state and thread-state checks pass.
