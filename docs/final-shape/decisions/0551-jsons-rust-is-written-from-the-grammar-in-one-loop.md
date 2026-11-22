# 0551: JSON's Rust is written from the grammar, in one loop

Status: done by the implementer, first step. The direction is the user's (2026-11-21): JSON's Rust is to be written
by the Bitlex compiler from the grammar, as Kotlin's is (0550, Swift-Kotlin-Scala-support's `bitlex_rust` and
`bitlex_lex_rust`), not lowered from tier 2's native IR as `rust_stream` does; and tier 2 is "single loop", simdjson's
shape, not recursion. How it is done is the implementer's choice. Follows 0550. Page 40, step 54.

## What was there

- `rust_stream` (0525-0546, 0577-0579) wrote JSON's tier 2 by walking the native IR's words: a lowering of the IR, deprecated by
  0550.
- Kotlin's generator (`bitlex_rust::tier2`) writes recursive descent, format 15 through `Writer`, and refused
  JSON's grammar at its first keyword kind (`true`). Its tier 1 (`bitlex_lex_rust::tier1`) needs `skip` and has no
  `prefix`; JSON's tier 1 has no `skip` (simdjson's stage 1: token starts only) and strips a byte order mark.

## The decision

1. **Tier 1 with no `skip` is simdjson's stage 1** (`bitlex_lex_rust`, `stage1`). Its records are each token's
   first byte, the bits of the token rows' start masks, in order. Tier 2 finds each token's end from masks tier 1
   gives it: a run's, a pair's, a last byte's (`stage1_ends`, by row), and a reader's (`readers`). The masks, regions
   and errors are Kotlin's code, unchanged.
2. **`prefix`** clears the prefix's bytes from the file's bits (`lv`), where the file starts with them: no class,
   `~` or token has them, and positions stay the file's (decision 0335). `check` statements are tier 2's.
3. **Tier 2 as one loop** (`bitlex_rust::tier2_loop`, its own module `one_loop`):
   - A `nested` rule is a level. Its call opens its node where the caller is (the key, its field and supertypes,
     is the caller's), pushes the place it returns to and goes to the rule's first place. Its end closes the node
     and goes to the place it pops. The places are the arms of one `'run: loop { match st { … } }`.
   - Every other rule is written where it is used. A part with no level's call in it is structured code (`loop`,
     `match`); a level's call ends its place, and the code after it is a place of its own.
   - A decision is a `match` on the next record's first byte. A token row's first bytes are those its start mask
     may hold at, bounded by its classes alone (`bounds`); two rows that may start at one byte are refused, so the
     byte tells the row. A word of a one-byte token is its byte; a word of a longer one (`true`) is its row's
     bytes, then a `match` on the token's word.
   - A node whose body is a region's opener, its pieces and its closer (`string`) is read at its token, each piece
     a leaf, each required piece checked against its kind's words (`keywords … required`), as simdjson's
     `parse_string`.
   - A `check K = G` is a function of the token's bytes, written from `G`, run where a token of `K` is a leaf.
   - The tree goes through format 16's writer (`f16::Writer`): each node's number is a literal of a code table the
     generator makes (`ENTRIES`: kind, field, supertypes), each node committed as it is made (none changes after),
     the root closed at the file's end, as the JIT's.
   - The levels open are bounded by `DEPTH` (1024), as the JIT's calls.
4. **A keyword's kind named in a rule** (`true`, from `keywords number { "true" -> true }`) is a test of its one
   word, in `bitlex_rust`'s shared `first` and `token_of`. Kotlin's grammar names none, so its output is the same.
5. **`rerail-json-aot`** holds the output (`tier1.rs`, `tier2.rs`), as `rerail-kotlin-aot` holds Kotlin's.

## Checks

- `json_tests::json_rust_is_current`: the crate's files are the compiler's output (`RERAIL_BLESS=1` writes them).
- `json_tests::json_rusts_tapes_are_the_jits`: on the tree test's files and 1023 to 1025 levels deep, the
  generated parser's format 16 and 16z tapes are the JIT's, byte for byte, in the language `ENTRIES` makes: 174
  tapes the same, 86 refused by both, none differ.
- JSON's tests: 10. `rerail-compiler`'s tests: 151. `rerail-kotlin-parser`'s `rust_is_current`, on a farm host:
  Kotlin's generated files unchanged.

## Found on the way

A 16z tape written with no commit differs from one committed as its nodes are made (the first piece on), the same
length. Format 16's bytes do not differ. The generated parser commits each node, as the JIT's direct run does. Why
an uncommitted 16z tape differs was not looked into.

## Not checked, not done

- Speed: nothing was measured. The writer is the host's `f16::Writer`, called through its methods, a commit a node;
  no room is reserved once, no widths are constant. Tier 1 makes whole-file mask vectors, as Kotlin's.
- The corpus (`JSONTestSuite`, the test data) was not run through the generated parser.
- 0541-0546's and 0577-0579's work on `rust_stream` is not redone here: the empty container's peek, the closer's check before the
  walk, the infallible open and close, the two-way return.
- `rust_stream`'s JSON tests and `aot_tests`, and `rr_aot`, still use the deprecated modules (0550).
- Which other grammars the loop form takes: only JSON's was tried.
