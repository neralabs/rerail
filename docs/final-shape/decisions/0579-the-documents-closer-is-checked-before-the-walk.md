# 0579: The document's closer is checked before the walk

Numbered 0549 where it was written (this branch); renumbered at the merge of the hub, which holds its own 0549.

Status: done by the implementer. The item is the user's (2026-11-21, "please do these", the fourth): "a test before
the walk". How it is done is the implementer's choice. Follows 0546. Page 40, step 53.

## Correction to 0546

0546 says the lean parser checks the end once, after its walk. It checks it twice:

- before the walk (`json_lean/src/stage2.rs:591`): the outer object or array must be closed by the last token;
- after the walk (`stage2.rs:770`): `next == index + count`.

After 0546 ours had the second check only. This record adds the first.

## The decision

1. **Found from the IR, not from JSON's bytes** (`rust_stream::last_closers`). The `OPEN` of a document's own call
   (`root_number`'s calls) qualifies where both of these hold:
   - In its rule, every way into a `CLOSE` before a `RETURN` comes from an arm of a byte switch on one byte, the
     closer, with no token taken between. A taken token, a call and an empty container's closer all count as takes.
     A word's arm has the bytes of the one-byte tokens that are that word. A switch not on the records, or an arm
     with no byte, disqualifies the rule.
   - Its `RETURN`'s place goes on to a counted end (`counted_end`, 0546) with no token taken.

   Then the walk accepts only where that closer is the token at `n - 1`.
2. **In that `OPEN`'s arm, before anything else, the last token is checked**:
   `if s.n == 0 || s.byte(starts[(s.n - 1) * rs]) != closer { return refuse(why::BYTE); }`. In JSON this is `}` in the
   `{` arm and `]` in the `[` arm.
3. **Nothing is taken away for it.** The end's test after the walk (0546) stays, as in the lean parser: `[1] ]`
   passes the check, and the count refuses it.

## Checks

- `json_tests::simdjson_forms_rust_trees_are_the_jits`:
  - the check appears once for `}` and once for `]`, and each opener's arm still peeks for its empty closer next
    (0545);
  - new refusals, the Rust's against the JIT's: `[1}`, `{"a":1]`, `[{}`, `{"a":[]`, `[1] ]`, `{} }`, `[1,2]}`;
  - the tapes equal the JIT's, as before.
- With each closer's byte changed by one (once, then put back), the test fails at its first file, a valid one,
  refused by the check.
- `aot_tests::grammars_rust_tapes_are_the_jits`: every grammar's Rust writes the JIT's tapes.
- The parser tests: 85 pass, all but the benches, Metal and the corpus.
- `rerail-compiler`'s tests: 128.
- `rerail-rr-aot` (generated, on a farm host): 15 pass, 6 ignored.

## Not checked

- Which other grammars qualify: their sources were not read.
- That `kind_target` gives every byte's arm, the multi-byte tokens' first bytes among them. The analysis relies on
  it, as 0545's `empties` does.
- Nothing about speed was measured.
