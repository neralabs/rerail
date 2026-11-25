# 0655: `.rerail`'s generated Rust in the lean parser's shape: the gaps left after 0623

Status: done (the user, 2026-11-23: "please implement all of these, evaluating ONLY by how much closer they
make the tier 1 and 2 code to the lean Rust code"). Follows 0623 and 0650.

## Intro

Decision 0623 put the generated Rust (`rr_aot`'s `tier1.rs` and `tier2.rs`) beside the lean parser
(`rr_syntax/src/lean.rs`) piece by piece. Read whole after 0650, the two still differ in six places. These are
structural differences, not tuning: each is a shape the lean parser has and the generated code lacks. Each step
below is judged only by that: is the generated code now closer to lean's? Timing decides nothing.

The tier 2 loop form (`tier2_loop.rs`, decision 0618) is not part of this decision.

## The gaps

Tier 2:

1. **A choice of tokens is one arm per token.** `any`, `any except` and the punctuation choices became a `match
   self.word()` with one arm, and one take, per token. Lean takes such a class with one test: `raw_line` is a loop on
   `kind() != BAD`, and `is_p` tests a punctuation code.
2. **The grammar's near-copies.** `unsigned` and `lattice_unsigned`, `literal` and `aggregate_literal`, `negated` and
   `aggregate_negated`, `punctuation` and `setting_punctuation` are each two rules that differ in a token or a call.
   Each is generated twice, where lean has one function with a parameter.

Tier 1:

3. **A checker in the product file.** Under `test-hooks`, `tier1.rs` holds a whole-file mask pass (`stage1`), a walk
   of the regions, and an assert at each region's opener. Lean's lexer holds no checker: its tests live apart.
4. **The token loop.** Lean makes a block's starts at once (not space, not inside a word, not a pair's second byte).
   It takes the common tokens, words and operators, before the next rare byte in a run: one `TOKEN` load and the
   word's run. Only the rare bytes reach its `match`. The generated loop starts at each byte past the last token, and
   every token goes through the rows' `match`.
5. **The block's masks.** Lean's `Window` has four masks: `space`, `word`, `rare`, `pair`. Its arms test the byte
   after a `/` or a backtick where they read it. The generated loop makes eleven classes and a dozen masks a block,
   whether or not a token reads them.
6. **The marks.** Tier 1 works out each token's marks (`hits`, `pending`, `tested`, and `float_after`, `line`,
   `newline`, `json_bad`). Lean's lexer has none: its parser asks where it reads one (`ends_a_line`).

## Step 1: a choice of tokens is one arm per leaf (done)

The tier 2 writer (`bitlex_rust`) has a pass over the rules' code, `one_arm_a_take`, after the takes get their tests
(`takes`). In a `match self.word()`:

- An arm whose take tests the very token its pattern matched takes it untested. `w::COMMA => word_leaf(w::COMMA, K)`
  becomes `tk(true, K, 0)`. A kind's guard, `_ if self.at_string() => tk(self.at_string(), K, 0)`, becomes the same.
  `merge` and `skip` take the same way.
- The word arms whose code is then the same are one arm, `w::A | w::B | …`, at the first one's place: the words are
  distinct, so their order is free. Consecutive guards of the same code are one guard, `_ if a || b`.

So `raw_line` has three arms where it had one per token: the words, the two words a mark may begin (whose take asks
`open_mark`, as `takes` decides), and the token kinds. The decision is the lean parser's class test, as a `match`.
`io_parameters`, `io_inner`, `punctuation` and the rest shrink the same way. JSON's and Kotlin's tier 2 too, written
by the same pass.

All tests pass: rr, JSON, Kotlin, clippy.

## Step 2, first part: the lattice's value and the punctuation are one rule each (done)

- **A lattice's value is lean's `lattice_operation`.** The engine reads `@` and a name with no `(` after it as a
  functor named, not called, and any other value as a term. The grammar's `lattice_term`, `lattice_product` and
  `lattice_unsigned` were a second copy of the term for its first primary. Now a token mark, `bare_functor` (`tokens "@"
  identifier ~"("`, asked by tier 2 as the engine peeks), picks `lattice_functor` (`@` and the name), else the value is
  `term`. This also reads as the engine where the copy did not: `@f * 2` was a product there, and the engine refuses it
  (its statement recovered now).
- **`punctuation` is one rule.** A setting's `setting_punctuation` was `punctuation` without `{`, which opens its JSON
  object. Now `punctuation` has no `{`, and the two declaration tails that take one say so: `(modifier: word |
  punctuation | '{')*`, as lean's loops test for the `{` where they read it.

Tests: `lattice_values_are_the_engines` holds each executor to the engine on lattices (a named functor, a call, a
term, a comment after the name, a named argument) and to lean where the engine refuses (`@f * 2`); the generated
Rust's tapes on the same cases.

## Step 2, second part: a literal and a negation's operand are one rule (done)

Lean has one `literal`: `!` reads a literal again, and the caller sets the field of the node it returns
(`self.literal()?; self.field(Field::Operand)`, or `Field::Literal` in a conjunction). It refuses a comparison
under `!` with the kind of the literal it read. The grammar had a second copy of the literal for each place: `negated`,
`negated_term` and `aggregate_negated`, each with no comparison and no field of its own.

- **`decide`**, a new Bitlex word (`relabel` to `DECIDED`): the node made last at the level has no field of its own
  now. Its call's field is then set where the call returns, where the node has none: the machine's `NODE_FIELD m=4`
  (the generated Rust's `field_made`). A literal holds `bare` while it may still be a term, as before. Where it turns
  out a literal, it is decided, where before it was relabelled `literal`. The machine writes it as `NODE_FIELD m=2` of
  no field, which the verifier now takes. The generated Rust writes it as `relabel_field(0)`.
- **The callers say the field**, as lean's: `literal: without negating literal` in a conjunction, and `operand: with
  negating literal` after a negation's `!`. Under the flag `negating` a comparison has no arm
  (`unless negating comparison_tail`), as lean's refusal of `Lit::Cmp` under `!`. A literal that ends `bare` there
  is refused at the statement's end (`provisional bare`), as `negated` refused one with no pattern after it.
- **A nested choice's field where it returns, in one machine too.** Where a function is a rule, a call of a
  nested choice that makes one node already took its field where it returned. In one machine the choice was entered
  at its first token with the field on its first node, which a `decide` inside it then took away. Now the field
  goes to the return there too (the state it returns to, `returned_field`). The one-node proof (`one_node`) counts
  a decided node as one with no field of its own. A reread of a node with no field is one node too: a reading renames
  a field the node has, and none where it has none.
- `aggregate_literal` holds its own `bare`, as the literal's word does. A `bare` given to the call would be set where
  it returns, after the `decide` in it.

The tier 2 writer also joins the words under one guard into one arm when their code is the same
(`w::EQ | w::LT … if self.flags & FLAG_NEGATING == 0`), as lean's `!negated && is_comparison(k)`.

`negated`, `negated_term` and `aggregate_negated` are gone from `tier2.rs`; `literal`, `aggregate_literal` and
`group_term` read both places, under the flag.

Tests: the rr suite (the interpreter, the JIT and one machine against the engine; the generated Rust's records and
tapes, and the one loop's, against the interpreter's and the JIT's) and both corpus tests over the 1,949 files.
JSON's and Kotlin's generated Rust is unchanged, and their suites pass. The loop's check now names the first node
where its tape differs from the calls' (`node_differs`).

## Step 3: tier 1 holds no checker (done)

Where tier 1's loop reads its masks from the bytes (decision 0623 step 1), `bitlex_lex_rust` no longer writes the
test build's checker into it: no stage 1 of its own (`Stage1`, `stage1`, `stage1_best`), no walk of the regions over
the file, and no assert at a region's opener or an assumption's byte (`no_test_checks`, which also turns the `if {
assert; E } {` the asserts left into `if E {`). The nibble tables stay: the loop's classify reads them. The functions
the loop no longer calls go too (`drop_uncalled_fns`, until none is left): the whole-file mask helpers and the window
readers only the checker read.

The lean lexer holds no checker either. Its tests live apart, and so do tier 1's: `rust_tests` holds the records and
the tapes of the generated Rust to the interpreter's and the JIT's on every case, and the interpreter's own tests hold
the regions' walk.

JSON's and Kotlin's tier 1 read their masks in stage 1 and are unchanged.

## Step 6: tier 2 asks the marks' token tests (done)

Tier 1's loop made a word of hits for every token (`hits`), carried a token's "after" hits to the next grammar token
(`pending`), and joined them into each record's marks (`tested`). Rows of tests for every token, where tier 2 reads
two marks (`float_after` and `json_bad`) at a few places. Lean's lexer makes no marks: its parser asks what it reads of
a token where it reads it.

Where tier 1's loop reads the bytes (0623 step 1) and a byte's kind is `TOKEN`'s, `bitlex_lex_rust` writes the token
tests into a function of their own, `token_test` (one arm a mark: each row's tests, the row known by its first byte,
its kind and whether it is an extra), and `tested`: whether a test holds at grammar token `j`, its own or one of the
token after it at the grammar token before or a comment between, as the loop carried them. `TESTED` names those marks,
and no record holds them (`UNRECORDED`). Tier 2's `bit_at` and `marks_hold` ask `tested` for them, as they ask `gap` for
the gap marks: at a number's next token where `numeric` reads `float_after`, and at a JSON level's tokens for
`json_bad`.

The loop holds no hits, no `pending`, no `tested`, and with them none of the gap's state (`from`, `before`): a record is
its kind, its word and an extra's own bits. A float the file ends with (`float_after`'s `else escape`) is asked once,
after the loop, of the last tokens. The tests are of the windows from the token's first byte (`Bytes::rewrite_windows`);
a window past the end of a source with no padding (tier 2's) is its last bytes and spaces, as the padding's.

JSON's and Kotlin's tier 1 read their masks in stage 1 and keep their hits: their `TESTED` is none.

## Steps 4 and 5: tier 1's loop and its masks in the lean lexer's shape (done)

Where tier 2 asks the marks (step 6), the loop keeps nothing between tokens. `bitlex_lex_rust` then writes the lean
lexer's loop (`lean_loop`):

- **Four masks a block** (lean's `Window`): the bytes `skip` holds, a word's (the run of the common word row), a rare
  token's first byte (`k_rare`, nibble tables of its own, `RARE_0`), a pair's first byte. Nothing else is made of the
  block.
- **The block's starts at once**: a byte `skip` does not hold, not a word's after its first, not a pair's second (the
  pairs read left to right, `left_to_right`), as lean's `starts`. After a rare token they are made again from its end
  (`starts_from`), the byte after it a start where `skip` does not hold it: a word right after a number (`12abc`) starts
  there, as lean's number reader writes it.
- **The run of common tokens** before each rare start: one `TOKEN` load for the kind and the word alone, the last byte by
  the word's run or the pair's bit, the record written. No `match`.
- **Only the rare starts reach the rows' `match`**: a string, a comment, a backtick literal, `#`, a number, a byte no
  token takes, and `/`, whose group's test picks a comment or the operator. The rows only the run reads (`identifier`)
  have no arm. Each arm reads its bytes from the token's first (the next bytes tested where it reads them, the windows
  from its first byte: `Bytes::rewrite_windows`), not the block's masks: `/` tests the byte after it, a backtick the
  bytes after it, as lean's arms do.

A common row is a run of one mask whose first byte it holds, a pair, or one byte, with no marks, no region and no test of
its own, and no byte of a word but the run's. The rest are rare. Where a grammar's rows do not fit, the loop is the one
before.

Tests: the rr suite (its records against the interpreter's and its tapes against the JIT's on every case) and both
corpus tests over the repository's 1,949 `.rerail` files (`corpus_tier1s_records_are_the_interpreters`,
`corpus_tapes_are_the_jits`). JSON's and Kotlin's tier 1 are unchanged.

## Step 7: a rare token's bytes read as lean's arms read them (done)

The user, 2026-11-23, after steps 1 to 6: "address the substantive differences until we hit a point of diminishing
returns". Read whole again, tier 1's rare arms still differed from lean's in how they read a token's bytes: each was a
window function over 64 bytes from the token's first (`first_bit`, `run_last`, `count_in` of a mask made there), where
lean's arms call its micro-lexers or loop on the bytes. Where the loop is lean's (`lean_loop`), `Bytes` now writes the
arms' reads as lean's (`lean_read`):

- **A token's end by lean's micro-lexers**: a line comment's `micro::find` of the newline, a block comment's and a
  double backtick's `micro::find_pair`, a triple backtick's `find_fence` (lean's, written into the file where an arm
  reads a fence), a string's `micro::string_end`. `micro_first` knows them by the mask the grammar's `first` reads: one
  byte, two bytes in a row, a fence, a quote not escaped.
- **A run by a loop on the bytes**, as lean's digit and word loops: `while P(l + 1) { l += 1 }`, `P` the class's byte test
  (`matches!(src[i], b'0'..=b'9' | …)`). A run that may take one byte of another class once (a number's `.`) holds a
  `once` flag. A count, a first byte or a test over the token is an iterator over its bytes (`any`, `all`, `find`), an
  escape carried byte to byte as `string_end` carries it.
- **A long test a function of its own** (`u_ok`, `h4`, `b_six`, …): a named class whose test is long is written once, as
  lean's `unescape` is one function, not inlined at each read.
- **The rare token's row by one `match`**: the row's pick and the rows' match were two matches, a group's tests
  picking its row and the row's arm testing its bytes again. Now one `match pick` (`one_pick`), a group's tests the arms'
  guards (`12 if src[s + 1] == b'/'`), a region's openers one arm each, as lean's `b'/' if …` arms. An arm whose region
  is always closed holds no `open_bad`.
- **No dead code**: `drop_uncalled_fns` counts a call (a name and `(`, `::<`, or the name given as `dispatch!` takes it)
  outside the function's own body and outside comments. A variable of the same name, a doc comment's mention or the
  function calling itself kept `block`, `escaped` and `padded` before.

The arms' code is then tidied as lean writes it (`tidy_lean`): `l` for `(l + 1) - 1`, `.any(…)` for a count `>= 1`,
`.all(P)` for a `find` of `!P` that is none.

Tests: the rr suite and both corpus tests over the 1,949 files; JSON's and Kotlin's generated Rust is unchanged
(their lexers are not `lean_loop`'s), and their suites pass; clippy.

## Step 8: tier 1's tables built as lean's (done)

- **`TOKEN` and `WORDS` are `const` blocks**, as lean's: `TOKEN` a byte's most common entry, then each run of bytes of
  one entry (`table_block`), where it was 256 literal entries; `WORDS` built from the grammar's `WORD_LIST` by
  `word_slot`, the hash `word_at` reads, asserting no two words share a slot, where it was a literal row a slot.
  Kotlin's tier 1, written by the same writer, holds its 122 words the same way.
- **An operator's code by one load**, as lean's `TOKEN[c + 256]`: a pair's entry is at `256 + c`, its first byte `c`,
  and where one first byte starts two pairs the second byte picks (`+ 512`, as lean's `<:`). The run's operator and the
  rare `punct` arm read their word there, where a pair went through `word_at`'s hash. A grammar whose first byte
  starts three pairs, or whose second bytes cannot pick them apart, is refused by the writer.
- **`token_test` a plain function**: it reads no vector, so it is no `dispatch!` of a backend's. Its byte tests are
  Rust's byte literals (`b'0'..=b'9'`), not hex, and the tests tidied as the arms' (a run of one byte at least from
  the token's first is no test).
- **No unread table**: `drop_uncalled_fns` drops a one-line `static` nothing reads (the nibble tables only the
  removed checker read).

Tests: the rr suite and both corpus tests over the 1,949 files; JSON's and Kotlin's suites (Kotlin's tier 1 re-blessed:
its `WORDS`); clippy.

## Step 9: a float by its own token, and records without marks (done)

- **A float is its number's own test**, as the lean lexer's number arm writes `FLOAT` where it read a `.`: `mark
  float_at = empty, at number when count(dot) at least 1`, asked where tier 2 reads the number (`numeric`, `minus`).
  It was `float_after`, a test carried to the token after the number, so a float the file ended with had no token to
  hold it and refused the file (`else escape`). Tier 1 now asks nothing after its loop, and such a file is read whole,
  as lean reads it. A kind of its own in tier 1's records would need the kind decided at the number's first byte from
  the bytes after it, a pass backward (`back_fill`), which decision 0416 removed; the token's own test is what the
  lean lexer reads, at the token. `numeric` is `nested`: the caller's field is set where it returns, not on the mark.
- **A mark that holds only at one kind is no test of its own where that kind's arm is the same** (`implied_kind`, in
  `decide`): `float_at` is asked only where a float is read otherwise than a number (`numeric`, `minus`), as lean's
  one arm `NUM | FLOAT` elsewhere. Kotlin's `real` the same.
- **A record is its kind and word, an extra a span in its list**, as lean's `Tok` and `comments`: where each rare
  arm's marks are none or the extra's own bit (`no_record_marks`), the arms' tuple has no marks, the record is
  `kind | word << 8`, and the token tests read `extra` for the extra's bit. The records test gives an extra its bit,
  as the interpreter's record holds it.

What tier 1 still refuses where lean writes `BAD` is what the bytes cannot say: a `\u` escape `unescape` reads by
characters (a surrogate pair joined, `\u+abc` taken by `from_str_radix`, an escape cut short by the string's end), a
number of 19 digits that may fit an `i64`, and a fence of four backticks or more (lean finds a fence of its own count;
the grammar's regions are fixed openers). These stay limits (decision 0650), the engine's to read.

Tests: the rr suite (the end-of-file floats now taken, the engine's tape), both corpus tests, the corpus tapes against
the engine's and lean's (and lean's on edited files), the unpadded program's; JSON's and Kotlin's suites; clippy.

## Step 10: the block's classes in tables of their own, rare among them (done)

- **The block's classes packed alone** (`cover`, in `block_from_classes`): the nibble tables hold the classes the
  block's words read (`ws`, `word`) and the rare tokens' first bytes, and nothing else. They were stage 1's tables,
  whose bits held every class stage 1 writes, ten of sixteen read by no word of the lean loop, and rare was a table pair
  of its own (`RARE_0`) classified apart. A class is the bits it is the union of: a bit already in a pair whose bytes
  are all the class's is shared, and each new bit a set of high nibbles times a set of low ones widened over the
  class. Each class's bits are named, as lean's `SPACE`, `WORD`, `RARE`: `S::test(c0, S::splat(WORD))`. Three
  classifies of classes a block became two.
- **Two pairs, where lean's are one**: `ws` two bits, `word` four, rare seven. Lean's rare is `" # / \`` and the digits,
  non-ASCII by a compare; ours holds the grammar's `other` too (controls, `$ ' ? \`, DEL), which lean's common path
  takes one byte at a time as `BAD` from its `TOKEN`, where our `bad` is a run of them, a rare row of its own.
- **The arm of none is what reaches it**: a byte whose group's tests none holds, four backticks or more (the grammar's
  escape, decision 0177). It was said to be a byte no token begins with, which no byte is: `bad` takes them.

Tests: the rr suite, the eight corpus tests; JSON's and Kotlin's suites; clippy.

## Step 11: lean's `Bad`, a byte or a character, and one pair of class tables (done)

- **`bad` a byte, a character past ASCII one token**, as the lean lexer's `Bad`: `token bad = start bad_first pair
  empty` (a byte no token begins with, `class junk`) and `token bad_char = start char_first run cont` (a lead byte,
  or a byte that continues none, through the bytes that continue it). They were one token, a run of `other`, which
  lean's lexer never makes: it writes a `Bad` a byte, and a character's bytes one.
- **A junk byte is a common token, its kind by the load**, as lean's `TOKEN` gives a byte no token begins with
  `(BAD, 1)`: a row of one byte that records no word joins the common path (`wordless`), beside the operators,
  where no pair starts at its bytes. `pair empty` is a one-byte row's shape, `s`. The rare match has no `bad` arm.
- **One pair table and one end rule for both**: a kind of no pairs takes the pairs' table where its byte starts
  none (`next_of`), and its rule is the pairs' rule where each test one of them has not cannot hold at the other's
  tokens, each one byte, by that byte alone (`Lowering::at_byte`, a mask at a byte where the byte alone says, at
  its delays). `punct`'s `-` test holds at no junk byte. The records' fused pair rule is the lean `TOKEN`'s: the
  operators and the bad bytes.
- **`untaken K …`** (Bitlex, `03-bitlex.md`): the lexer's errors, which `any` and `balanced` take no more than a
  rule, as a region's `open K` already was. `bad_char` is one, so a raw line holding a character past ASCII
  recovers, as the lean parser's `raw_line` fails at a `Bad`; tier 2 is as it was (`bad_char`'s record kind is
  `bad`'s, no switch telling them apart).
- **One pair of class tables**, the lean lexer's `LO`, `HI`: rare is now lean's own, `" # / \``, the digits, and
  the bytes past ASCII by a compare (`S::or(S::test(c, S::splat(RARE)), S::ge(v, 0x80))`, lean's `ge(x, 0xc0)`:
  ours takes a byte that continues no character too). A class shares a bit already packed whose bytes are all its
  own, and its new bits cover only what the shared ones leave; the smaller classes are packed first, so `word`
  shares rare's digits. `ws` two bits, rare three, `word` three more: eight.

Tests: `tier1_reads_leans_bad_tokens` over the cases, among them a run of junk and characters and both in a raw
line, each read as lean's tape by the interpreter, the JIT and the one machine; the rr suite, the eight corpus tests;
JSON's and Kotlin's suites (their Rust unchanged); Bitlex's own; clippy.

## Step 12: a group's tests as lean's arms make them (done)

- **No test of the byte the pick says** (`counted_runs`): where one byte alone picks a group (`/`, the backtick), its
  arms test what follows it only, as lean's `b'/' if d == b'/'`. They tested the first byte again.
- **A run of one byte counted once**: a group whose arms are runs of one byte of one, two, three (the backtick
  literals) is one arm that counts the run (`k`) and reads by its count, as the lean lexer's backtick arm; its arm of
  none, four or more, is that `match`'s. The arms tested each backtick again.
- The record's doc says what it holds since step 9: its kind and its word.

Tests: the rr suite, the eight corpus tests; JSON's and Kotlin's suites (their Rust unchanged); Bitlex's own; clippy.

## Step 13: a string's `\u` and a long number read as written, a checker's reading (done)

The user's rule (2026-11-23): tier 1 may handle errors otherwise than the engine. tree-sitter is the reference, and
the job is to check source code, not to be pedantic. Two of the three refusals were the engine's error checks, not
syntax:

- **A `\u` in a string is the string's**, whatever its digits. A lexer reads a string to its closing quote, and
  tree-sitter's reads a string. The grammar's `escape "a \u escape the engine checks"` and its masks (`u_four`,
  `u_brace`, `b_short`, `b_four`, `b_five`, `b_six`, `u_ok`, three classes) are gone, and with them tier 1's six
  functions and the string arm's refusal. JSON's `\u`, four hex digits, stays `json_bad`'s (`uesc`, `h4`): the
  grammar's JSON check, the lean parser's.
- **A number's digits are its text**: an integer of 19 digits or more is a number. Its `escape … at number when
  all(digit) at least 19` is gone, and the number arm's refusal with it. No executor makes a check of a token
  that refuses (`CheckThen::Escape`) any more.
- The lean parser still writes `Bad` for both, as the engine's `unescape` and `parse` refuse them. The tests hold the
  generated parser to lean's tape of the same text with each such string's escaped `u` an escaped backslash and each
  such integer `0…01`, of the same lengths (`as_lean_reads`), and to the engine's tape the same way.
- **Four backticks or more stay refused.** They are no error: lean and the engine read a fence of N backticks, closed
  by the next N. The grammar's regions have fixed closers, and a closer as long as its opener would be a new region
  reader in every executor (the interpreter, the JIT, Metal). The corpus holds no such literal. A file with one goes to
  the engine, which reads it.

Tests: the rr suite (`rr_takes_a_strings_unicode_escapes_as_written`, `long_numbers_are_numbers`,
`a_numbers_token_takes_its_digits`; the marks test refuses four backticks at a token now), the eight corpus tests;
JSON's and Kotlin's suites; Bitlex's own; clippy.

## Step 14: a float is its record's, from the number arm's read (done)

The user asked whether a float should be its own record kind (2026-11-23). It should be told by its record, written
where the number is read, as the lean lexer's number arm writes `FLOAT` where it read a `.`. Until now tier 1 as Rust
wrote the number's record without it, and tier 2 asked `float_at`'s token test (`TESTED`, `token_test`), which read the
token's bytes again for a `.`.

- **How.** A token test its row's own read decides is the record's bit (`once_hits`, in the lean lexer's shape): a test
  `count(D) at least 1` at a token of `run R once D` holds where the run read its one `D`. The number arm reads the run
  with that flag (`run_once_hit`, `let (l, once) = …`), and writes it into the record as `float_at`'s bit (`kind1 |
  once << 16`). `TESTED` loses the bit, `token_test` its arm, and tier 2 reads the bit from the record (`code_at`).
- **The bit, not a kind byte of its own.** The interpreter, the JIT's record pass and Metal already write `float_at`'s
  hit into the number's code at that bit (`CheckThen::At`). Tier 1 as Rust's records are now theirs at it, and
  `tier1_differs` compares it. A kind byte of its own would make every switch of tier 2's that takes a number take a
  float too (lean's `NUM | FLOAT` arms), and the masks' record passes would need the kind at the number's first byte
  from the bytes after it (`back_fill`, decision 0416). The bit is what lean's two kinds mean: a number, and whether it
  is a float, told where it was read.
- `no_record_marks` takes an arm's record only as a line of its own, not a binding's pattern (`let (l, once)`).

Tests: `a_float_is_its_records` (the arm writes the bit, `TESTED` holds `json_bad` alone, the records of `1.5` and
`3.25` hold it, `2`'s not); the rr suite; the eight corpus tests; JSON's and Kotlin's suites (their Rust unchanged);
Bitlex's own; clippy.

## Step 15: a byte's classes by one load, a number by two runs (done)

The last two differences from the lean lexer's arms:

- **A class by its bit in `CLASS`** (`class_bit`, `class_table`): in the lean lexer's shape, a class of more than one
  byte tested of a byte is `CLASS[usize::from(src[i])] & C_NAME != 0`, one load and its bit, as lean's `CLASS[b] &
  DIGIT`. The table is built at compile time from the grammar's classes, a bit a class in the order first tested
  (`C_WORD`, `C_DIGIT`, `C_CONT`, `C_CTL`, `C_JESC`, `C_HEX`). A class of one byte stays a compare (`== b'.'`). They
  were each class's ranges (`matches!`) at every test.
- **A number's two runs** (`two_runs`): `run R once D` where `R` is `A | (D & X)` and no byte is both `A` and `D` is
  the run of `A`, then, where `D` and `X` hold at the byte after it, that byte and the run of `A` again, as lean's
  number arm reads the digits, then `.` and a digit, then the digits. `X`'s `prev(A)` holds after the run where the
  row's first bytes are all `A` (each row's first bytes, `row_firsts`), so it is no test, and its `next(A)` is the byte
  the second run starts after (`l += 2`, lean's `end += 2`). The loop tested "a digit, or a `.` between digits" at each
  byte, with its `once` flag.

Tests: `tier1_tests_bytes_as_lean` (the table, no class's ranges in the loop, the number arm as lean's, `1.2.3` as
`1.2`, `.`, `3`); the rr suite; the eight corpus tests; JSON's and Kotlin's suites (their Rust unchanged); Bitlex's
own; clippy.

## The six gaps, where they stand

Read off the tier 1 Rust as it is now (`rr_aot/src/tier1.rs`, its line numbers and names):

1. **A rare token's end**: lean's micro-lexers (`micro::find`, `find_pair`, `string_end`, `find_fence`) and byte loops;
   no window function, no `cls_*`, `w_*`, `wfirst_*` or `wrun_*` (steps 4, 5, 7).
2. **The pick's tests once**: a group's arms test only the bytes after the one the pick says, a backtick run counted
   once (step 12).
3. **`BAD` where lean writes it**: a block comment, string or literal open at the end, a byte no token begins with
   and a character past ASCII are `bad` tokens, their statement recovering (decision 0650, step 11). A string's `\u`
   and a long number are read as written (step 13). One refusal stays, four backticks or more, which is syntax, not an
   error.
4. **Marks**: no marks in a record and no extra's bit (step 9), no `float_after` and no check after the loop. A
   float is its record's bit, from the number arm's read (step 14). `token_test`, `tested` and `gap` stay for what tier
   2 asks where it reads it: `json_bad` (the grammar's JSON checks, the lean parser's), and `gap` is lean's `on_line`.
5. **No dead code**: no `block`, `prev`, `next`, `fill`, `escaped`, `any`, `bit`, `set`, `toggle`, `held`,
   `padded`; `drop_uncalled_fns` drops a function or a table nothing calls (step 7).
6. **Tables**: `TOKEN` gives a pair's kind and code by one load (`+ 256`, `+ 512` for `<:`), `WORDS` is built at
   compile time from `WORD_LIST` (step 8), and the block's classes are one pair, lean's `LO`, `HI` (step 11).

### What the refusal left reaches

Over the corpus the corpus tests read (2,013 `.rr` files) no file holds four backticks. Since step 13 tier 1 refuses
no corpus file at a token: the three files with a number of 19 digits or more are read whole.
