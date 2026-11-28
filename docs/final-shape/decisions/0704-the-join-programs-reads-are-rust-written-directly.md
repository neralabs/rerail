# 0704: The join program's reads are Rust written directly, not the JIT's compiled CLIF

Status: decided (2026-11-25), by the user: "please use Rust, not a JIT, for the reads". Supersedes decision 0567's
"The reads called, not written again" and the part of its oracle section that gives `join_run` the compiled reads
(`Launch::reads`, `rerail_ukl_rust::Reads`). Follows 0534 (the tape's reads in Rust) and 0353 (blocks read in
place).

## What was there

The CPU's join program (`rerail_ukl_rust::join_run`) read every view row, walk, relation, atom, verdict and cached
text by calling the JIT's compiled CLIF reads (decision 0310) through a table of function pointers handed in with
each launch (`Launch::reads`, in `rerail_clif::reads::READS`' order). The CPU device made that table for any
library whose kernels call it (`calls_reads`), from `rerail_clif::jit::Reads`.

The table was my choice, not the user's. Decision 0567 recorded it under the user's size bar for rs.rs's generated
Rust ("under 200kb"), as if the indirection followed from that bar. It did not: the bar was the requirement; the
table was one way I chose to meet it, and then carried into the hand-written oracle as well.

A profile of the CPU joins showed what that costs: one indirect call per row and column into code that cannot be
inlined (`vread_dir` → `view_child` → `num_select`), the bulk of the program's time on the view-reading strata.
The user then asked for the reads in Rust.

## The decision

1. **The join program's reads are Rust, written directly**, each one following its CLIF read and held equal to it
   by a test over real batches' directories. The CLIF reads stay for their other readers: the JIT's executor, the
   Metal kernels and SQLite's virtual tables.
2. **One crate holds the layouts every reader reads in place, with their host reads**: `rerail-reads`, which has no
   dependency of its own. In it:
   - `view`, the tape's view and the rolling cache, with its host lookup (`cache::lookup`);
   - `ids`, a projection's numbering;
   - `views`, the views' entries and directory with its searches (`doc_of_row`, `doc_of_key`);
   - `tape`, the tape's reads (0534's, generic over where the bytes lie: a slice, checked, or `InPlace`), with the
     16z reads through the directory's shared cache, the very cache and protocol the JIT's reads use.

   rerail-clif and rerail-tape-format re-export these where they were. The join program could not depend on
   either: the workspace builds clif with its IR, which depends on rerail-blocks, which depends on the device crate
   that runs the join program.
3. **The table is gone**: `Launch::reads`, `rerail_ukl_rust::Reads` and `calls_reads`; the CPU device's wiring of
   them (`with_rust`'s reads, `with_host`'s, `Run::Host`'s); rerail-gpu's `compiled_reads` and its dependency on
   rerail-clif; and rs.rs's compiled-read path (`Read`, `call_compiled`), which no caller gave any reads.
4. **Then the joiner itself**: a row's tape node found once and all its columns read from it, and the index builds
   and scans lane-wise, each step measured against the scalar joins.

## As built (2026-11-26)

- `join_run::reads`: `Doc::of` finds a document once (and, for 16z, the host's view of its cache), and every view
  row's columns (node, child, field, core, line, the four `Loc` views, tape, depth, order), the walks (child_first,
  child_next, up, end, number) and the atoms' reads.
- `lanes`: the program's scalar reads of relations, each its lane read at one lane (`lanes::one`), and the rows'
  length, bounded write and an index's marks.
- The verdicts and the cache's texts: `rerail_reads::view::cache`'s reads, as the JIT's.
- Tests: `the_join_programs_view_reads_are_the_jits` (every column of every view row and every walk of a batch's
  directory, formats 16 and 16z, against the JIT), `the_16z_rust_reads_are_the_jits` (8 threads sharing one
  cache, Rust and JIT reads mixed), `the_scalar_reads_are_the_compiled_reads` and
  `the_scalar_reads_layouts_are_the_blocks`.
