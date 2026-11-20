# 0451: Tier 1 written as Rust in the lean lexer's two stages

Status: decided (2026-11-19). It closes gap A of 0431 and states what is left of gap B. The
user: "please redo the tier 1 rust generation".

## Intro

Kotlin's tier 1 ahead of time (`kotlin_aot`'s `out/tier1_aot.rs`) used to be the JIT's Cranelift
IR translated line by line (`clif_rust`), in 41,235 lines:
- numbered SSA variables;
- loads and stores at frame offsets;
- a label for each IR block;
- a static table of 5,864 words.

It now comes from the block program and its records (`rerail_compiler::rust_block`). It has the
two stages of the lean lexer (`kotlin_lean`'s `block.rs` and `lex.rs`) and is 3,948 lines.

## The shape

1. **Stage 1, the masks.** A loop over the 64-byte blocks, dispatched over `rerail_simd8`'s
   backends as `block.rs` is (`dispatch!`, `S::load`, `S::classify`, `S::eq_mask`). A block's
   values are named `let`s:
   - `is_quote` for a byte compare, `class3_05` for a class;
   - a stored output by what reads it: `blank`, `simple_identifier_run`, `quote_opens`,
     `line_comment_tag`, `mark4`, `tier2_17`;
   - `v{i}` for any other value.

   A carried operation keeps its state in a local across the loop: `last_x` for `prev`, a
   carry for each `fill` and `escaped`. The masks come out as `struct Masks`, a `Vec<u64>` each,
   with each output's delay taken off.
2. **The regions** (strings, comments, templates) come from a walk of their own between two
   block loops. Stage 1a makes only the walk's openers and closers. `regions()` goes from
   region to region, as `BlockProgram::walk_group` does. Stage 1b reads what the walk made.
3. **Stage 2, the token loop** (`Tokens::run`). It goes past the blanks (`next_clear`). Then
   `match END[first byte]` picks an arm a rule, commented with its token's name: a run, a pair, a
   last mask, a pair table, or a reader. A reader has a function of its own (`read_quote`), and a
   region read in parts has one more (`read_quote_parts`, as the lean string reader).
   `Tokens::token` writes a record:
   - its kind, from `KIND` or a retag;
   - its word, from the perfect hash;
   - its marks;
   - its gap marks: `hits`, `unless`, and the hits carried to the next grammar token.

   The bits tier 2 tests itself are left out, as the CPU's tier 1 leaves them
   (`lazy_code_bits`). A token the file is refused at ends the loop before its record, as the
   JIT's loop ends.
4. **The entry.** `tier1_entry` has `native_cpu_block::Entry`'s signature and its own
   `repr(C) Row`. It writes the `Batch` the JIT's tier 1 writes:
   - the records, by the layout;
   - the counts and the refusal flag;
   - the masks tier 2 reads from tier 1's words (`BlockNative::masks`), leaving out those tier 2
     reads from the bytes (`records_made`).

   The JIT also writes masks that nothing reads. The generated code makes none of them.
5. **One source, two readers.** The build script and `aot()` write the source with the same
   expression (`src/tier1_source.rs`), and `aot()` checks the fingerprint `SOURCE`. The table
   check and the `STRUCTURED` assertion are gone. `rust_block` refuses what it does not write yet:
   - records without a skip;
   - a prefix, a gap rule, or rows;
   - a fixed point that is no walk;
   - `BackFill`, `Pair`, `Prefix`, `Phase`, `Position`, `Cmp`, `Bit`.

## A bug the comparison found

`aot_tapes_are_the_jits` now compares two independent tier 1s, so it found a JIT bug. At `/`,
the comments' reader goes on to its other rule, a pair table. But the JIT's token entry took
the pair row from the byte's own rule, which is the reader's, so the row was 0. `/=` was then
read as `/` and `=`. The entry now takes the row of the rule the reader goes on to, as
`pair_of` already did for the codes. The case is in `statements_and_expressions_are_the_lean_parsers`.
The one corpus file with `/=` now parses: 448 the same, 43 refused, 0 differ.

## What is left: gap B

Stage 1 still makes about 1,700 values a block, 383 of them carried words. Its 99 stored outputs
are the marks, the token tests, the untaken bytes, tier 2's masks and the totals, and between
them they reach almost every operation of the program. The lean lexer's `lex_blocks` makes a few
class masks. Closing B moves those questions out of the block loop into the token loop, at the
token where the lean lexer asks them. That is 0429's order of work, and it is now visible in the
generated source, a named `let` each.

## Tests

- `aot_tapes_are_the_jits` passes on all 495 corpus files: the records, counts and masks tier 2
  reads are the JIT's, and the tapes are the same on the 448 it parses. For a file tier 1
  refuses, the records are compared only up to the count.
- `structure` and `tier1_alone` pass.
- The compiler tests (128) and the Kotlin tests (51) pass.

## Kinds that share a first byte: the row's kind, not a code bit (2026-11-19)

Go-C-and-C++'s 0445 (point 3) and this branch's 0431 F step 2 solve one problem: a token mark,
or tier 2, telling apart two kinds that start with the same byte (the preprocessor's `#` and
`##`, Kotlin's string parts). 0445 adds a bit to the record's code per such kind, the OR of its
rows' start masks (`Records::kind_rows`): a mask made in stage 1 at every byte, and a code bit
from the 16 the marks share. F step 2 gives the record its row's kind, as the lean lexer's
`Tok::kind` does: the token loop knows the row from the path it took (a pair table's second
byte, a reader's arm, a string's part) or from the token's word (keyword rows), so the kind is
one table lookup and no mask.

**Decided: the row's kind.** It leaves stage 1 the lexer's masks (the direction of this
decision's gap B), spends kind values, which are plentiful, not code bits, which are not, and
is one mechanism: it already removed every `ROW` test from tier 2. At the merge with 0445:
- a keyword row gets a record kind of its own from its word, in the token loop, extending
  `ReadPart::kind` past the string parts;
- `TokenElem` tests that kind; `Records::kind_rows`, its code bit and its masks go, and so does
  `BlockNative::read_at_tokens`'s use for it;
- 0445's test `a_token_mark_tells_a_kind_by_its_rows_where_a_first_byte_is_shared` and the
  preprocessor's cases stay, ported to the kinds (Q282).

## `rust_block` names every field it reads

`Gen::new` takes the records apart field by field (`Records`, `Reader`, `ReadArm`, `ReadPart`,
`TokenCheck`, and each `CheckThen`), with the reason it ignores each lowering hint. A field or
a test's ending added on another branch (0413's `run … once` and `CheckThen::Cut`, 0445's
`kind_rows`) fails the build there until the emitter writes it or refuses it, never a wrong
tier 1 that only `aot_tapes_are_the_jits` would catch.

## Gap B, step 1: `return@` and `as?` are two tokens, joined by tier 2 (2026-11-19)

Stage 1 decided `return@` (`break@`, `continue@`, `this@`, `super@`) and `as?` with masks spelled a
byte at a time (`jat`, `jat_last`, `asq`), folded into the name's run and excluded from `punct`'s
starts, and refused a label's definition with an escape over them (`lab_last`). 0429's table said
the lean lexer's word reader makes them one token. It does not: the lean lexer reads `return`
then `@`, and the lean parser joins them where its rule reads them, by `adjacent(0)` (the two
tokens touch): `parse.rs` at `label_ahead` and the jumps, and `parse/expr.rs` at `as?`
(`K::TAsQuestion` from the `as` to the `?`).

The grammar now does the same, with what Bitlex already has:
- **Tier 1** reads two tokens: the name's run is its word's bytes (`name_more = name_tok &
  ~word_start`), `punct` starts at the `@` and the `?`, and the keywords `return@` … `as?` are
  gone.
- **Adjacency** is a gap mark, `apart` (a blank or a comment's first byte between two tokens),
  read by token marks as `without apart`: `this_at` (`"this" "@"`), `jump_at` (`"return" |
  "break" | "continue" "@"`), `as_q` (`"as" "?"`). Tier 2 tests token marks alone (0407), so
  they cost no bit and no mask.
- **The rules** take the mark and join the leaves with `merge`: `jump_at 'return' merge 'return@'
  '@' …`, `as_q 'as' merge 'as?' '?'`. tree-sitter implies `this@`, so `this_label` implies both
  of its tokens (`implied this_label 'this' '@'`) instead of merging them.
- **A label's definition** is `label_at` (`simple_identifier "@" without apart`; a token mark's
  kinds leave out the hard keywords). It is not hidden and no rule takes it, so it refuses the
  file where it holds, as the escape did.

**The code's mark bits.** The code holds 16 bits for marks and extras, and Kotlin used all 16:
three token marks (`blk`, `lam_bare`, `lam_params`) sat before the last mark that holds a bit
(`bt_cref`, which refuses alone), so they held bits they do not need. `bt_cref` now comes right
after the mask marks and the token marks after it. Their order among the marks that hold at the
same token is unchanged (`blk` before the lambda's marks); none of them shares a first token with
the marks they moved past. The code now holds 13 marks and 15 bits.

**Measured on the generated tier 1:** the block program went from 1,671 operations to 1,565;
stage 1b (`masks`) from 1,776 values a block to 1,656, and its carried words from 383 to 347;
`tier1_aot.rs` from 3,948 lines to 3,802.

**Tests:** the compiler (128), Kotlin (51) and Bitlex (11) tests pass; the corpus is 448 the
same, 43 refused, 0 differ; `aot_tapes_are_the_jits` passes on the 495 files. Checked by hand
against the lean parser: `as?`, `as String?`, `return@run`, `return@forEach`, `this@A.g()`,
`break`, `continue@x`, `x?.hashCode() ?: 0`; `loop@ for` refused.

**What `token_level_probe` lists now:** the keyword tests read by the comment escape `goes_now`
(`ctor_w`, `finally_w`, `where_w`, `catch_w`, `by_w`, `as_w`), by `awe_word` and `else_blk`
(`else_w`), and `three` (`===`, `!==`, `..<`), read by `p_start`.

## Gap B, step 2: a comment the lean lexer loses, lost where the lean parser asks (2026-11-19)

Stage 1 refused a file wherever a block comment that starts a line came before a token that goes
on the line before (`.`, `?:`, `as`, `where`, `by`, `!=`, …): the escape "a block comment before
a token that goes on the line", over `goes_now`. The lean parser takes such files. Its `asi()`
reads the comment through when it asks for a semicolon there, finds that the line goes on, and
tree-sitter loses the comment's bytes: a comment of no width, at the token after it, placed after
that token (`lost`). It asks only at a statement's level (`self.stmt`), so inside brackets the
comment is any other.

**The construct: `, loses W`** on a gap mark. Where a level asks for the mark at a token where
`W` holds, the extra pending last is lost: of no width at the token's start, and placed after the
token. `W` holds only where the mark does not. Kotlin's `semi` loses at `lost_at`, the first
solid byte after such a comment where the lean lexer loses it (a byte of `goes_on`, `!=`, `as`,
`where`, `by`). The escape keeps the cases `asi` does not lose: `catch`, `finally`, a header's
`constructor`, `else` but a `when` entry's, and annotations before a constructor.

**Where a level asks.** A state asks where it takes the mark, or where its level may end at the
mark (an `, ends` test) and its call returns to a level that is the mark's. That second part is
the lean parser's `stmt`, and the compiler derives it as a flag, one past the grammar's own
(`Mark::lose_flag`). A call sets it where the state it returns to takes the mark, keeps its
caller's where that state may end at it, and clears it elsewhere (`lose_flags`). So inside
`foo(…)` the expression's level does not ask: `value_arguments` takes no `semi`.

**Tier 2: `SETTLE m=2` and `m=3+f`**, right before the mark's `MARK`. Mode 2 sits in a state that
takes the mark, mode `3+f` in one that may end at it, with flag `f`. Where the mask holds and an
extra is pending, the extra is taken out and its kind kept in one state slot (`lost`). The next
`TOKEN` that takes a token makes it again, after the token's leaf, of no width at its start, before
the gap's extras: the lean parser's placement. With none left pending, a node that closes before
that ends at the token before (`keep`). The state shape (`stream::validate`) allows the one word
before a `MARK`. Every executor:
- the interpreter (`stream::run`);
- Cranelift (`Op::LoseLast`, `Mem::Lost`, `lost_leaf` in `TOKEN`'s both paths);
- the generated Rust (`S::lose`, and `TOKEN`'s remake).

**Bitlex's reference** asks after each token, where the follow holds the mark. A nested level's
follow now holds `END` where its rule may end there. With `END`, it asks for the marks its level
is the mark's for: those its call site's follow held, and the caller's own where the caller
could end there (`Parser::asked`). That is tier 2's flag.
`Token::lose` names the marks whose `W` holds at a token. A lost extra goes after the token at
its place, in the deepest node holding that token and the next.

**Tests:**
- `ladder_tests::a_mark_that_loses_in_the_reference`: the reference, the interpreter and Cranelift
  on 136 generated documents and fixed ones. A comment in a statement's chain is lost (width 0);
  one inside `(` keeps its 7 bytes.
- `lost_comments_are_the_lean_parsers`: 13 texts against the lean parser, chains, `?:`, `as`, `+`,
  `!=`, two lost comments in one chain, and `foo(…)`, `if (…)` inside brackets.
- `aot_tapes_are_the_jits` reads the same texts too (the corpus has none): 508 files, the
  generated Rust's tapes the JIT's.
- `what_step_20_does_not_read_is_refused` now lists a comment before `catch`.
- The compiler (129), Kotlin (52), Bitlex (11) and `rr_aot` (15) tests pass. The corpus is
  unchanged: 448 the same, 43 refused, 0 differ (no corpus file has the case).

**Stage 1 is unchanged by this step** (1,655 values a block, from 1,656). The escape's keyword
tests moved into `lost_at`, which tier 2 reads at the token. The token-level shape is next:
`lost_at` asks whether the token after the comment is one of the tokens of `semi`'s `unless
before`, and the token loop knows that from the record's kind and word, as it knows a `before`.

## Gap B, step 3: `===`, `!==` and `..<` in the pair table (2026-11-19)

The lean lexer reads these operators by their first byte, then the next two. For `=`, it gives
`===` where the next byte is `=` and the third is `=` too, `==` where only the next is, and `=`
otherwise. The grammar read them another way: `three`, a mask of their text at every byte
(22 values a block); a row of their own (`op3`); and three terms in `p_start` that kept their
inner bytes from starting a token.

Now they are entries of the pair table, the lean lexer's shape:
- **Bitlex.** `pairs P = "AB" … "ABC" …` takes three-byte entries. A triple's first two bytes
  must be a pair. The lowering makes `P_three`, the starts of `P` where a triple's bytes are.
  `pair P`'s last byte is then two past `P_three`'s starts. `TokenRule::Pairs` carries it.
- **The table.** `PairTable::thirds` holds the triples. `PairTable::last` is a token's last byte:
  its first byte; the next, where the two are a pair; the one after, where the three are a
  triple. Text form: `recthird ABC`.
- **Every executor:**
  - `records::last_by_rule` uses `PairTable::last`.
  - The JIT adds a compare of the token's three bytes to each triple (`third_last`), on both
    the table path and the pair-word path.
  - The generated Rust writes `next{i}(src, s, n)`. It matches the three bytes after the pair,
    as the lean lexer's `match` does.
  - The prover counts a triple's third byte as a token byte, for the blank marks.
- **Kotlin.** The three operators are in `two_start`, and their words in `keywords punct`.
  `three`, `op3` and the `p_start` terms are gone. `second` is `prev(two_start) |
  prev(two_start_three, 2)`.

**One limit, now an error.** `P` is every other byte of a run of pair starts
(`two & ~escaped(two)`). That is the left-to-right reading, except where a triple's last byte
starts a pair with the byte after it (`!===`, `====`). There the token loop reads `!==` and
then `=`, and the mask reads the run another way. No language has such text, and Kotlin's lean
parser refuses it. So a token that meets it is an error on every target: `K: a triple's last
byte starts a pair`, from `P_two` (the run's starts before escaping). The JIT's lean starts
(`lean_starts`) read the pair word's starts, so they are off for a table with triples. That
table's tokens end as the table says.

**Tests:**
- `ladder_tests::a_pair_table_with_triples`: the reference, the interpreter and Cranelift on
  generated documents and fixed ones; `!===` and `====` are refused.
- Kotlin's three-byte operators (step 17) and the rest pass (52). The corpus is unchanged: 448
  the same, 43 refused, 0 differ. `aot_tapes_are_the_jits` passes.

**Stage 1:** `three` is gone from the token-level probe. The block program went from 1,564
operations to 1,559.

## What stage 1 still computes, by reader (2026-11-19)

A count over the generated `masks()` after step 3. It follows each stored output back through
the values it reads.

| Reader | Values reached | Only this reader's | Outputs |
|---|---|---|---|
| Marks' masks (`mark_acc`, `mark_recv`, `mark_generic`, …) | 512 | 232 | 13 |
| Refusals (escapes and errors, `total*`; untaken bytes) | 475 | 217 | 11 |
| Tier 2's masks (`tier2_*`) | 418 | 104 | 31 |
| Token rules (`*_last`, `*_run`) | 309 | 49 | 6 |
| Regions, tags, checks | about 290 | about 55 | 39 |

The largest single cones are `acc`'s (104 values of its own), then three escape classes (83, 73
and 27), then `recv` (29) and `generic` (17). The lean lexer makes none of them. It asks the
marks' questions in the parser at a token (`accessor_ahead`, `receiver_end`,
`type_args_end`). It finds the escapes' constructs in its readers, at the token that has them.
In this order, these are the differences left:

1. **The refusals as one flag.** Done below.
2. **The masks named by the grammar.** Done below for the marks.
3. **The marks' masks at the token.** `acc`, `recv`, `generic`, `dotted`, `fn_recv` and `named`
   are context fills (`decl_run`, `after_ty`, `acc_word`) at every byte. In the lean parser each
   is a question at one token. This is 0429's step 2, the keyword marks into the token loop.
4. **The escapes at the token.** Each escape is a byte mask of a construct that a token's
   reader meets: a number's suffix, a string's `$`, a `\u`. Each moves into its token's reader
   or check, as the lean lexer's readers refuse.
5. **Tier 2's masks.** Tier 2 still reads 31 of tier 1's mask words (the drops, the lost
   comments, `token_all_x`). The lean parser reads only tokens.

## The refusals: one word, as the lean lexer's error flag (2026-11-19)

Six totals and five untaken-byte masks were each stored as a `Vec<u64>` a block, and `lex` ran
`.any()` over each. Nothing else read them. Now stage 1 ORs them into one word:
`refused |= (…) & within(i, len, d)`, one term for each delay, cut at the file's ends. `Masks`
holds `refused: bool`. A total that another reader has (a mark or a tier 2 mask) stays a mask.
`aot_tapes_are_the_jits` passes.

## The marks named by the grammar

`Names::marks` (text `name mark K "N"`) names each mark in the records' code by its bit: a
grammar mark's name, or `K_start` for an extra's starts. `rust_block` names a mark's output
`mark_{name}`, so `mark_acc` replaces `mark6`. The tapes are unchanged: the compiler (130), Kotlin
(52) and `kotlin_aot` (4) tests pass.

## Item 3, `acc`: an accessor asked at its token (2026-11-19)

`acc` was a mask mark. Its mask asked at every byte whether a `get` or `set` began its line, or
came after a visibility word that began its line (`acc_word`, over the fills `line_first`,
`line_mod` and `acc_vis`). The lean parser asks at the token: `declaration_ahead` reads the
modifiers, and `accessor_ahead` reads `get` or `set`. The line break is `Tok::nl`, which tier 1
already gives the record: `semi`'s bit, set by the token loop while it skips the blanks (0430).

Now `acc` is a token mark, a question tier 2 asks at the token:

```
mark acc = tokens "get" | "set" with semi, unless before "get" | "set" "." | ":", unless before "get" "(" ~")"
mark acc = tokens after "private" | "protected" | "internal" | "public" with semi "get" | "set" without semi, unless …
```

Two constructs make it work:
- **`E with M`**, beside 0430's `without M`. The element holds only at a token whose record has
  mark `M`'s bit, as the lean parser asks `peek_at(j).nl`. It is `TokenElem::with`, text `h{bits}`,
  and every executor tests it: the reference, the interpreter, Cranelift and the generated Rust.
  The `after` element takes it too.
- **A token mark's `unless before`.** These are the mark's entries with `unless` set, as a mask
  mark's are (`TokenMark::unless`, `patterns_hold`). The reference holds the token mark only
  where none of the runs follows.

The second definition is for a `get` on its visibility word's line (`without semi`), as
`acc_vis` was, so the two never hold at one token. A token mark costs one of the 32 mark orders.
So the definitions are few, and they come after the mask marks, keeping the code's 16 bits for
the marks that need them.

**Test:** `ladder_tests::a_token_with_a_mark`. A `get` after a line break, or on a `pub`'s line
after one, is an accessor unless a `.` follows. It checks the reference, the interpreter and
Cranelift.

**Result:** the compiler (131), Kotlin (52) and `kotlin_aot` (4) tests pass. The corpus is
unchanged: 448 the same, 43 refused, 0 differ. The block program went from 1,559 operations to
1,507, and the marks' cone in stage 1 from 512 values to 398. `line_mod`, `acc_vis` and
`acc_word` are gone.

## Item 3, `named`'s `!`: a token mark (2026-11-19)

`bang_test` was a mask at every byte: a `!` first on its line before the word `in` or `is`, a
`when` entry's negated test. It is now `named`'s third token mark, `tokens "!" with semi "in" |
"is"`, beside its two others. The corpus is unchanged (448 the same, 0 differ). The Kotlin (52)
and `kotlin_aot` (4) tests pass. The block program went from 1,507 operations to 1,495.

## Item 3, `generic`: two token marks (2026-11-19)

`generic`'s mask had three byte questions, and each is a question about tokens:
- a `<` touching the word before it, `prev(w)`: now `after simple_identifier "<" without apart`;
- a `(` first after a type's `:` (`colon_ty`, `after_ty`, a fill): now `after ":" "("`;
- a `suspend` there: also `after ":"`.

```
mark generic = tokens after simple_identifier "<" without apart (…)* nest ("<" to ">") ("(" to ")") ">" "(" | …
mark generic = tokens after ":" "(" | "suspend" "("
```

The `:` token is its own word. `::` and `?:` are pairs, so `colon_ty`'s exclusions come free. A
number before `<` is left out. Its kinds share their first byte (`0`), so a token element cannot
name them yet, and no Kotlin type's arguments follow a number. `named`'s two name definitions
share their first element and are now one, so the mark orders stay within 32.

The corpus is unchanged (448 the same, 0 differ). The Kotlin (52) and `kotlin_aot` (4) tests
pass. The block program went from 1,495 operations to 1,471.

## Item 3, `fn_recv`: a token mark (2026-11-19)

`frecv_at` was a word's start not touching a `.`, `<` or `>` and not `suspend`, at every byte.
It is now `tokens after ~("." | "<" | ">") simple_identifier … "." "(", unless before "suspend"`.
The mark is hidden, so it is asked only where a type starts, and there a spaced `< A.()` does not
come. The corpus is unchanged (448 the same, 0 differ). The Kotlin (52) and `kotlin_aot` (4)
tests pass. The block program went from 1,471 operations to 1,450.

What is left of item 3 is the declaration's run: `recv`, `dotted` and the escape "a nullable
receiver" all read `decl_run`, a fill from `fun`, `val` or `var` to the declaration's stop, with
`tp_body` and `rstart` under it. The lean parser has no such run. `receiver_end` is a forward scan
from the token after the keyword, taken by the declaration's rule. That is one change for the
three: the receiver becomes the rule's lookahead at its token. Stage 1's marks' cone is now 329
values, from 512, and 38 of those are `recv`'s own.

## Item 3, the declaration's receiver: tier 2's, as `receiver_end` (2026-11-19)

Stage 1 made the receiver's context at every byte. `decl_run` was a fill from `fun`, `val` or
`var` to the declaration's stop, with `tp_body`, `rstart` and four more fills under it. `recv`
(the receiver's first token) and `dotted` (its last `.`) were masks over it. The escape "a
nullable receiver" was too. `dotted` also held at a delegation's `{` (`del_brace`, over the fills
`class_decl` and `obj_decl`), where it ended the delegate's expression. The lean parser has none
of this. `receiver_end` scans forward from the token where the receiver may come. A delegate's
expression is read with `no_lambda` set, and the postfix loop asks `T::LBrace if !self.no_lambda`.
Now the grammar does the same:
- **`recv`** is a hidden token mark: the run of a type's tokens to a `.` (or `?.`) and a name that
  no `.`, `<`, `>`, `?.` or `,` follows. Hidden, it is asked only where `receiver` may come, after
  the keyword and the type parameters, so it needs no context. A receiver ending in `?.` holds it
  too, and the receiver's rule, which reads no `?.`, refuses it. That is the escape's refusal,
  made where the lean parser reads the token.
- **The receiver's loop** is its own rule: `recv_type = retag user_type type_name [type_arguments]
  (dotted '.' type_name [type_arguments])*`. `dotted` is now a hidden token mark, a `.` whose name
  another `.`, `<`, `>`, `?.` or `,` follows. The loop goes on there, and at the last `.` it ends
  and `receiver` takes the `.`. This is `user_type`'s `i != type_stop`, asked at the `.`.
- **`no_lambda`** is a flag, set over a delegate's expression and an object literal's specifiers.
  It is clear again inside parentheses (`value_arguments`, `parenthesized_expression`). The
  postfix loops take a trailing lambda alone (`lambda_call`) and a call's lambda after its
  arguments only where it is clear. The guard sits on the loop's own arm, as in the lean parser.
  In a choice the loop calls, the guarded arm is in a state that cannot end, so it refused there.
  The compiler does not carry a guard out to the caller's arm (an open limit: a guard belongs
  where the level may end).

Gone from stage 1: `decl_kw`, `after_kw`, `first_tok`, `tp_body`, `decl_stop`, `decl_run`,
`after_tp`, `rstart`, `class_decl`, `head_brace`, `object_w`, `obj_decl`, `del_brace` and one escape.
The mark orders stay at 32.

A receiver after nested type arguments in the type parameters (`fun <T : Comparable<T>> B.f()`)
was refused, because `tp_body` ended early there. It is now read as the lean parser reads it, and
it moved from `what_step_22_does_not_read_is_refused` to `receivers_are_the_lean_parsers`.

**Result:** Kotlin's tests (52) and `kotlin_aot`'s (4) pass. The corpus is 450 the same (from
448), 41 refused, 0 differ. The block program went from 1,450 operations to 1,380, and the
marks' cone in stage 1 from 329 values to 270. That leaves the marks' cone with the gap marks
alone: `semi`, `semi_close`, `head_ctor`, `apart` and `real`. Item 3 is done.

## Item 4 measured: what the escapes cost stage 1 (2026-11-19)

The block program with every escape taken out (`token_level_probe`, which now reads
`RERAIL_KOTLIN_GRAMMAR`) is 1,076 operations, against 1,450. By family, each taken out alone:

| Escapes | Operations |
|---|---|
| Comments the scanner reads with the next line (`asked_blk`, `after_asked`, `else_next`, `ann_ctor`), `;` first on a line, a comment on an annotation's line | 255 |
| Strings and templates (a `${…}` with a brace, a raw string's quotes, `$` outside a string) | 47 |
| Numbers (a `0x` with no digit, a suffix, a trailing `_`) | 37 |
| The receiver's `?.` (now gone, above) | 16 |
| A character's `\u` | 12 |
| A byte above 0x7f, a nested block comment | 7 |

The comments' escapes are two thirds of the cost. Each is a question the lean lexer's `asi` asks
at the token after a line break, as `semi` already is (0430). They come next.

## Item 4, comments, step 1: an extra keeps the line breaks of its own gap (2026-11-19)

The lean lexer's `skip` knows, for each comment it passes, whether a line break came before it.
`asi` reads that: a block comment first on its line, after a line break, then the token after it.
Tier 2 can only ask the same question at the token if a comment's record carries it. Until now,
a gap mark's bits went to the next grammar token alone, and an extra's code held none.

Now an extra's code holds the blank marks (`Records::blanks`, Kotlin's `semi` and `head_ctor`)
of its own gap: the bytes from the record before it to its start. The grammar token after the
comments still holds the gap's marks from the grammar token before, as it did. The plan for the
rest of the comments follows from this:
1. drops and loses as token patterns over the raw records, comments included, as `asi`'s cases;
2. Kotlin's `semi` written so, and `swallow_at`, `lost_at`, `asked_blk` and `after_asked` gone;
3. settles by the comment's own bit;
4. the comment escapes as refusals over those patterns.

Every executor writes the bits:
- the interpreter (`records::gap_marks`);
- Cranelift's token tail, from the blank run and the lifted bits;
- its `late_marks`, whose cursor now stops at the first record at or after a bit, so that a
  second comment in a row gets its own;
- `rust_block`'s `token()`, from `from_record`;
- Metal's record pass.

Cranelift folds no lifted bits past blocks where a blank mark is lifted and there are extras,
because a folded bit can't be told to its comment.

**Tests.** `a_gap_mark_read_from_the_blanks` checks the interpreter's and the CPU's codes on
`ladder::BLANKS_TEXT`. `an_extra_keeps_its_blank_marks_on_metal` checks Metal's forward carry
against the interpreter, and fails on the old Metal code.

**Result:** Tapes unchanged. The compiler's tests (131), Kotlin's (52) and `kotlin_aot`'s (4)
pass. The corpus is 450 the same, 41 refused, 0 differ.

## Item 4, comments, steps 2 and 3: `drops` and `loses` asked at the token (2026-11-19)

Before this step, `drops W` and `loses W` named stage-1 masks: `swallow_at` and `lost_at`, built
from `asked_blk` and `after_asked`. Tier 2 tested them at the token's start. Now each is a
question over the records as the lexer wrote them, comments included: Bitlex's `GapPattern`,
written as `, drops [unless] after A … before B …`.
- `after`'s elements are the records before the token. The last of them is the comment pending
  last, and its `with semi` reads the comment's own gap (step 1).
- `before`'s elements run from the token on.
- A new element, `starting "B"`, tests the token's first byte, as `asi` reads the byte after
  the comment.
- A mark's clauses are its entries. The question holds where no `unless` entry does and one of
  the others does, as the token marks' entries decide.

Kotlin's `semi` now says `asi`'s two cases itself:
```
drops after ~";" multiline_comment with semi before starting "aew",
drops unless after multiline_comment before "as" | "where" | "else",
loses after ~";" multiline_comment with semi before starting ",.:*%><={[(?|&/",
loses after ~";" multiline_comment with semi before "!=" | "!==" | "as" | "where" | "by"
```

The program keeps the questions as `Words::gap_patterns` (text rows `gappattern`). They are a
list apart from the token marks, because Kotlin's 32 mark orders are all taken. Each element is
the record at its offset from the token. `SETTLE m=1` and `m≥2` name a pattern's first entry
where they named a mask.

Every executor asks the question only once an extra is pending:
- the reference (`eval.rs`, over its tokens);
- the interpreter (`gap_pattern_holds`);
- Cranelift (`gap_pattern`, using the same element test as the token marks, `elem_test`);
- `rust_stream` (`gap_N`). Its question reads as `asi` does:
  `if !matches!(b, b'a' | b'e' | b'w') { return false; }`.

`validate_gap_patterns` refuses a `with M` on an extra's element unless `M` is a blank mark,
because an extra's code holds no other mark.

The two ladder tests say the same in the new form: `drops before starting "i"`, and `loses
after comment with line before "."`. Their masks are gone.

**Result:** Gone from stage 1: `lost_at`, `awe_word` and `swallow_at`. The block program went
from 1,380 operations to 1,334. Tapes unchanged: the compiler's tests (131), Kotlin's (52),
`kotlin_aot`'s (4) and Bitlex's (11) pass. The corpus is 450 the same, 41 refused, 0 differ.

**Still to do:**
- `asked_blk` and `after_asked` remain, for the escapes alone (step 5).
- `settles` still tests a mask (step 4).
