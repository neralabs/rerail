# 0546: The end is the stream's count, tested once

Status: done by the implementer. The item is the user's (2026-11-20, "address each of these in turn", the seventh):
"The end of input is tested at every token (s.start == s.len). The lean parser checks once, before the walk, that
the outer container's closer is the last token." How it is done is the implementer's choice. Follows 0545. Page 40,
step 50.

## What was there

- After 0543 no token tested the end any more: `token_all` is one load and one add.
- The end was tested after the root's value, at each of its two places (a string root, and the rest):
  - a `TOKEN` took one more token, past the count, whose start is the stream's end entry (the file's length);
  - a byte switch with no byte arm tested `t.start == s.len`;
  - its end arm went to `ACCEPT`, which tested `t.start == self.len` again.

## The decision

1. **Found from the IR** (`rust_stream::counted_end`). A switch qualifies where all of these hold:
   - the run is a sentinel run (0543), records fetched, not flagged, and the last end needs no copy (`last_ends`);
   - it is a fresh byte switch of the records with an end arm, and no byte arm of its own or the other switch's
     (`fetched_arms`);
   - every word on it comes right after a plain `TOKEN` that takes.
2. **There the `TOKEN` takes nothing, and the switch tests `t.next as usize == s.n`**: every token taken. Else it
   refuses (`why::BYTE`), as the byte switch did for a token it has no arm for.
3. **`ACCEPT` tests no end where every way into it is such a switch's end arm** with no take after it
   (`accept_at_end`). Then only the open nodes are checked there.
4. **Not done: the lean parser's check before the walk** (that the outer closer is the last token). Here the end is
   tested once, after the walk, as the lean parser's `next == index + count` after its outer close. A check before
   the walk would add one test and take none away.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`:
  - the format 16 source has no `t.start == s.len`, `t.start == self.len` or `t.at_end`;
  - it has `if (t.next as usize == s.n) {`, and `accept` begins with the open nodes' test alone;
  - new refusals, the Rust's against the JIT's: `[1] 2`, `{} {}`, `"a" "b"`, `1 2`, `[]]`, `{}}`;
  - the truncated files (0543) are still refused, and the tapes equal the JIT's, as before.
- The parser tests: 85, all but the benches, Metal and the corpus.
- `rerail-compiler`'s tests: 128.
- `rerail-rr-aot` (generated, on a farm host): 15 pass, 6 ignored.

## Found on the way

In the full parser test run, `aot_tests::grammars_rust_tapes_are_the_jits` panicked twice in Cranelift's `Arm64Call`
relocation. The cause was the writer's ops: they called each other as direct branches in tier 2's module, whose code
is not in one arena without `Cut::near`. `define_ops` now takes `near` (commit f50a52bd99, and its caller in
5027687aa4). A test lays the ops 320 MB apart: it is red with direct calls, and green now.

## Not checked

- Whether some way reaches such a switch with tokens already taken past the count. The old form then accepted on
  the end entry, and this one refuses. No test file does this.
- Which other grammars qualify: their sources were not read.
- Nothing about speed was measured.
