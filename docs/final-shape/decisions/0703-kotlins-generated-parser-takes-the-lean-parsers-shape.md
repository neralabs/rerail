# 0703: Kotlin's generated parser takes the lean parser's shape

Status: being built (the user, 2026-11-25: "We will need to make the code match more to get our performance back,
without making the Rerail or Json code match less", then "address all of these"). Follows 0681, 0685 and 0696.

## Intro

Kotlin's generated parser (`kotlin_aot`) is about 1.7 times the lean parser's CPU (`kotlin_lean`), its tier 2 1.4 to
1.65 times the lean parser's own work and its tier 1 about 6.5 times the lean lexer (8251c7bb7b). A count of the
two codes' shapes, tier by tier, found the rules themselves the lean parser's size (137 functions in 4,000 lines
against 151 in 3,522) and five differences, all Kotlin's but the last: rr's and JSON's generated code has none of
them.

- Tier 1 is the two-pass form: a whole-file stage 1 of 144 masks, then a token loop over them, then about 30 more
  whole-file passes for its refusals. rr and JSON get the lean lexer's one-pass form. Kotlin's strings and templates
  trip every condition that refuses it (`bitlex_lex_rust`, `fused`).
- Tier 1 works out ten marks a token (the line breaks, the semicolons, `as_open`, …) and tier 2 checks and tracks
  them in every rule (523 tests, 693 writes of `asks` and `marked`) and on every token (`open_mark`). The lean lexer
  gives a token one line-break flag, and the parser asks the automatic semicolon only where it reads one.
- The marks' lookahead is a function a state: 2,164 functions, 76% of tier 2. The lean parser asks the same with a
  test of a few tokens and, for the hard case, a trial read of its own expression, rewound.
- Precedence is ten functions, each its own copy of the operator loop. The lean parser has one loop with a
  minimum precedence. rr's lean parser has a function a level too (`term`, `product`): that shape stays where the
  grammar is written so.
- A name that is no hard keyword was tested against a list of 26 words, 673 times. The lean parser numbers its
  words hard keywords first and tests one compare (`ident_word`).

## Decisions

1. **A row's hard keywords are numbered together** (`bitlex_tier2::lower`: the keywords' strings hard before soft,
   row by row, each row's in the order of its kinds). A name's test against a row's hard keywords that are one run
   is one range test (`bitlex_rust::kind_test`), as the lean parser's `ident_word`; a row whose are not keeps its
   list. Kotlin's 673 lists are each `!(w::AS_1..=w::NULL_26).contains(&word)`, tier 2's Rust 1.74 MB to 1.52 MB. The
   runs of kinds that make a keyword's leaf one add (`word_kind`) are within a row, so they stay. rr's and JSON's code
   changes only in its word numbers.
2. **An operator ladder is one loop** (`G::ladders`, `G::ladder`). Three or more nested choices `H (op | …)*` of one
   head whose loops end alike (their exits, the marks after them) are one method, `H_ladder(set)`. It is the loop over
   every level's operators in one order that keeps each level's, each operator's tests guarded by its bit of `set` (an
   `unless` of the ladder's: a flag named `#i`, `flag_cond`), so an operator a level does not take falls through to the
   next arm as the level's own decision did. Each level keeps its head's call and its depth, then calls the ladder
   with its set. The lean parser's `expr(min)` takes a minimum, since its levels nest. Kotlin's do not (`gen_op` and
   `kw_infix_op` are some middle levels' only), so the set is a bit set. Kotlin's eleven levels of `unary_tight` are
   one ladder. rr's levels each have their own head (`term` over `product`, `product` over `primary`), as rr's lean
   parser has them, so it has no ladder, and neither does JSON. Their code is unchanged. Both of tier 2's Rust forms
   that call methods (`source`, `tier2_calls`) take the ladders; the one loop's form (rr's `tier2_loop`) has none.
   `RERAIL_BITLEX_LADDERS` prints the candidate groups.
3. **A token pays for the marks no rule takes only where one may hold** (`open_mark`). A mark each of whose
   definitions is a code bit at the token is asked behind one test of those bits of the token's code (with
   `tier1::ASKED` and `TESTED`, constants: where a bit is asked of the gap or the bytes, the test folds away). A mark
   each of whose definitions' runs starts with a kind is asked behind that kind's test, the marks of one kind together.
   Kotlin's plain token (the `_` arm) was five hold tests, each a `taken` bit and a function: now one code-bit test and
   two kind compares, as the lean parser asks a label only at a name. rr's arms keep their tests, its bits being asked of
   the gap; JSON has no such mark.
4. **The escapes after the token loop are passes over the words** (`one_pass_after`). Each escape made of `code`
   (`code_escape`) was its own pass over the file per mask, each a vector of the file's: 26 in Kotlin's tier 1, beside
   the test build's own. Now they are loops over the words, each mask a word: its `prev` its word and the word before's,
   a `fill` its carry, an `any` the word tested. A mask read past its word (`next`) starts a new loop before the read,
   and a mask a later loop reads is kept as a vector as well. Kotlin's are two loops, with one vector (`t108`, read two
   bytes ahead). rr and JSON have no escape made of `code`. A check's reason with a line break in it is written with
   `\n` in its comment.
5. **The gap marks of one mask are one bit** (`DERIVED`, `derived`, `carried`). Gap marks whose tests in tier 1 are
   one (one accumulator, mask or record test) have one carrier, the one of the lowest bit with no `unless after`, and
   tier 1 writes its bit alone. Tier 2 asks the others where a rule reads one: the carrier's bit at the token, and the
   word of the grammar token before for an `unless after` (`bit_at`, `marks_hold`). An extra keeps a derived mark of
   its own gap only where it keeps the carrier, so an extra's is its carrier's bit (`carried`, in `rec_marks_hold` and
   where the pending extras settle at a mark). A mark with token tests, an `else escape`, or one asked of the bytes is
   not derived. Kotlin's `head_ctor`, `by_lf`, `ann_lf` and `semi` (`lf & code`) are one bit, `head_ctor`'s: the lean
   lexer's `nl`, of which the lean parser asks `asi`. Kotlin's test of tier 1's records against the interpreter's
   checks each derived bit as tier 2 asks it. rr's marks are each their own (`DERIVED` 0), and JSON has none.
6. **Stage 1 is made as the token loop comes to its blocks** (`bitlex_lex_rust/interleave.rs`). The two-pass form's
   stage 1 is one pass over the blocks with carries. Its carries are now a state (`Stage1Run`), and the pass runs a
   step at a time from it (`stage1(src, st, o, upto)`, dispatched as before), writing its masks into `Stage1`. Each
   block of the token loop makes stage 1 through the next block, 16 blocks more at a time; a scan of a mask forward
   (`first_bit`, `run_last`) makes blocks as it comes to ones not made (`s1_first_bit`, `s1_run_last`); after the
   loop, stage 1 is made to the file's end, for its escapes (which now join the refusal there) and the passes after
   the loop. Every other read of the loop is at or before the token's last byte, which a scan found, or a few bytes
   past it. So tier 1 reads the bytes once, each block's masks made just before the loop reads them. (The lean
   lexer's `Blocks::of` makes its blocks' masks before its loop, for the whole file; it keeps 5 a block.) The test build's walk of the regions reads a stage 1 of its own, made whole. Kotlin's is the
   only tier 1 of the two-pass form; rr's and JSON's are unchanged. A check's reason with a carriage return in it is
   written with `\r`.
7. **The gap marks of the bytes before a token are asked by tier 2** (`SPANNED`, `spanned`). A gap mark that is a set
   of the skipped bytes since the record before, with a test of the bytes at the start (written from the bytes,
   `Bytes::at`, `text` now among them), or that holds at every skipped byte and every extra since the grammar token
   before (its `here` the extras' regions, which no grammar token reads), is written by no record. Tier 2 asks it
   where a rule reads one (`bit_at`, `marks_hold`, `open_mark`), as the lean parser asks `adjacent` and its scanner
   reads a gap back from a token. Read from the token back over the skipped bytes, and from the record before (the
   grammar token before, or the extra before) forward: a string's part between two grammar tokens is no record and
   carries the gap over, and a part is never ended by a skipped byte. Within a template of the grammar token before,
   only back. Tier 1's accumulators no mark it writes reads are gone. Kotlin's `own_line`, `nl_end` and `apart` are
   asked so, and `apart`'s accumulator is gone. Kotlin's test of the records asks them as tier 2 does, and a test reads
   the gaps around parts and comments. rr and JSON have no such mark (`SPANNED` 0).
8. **Stage 1 keeps only the masks the product reads** (`interleave::test_only`, `interleave::bit_reads`). A mask
   only the test build's walk of the regions reads is stored in a test build alone. A one-byte test of a mask that
   reads no region (`bit(&M, i)`), where `lex` reads `M` only so, is a test of the bytes at `i` (`Bytes::at`), as the
   lean lexer tests `c == b'/'`, and the mask is then one of the first kind. Kotlin's product stage 1 keeps 100 masks
   of 144: 29 were the walk's alone, and 15 are byte tests. The rest are read over a token's bytes (`any_in`,
   `last_in`, `count_in`), by the scans to a closer, after the loop, or by tier 2.
9. **A block's masks are kept together** (`interleave::together`), as the lean lexer keeps a block's in one
   (`Vec<Masks>`). Stage 1's masks the product reads are one `m: Vec<[u64; K]>`, each mask a column (`S1_*`, read as
   `Col`); the helpers that read masks take any words (`Words`: a vector's, a slice's, a column's). A mask tier 2
   takes, the loop writes, or only a test build keeps stays a vector of its own, and a scan that makes blocks of one
   has its vector's form (`_v`). Kotlin's are 95 masks a block together, and 5 vectors, the masks tier 2 reads a
   string's parts by.
10. **A level's call that keeps its caller's questions is a call** (`call_asks`). `asks` carries the questions a
   level that may end at a mark asks (Kotlin's: only `semi`'s, the lean parser's `stmt`). A call that keeps every
   one (`asks & keep | set` is `asks`) saves, sets and restores nothing. Kotlin's 16 such calls are plain calls; 94
   set it. rr and JSON have none.
11. **Stage 1's masks are read from the bytes where `lex` reads them** (`interleave::bit_reads`,
   `Bytes::partial_windows`), as the lean lexer reads its bytes: a test of one byte at the byte, a scan, a count, a
   forward mask's bits (`put`, now `wput`), a backward scan (`last_in`, `wlast_in`) and a pass's word (`M[i]`,
   `prev(&M, i, k)`) 64 bytes at a time from the bytes (`Bytes`' windows, the one pass's), `lex` generic over the SIMD
   backend (`generic_lex`). An escape's carry at a window is read back from the bytes: the parity of the run of its
   escaping bytes that ends before it (`esc_back`), simdjson's carry, so an escape reads from anywhere, not only from
   a token's start; `text` is its bytes' windows (`wbyte`). A mask with a read with no such form (a `fill`, whose
   carry is the state since the file's start) stays stage 1's. A scan of the bytes ends where stage 1 may not yet be
   made, so it is made through the scan's end (`s1_past`). A mask a comment names, or only the test build reads, is
   the test build's. Kotlin's stage 1 keeps 21 masks a block, the fills and the masks read with them, and the 5
   vectors tier 2 takes; tier 1's Rust is 304 KB to 782 KB, each mask's window a function of its own (`w_*`). rr's
   and JSON's code is unchanged.
12. **A fill is read from the bytes too** (`fill_back`). Its carry at a window is whether it holds at the byte
   before: read back over the bytes that continue it to the nearest one that starts it (it holds) or ends it (it
   does not), as the lean lexer reads a run. Kotlin's fills are all such runs (a word's, a number's, the gap after a
   `;` or a line break, an `as` type's), not a state from the file's start, as step 11 said. Each fill's and escape's
   window and test is written once (`wf_*`, `af_*`) and each grammar mask's (`d_*`, `at_*`), as the lean lexer writes
   a read once, not inlined into every mask that reads it. Kotlin's stage 1 keeps no mask a block: only the 5 vectors
   tier 2 reads a string's parts by. Tier 1's Rust is 607 KB (304 KB before step 11). rr's and JSON's code is
   unchanged.
13. **`marked` is cleared where a rule may end with it set, and `asks` is set where its context is** (`G::ends_as`,
   `bitlex_rust/asks.rs`). `marked` (a mark taken last, no token or rule's end since) is cleared at a rule's end
   only where its body may end on a mark or take nothing: one that ends on a token had it cleared by that token's
   take. Kotlin's clears are 9 of 191. Within a level `asks` is its entry's (each call restores it), so the values
   each level is entered with are a fixpoint over the calls (`settle`). A call whose setting leaves its caller's values
   as they are is a plain call; a level every caller sets alike sets it at its start; and a rule that sets it alike for
   every call that sets it, and reads none of it, nor do the levels it calls plainly, sets it once at its start, for
   the context (`contexts`), as the lean parser sets `stmt` where a statement or a parenthesis starts. Kotlin's 94
   settings at calls are 27, each a rule's one switch (an `if`'s or a `while`'s condition, a parameter's type, a
   template), and 22 levels and contexts set their own. rr's code changes only in an inlined block's indent; JSON has
   neither.
14. **A mark's mask is taken under its `unless after` one-byte tokens** (`under_unless`). In the gap since the
   grammar token before, no byte of such a token is code: a code byte of one is a grammar token, the one before,
   which the `unless` excludes. So `code & C`, `C` a class of those bytes, is none there, and what it empties goes
   (a `prev` or a `fill` of none). Kotlin's `semi_close` (`close_at unless after ";"`) is then `lf & code`, the line
   break's carrier: a derived mark (`DERIVED`), not tier 1's. A comment's own mark of such a mask holds its `unless`
   too, of the grammar token before it (`carried(before, bits)`, `before_x`), as the interpreter's mask gives it
   (after a `;`, none). Tier 1 writes three of the ten marks: the line break (the lean lexer's `nl`), `as_open` and
   `real`. rr's code changes in `carried`'s parameter and two blank lines; JSON's not.
15. **A gap mark of a fill is asked by tier 2 of the bytes, `code` of the records** (`code_at`, `Bytes::code_fn`).
   A byte is code where no extra holds it, nor a token of a region (a string's, a character's, a name in backticks:
   the rows that read a region, their kinds their own or their first bytes', none another row's). A gap mark whose
   accumulator reads a fill is tested at each byte since the grammar token before's second (`spanned`, given
   `code`), each mask's test written once (`ca_*`), its fills read back from the bytes. Kotlin's `as_open` is so:
   tier 1 keeps no `as_lt` over the gap. Tier 1 writes two of the ten marks, the line break (the lean lexer's `nl`)
   and `real` (its real literal's kind); tier 1's Rust is 573 KB. rr's tier 1 gains `code_at` (its regions none),
   and its calls of `spanned` the question; JSON's code is unchanged.
16. **Tier 2 reads a region token's parts of the bytes at the token** (`span_reads`, `Span`, `field`), as JSON's
   tier 2 reads a string's pieces: tier 1 keeps no mask of the file for them (`Masks`, its 37 vectors, and stage 1's
   vectors of their values outside the regions, are gone). Tier 1 gives a token's span of the bytes (`span`: its
   region by its opener's test, its closer by the region's search, its last byte), whether a region opens at a byte
   (`opens`), and each field's word of 64 bytes (`field`), written as tier 1 wrote it over the token (`field_lines`,
   one window, `win_put`). `bit_reads` reads their masks of the bytes as it reads `lex`'s. Tier 2 scans a field a word
   at a time (`ffirst`, `fbit`); a template's `}` is the first byte after the template's last token that opens the
   region its string resumes (`tail_at`), the regions of the strings open over templates a stack (`string_k`).
17. **The escapes made of `code` are tested in the token loop** (`code_pass_in_loop`), as the lean lexer refuses at
   the token it reads: word `i` of `code` is known once every token that starts before its end is read, so the passes
   over the words step to each block at its start (`code_pass!(b)`) and finish after the loop. A pass that reads an
   earlier one's mask past its word steps a word behind it, that mask in a ring of four words; `code` is the region
   tokens' spans not yet passed (`code_spans`), each word in a ring too. The test build keeps `code_l` and holds each
   word to it. The product's `lex` makes no vector of the file but its records (`lv` too is the test build's).
18. **A rule that is no level ends with the level it is in** (`follow_taken`, `exits_ending`): a mark that follows it
   only where the level it is in ends is that level's, so a call in it keeps the caller's `asks`; only a mark a
   caller takes after it sets them. The precedence ladder's operands no longer set `asks` (12 did): tier 2 switches
   it in 30 places, true where a statement or a class member starts (2) and false at a bracket, a condition, a
   parameter, an annotation or a template (28), as the lean parser switches `stmt` in 30 (true at 6, where its
   statements, a block, a class's or an enum's body, a lambda or a `when`'s body start).
19. **The ladder takes a minimum** (`ladders`, `flag_cond`), as the lean parser's `expr(min)`: each level its minimum
   (by how many operators it takes, the widest 0), each operator its precedence, binding where `p > min`. A generic
   call binds where `5 > min`, as the lean parser's where `min <= COMPARISON`; `as_kwi`, which only the comparison's
   operand takes, is tested of that level (`matches!(min, 4)`), as the lean parser tests a keyword as a name apart.
20. **Soft keywords are numbered by class** (`bitlex_tier2::lower`): in their row's order, as the lean parser lists
   its words (`w`), where they were numbered by their leaf kinds; Kotlin's row lists them by class, the modifiers by
   their nodes. A word set whose numbers run three or more is tested as ranges (`moves::word_pattern`), and the
   choice-of-tokens pass (`classes`) counts a set by its tests, a run one. A modifier is one range, where it was
   `SOFT_WORD[w] != 0 && !matches!(w, …)` of the 17 soft words that are none. The longest word tests left (15 to 17
   alternatives, ranges among them) are where an expression or a statement may start: punctuation, keywords and
   names, as the lean parser's `starts_expression` matches 17 token kinds and two word classes. rr's soft words are
   renumbered too; its code differs only in its numbers and in two tests now one range each
   (`PUNCTUATION[w] != 0 || w == LBRACE`, `TRUE | FALSE | NULL`).
   The words a declaration starts with are three runs (`VAL..=TYPEALIAS | AT | INIT..=CONSTRUCTOR`).
21. **Hard keywords and punctuation are numbered by class too** (`bitlex_tier2::lower`): every keyword row in its
   order (a leaf's kind is its word's, `word_kinds`, whatever its number), and a token mark's words printed as ranges
   (`moves::word_pattern`). Kotlin's rows list them by class: the hard keywords that start nothing, the expression's,
   the declaration's, the loops'; the punctuation's equality operators together and those that may start an
   expression last. `as_kwi` is `CLASS..=DO | PACKAGE`; the statement's start `THIS..=DO | LPAREN..=AT | BY..=OUT |
   SEALED..=ENUM`. No word test in tier 2 lists more than two keywords: the two that list four or more words are
   punctuation's (`IN | AT | LPAREN | FILE`, a postfix's `DOT | BANG_BANG | LPAREN | LBRACK | PLUS_PLUS..=COLON_COLON`).
   rr's code differs in its numbers and its ranges' ends.
22. **`asks` is set true where a body starts** (`asks::lift`), as the lean parser sets `stmt`: a level's own setting
   that ignores its caller's (`r_statement`'s, `class_member`'s) goes up through its callers that make no node (a
   list's loop, `member_ends`) to the one that makes the body's, which sets it for every level it calls; its own reads
   of its old value are constants there (`statements`' `ask_semi(true)` at its end, nothing at 0, is gone). Where that
   level's other callees read what it would set, the call is set instead (`source_file`'s statements: its file
   annotation's type arguments read the root's). Tier 2 sets `asks` true at `statements` (the lean parser's
   `statements`, a block's and a lambda's), `control_structure_body` (a `when` entry's body, as the lean parser's
   `when_expression`, and an `if`'s or a loop's), `class_body`, `enum_class_body` and the file's statements: the lean
   parser's six body starts, the block and the lambda one method here. The callees ignored their callers' value: the
   parse is the same.
23. **Kotlin's punctuation by class, a postfix's and a prefix's tokens runs**: `*` `/` `%` together, then the equality
   operators, then a postfix's tokens (`.` `!!` `(` `[` `::` `++` `--`), a prefix's (`++` `--` `+` `-` `!`), and the
   others an expression may start with (`{` `@`), each set one run. One word test lists four words, of three classes
   (`IN | AT | LPAREN | FILE`: a hard keyword, two punctuation's and a soft one), which no order of classes makes one.
24. **A word set of three classes stays a `matches!`**: `type_projection`'s start (`IN | AT | LPAREN | FILE`, a hard
   keyword, two punctuation's and a soft one) is no run under any order of classes. A table of it (`WORD_SET_0[w] !=
   0`, as the lean parser's `modifier(code)`) is a load, where the compiler makes the `matches!` of four words under
   128 a test of a constant mask; renumbering them into a run breaks the classes' runs that hotter tests are (a
   statement's start, a name's). Kept as the `matches!`.
25. **A token's window tests ask only its own bytes**: a test over a token's bytes (`count_in`, `first_in`, `any_in`)
   calls its window function with the token's range as a mask (`upto(p, l)`), ANDed into its first conjunct before the
   early exit, so a byte in the window past the token starts no fill. Measured alone on the corpus (491 files, 3.6
   MB, same run, best of 15): tier 1 743 ms against 735 ms before 0703, no change.
26. **An escape at a number's end is tested there, once**: an escape with a factor that is a run's last byte (`Y &
   ~next(Y)`, joined or narrowed), or `prev` of one, holds only at that byte or the next. It is tested by bytes at the
   token's end (`l`, `l + 1`) in the rows whose run is `Y`, as the lean lexer's number reader refuses at the number's
   end. Kotlin's three (a digit after a binary number, a word touching a real number, a number that ends in `_`) leave
   the name arm; the other rows assert, in a test build, that it holds at none of their bytes. rr and JSON have none.
   Measured (same run): tier 1 243 ms against 735 ms before 0703 (3.0×), the AOT parse 507 ms against 982 ms (1.9×).
27. **A character is decoded only at a byte past ASCII**: a window test's early exit counts a mask of the bytes
   (`d_`) or a fill (`wf_`) as many classes, so a name's "a byte above 0x7f, not a name's letter" reads `cls_high`
   first and `d_uni` only where it holds, as the lean lexer's `name_end` decodes only at a byte from 0x80.
28. **An escape at a row's start is tested at its token's first byte**: one with a row's `start` as a factor holds only
   at the first byte of that row's token. It is tested there, its other factors alone (the row holds its start), and
   the other rows assert in a test build that it holds at none of their bytes. The operator's "a triple's last byte
   starts a pair" is `at_two_start_three(s) && at_two_start_two(s + 2)`, its own bytes as the lean lexer's `match`
   reads them, no longer a scan of the pair masks over every operator's and number's bytes.
   Measured with 27 (same run, m1max, best of 15): tier 1 45 ms against 229 ms after 26, the AOT parse 278 ms against
   469 ms. rr and JSON are unchanged.
29. **An escape made of `code` is tested at its bytes, its context read back** (in place of step 17's masks over the
   words): its factors of the bytes alone (`semicolon`, `high`, `prev(gt)` and a blank, a `/` or a `\r`) are a word's
   window as the loop passes the word, and at each byte they hold at the whole escape is tested there
   (`rca_*`, `rcaf_*`), its fills read back from the byte and `code` of the region spans the loop has read (`code_of`),
   as the lean lexer looks at a `;`'s line only at the `;`. No fill is carried from word to word, and every span is
   kept. An escape with no factor of the bytes alone keeps step 17's form. Kotlin's three ("a name's letter past ASCII
   touching a number", "a `;` after a line break and a comment", "a blank, a comment or a `\r` after a type the lean
   parser may split", the last now with refusal tests of its own). Measured (same run, m1max, best of 15): tier 1
   34 ms against 45 ms after 28.
30. **The token loop reads a block's cheap masks from its word, and a pair set by tables**: in a row of code (and the
   loop's head), a read of a mask of four classes or fewer and no escape is its block's word (`k_M`, made once a block
   at the loop's head), then windows past the block (`brun_last`, `bfirst_in`), as the lean lexer's `lex_blocks`
   reads a name's run and a gap's line end of its block's `word` and `eol`. A mask of fills or more classes stays a
   window where a token asks it: made every block, the number's and the decoding's masks made tier 1 4.4 times
   slower. A set of four texts or more (a `pairs` mask, or one side of its `&`) is tested by a table a byte position,
   each byte's bits the texts with it there (`AT_TWO_START_2_0[src[q]] & AT_TWO_START_2_1[src[q + 1]]`), as the lean
   lexer's `ONE[c]` and `FOLLOW[next]`. Measured (same run, m1max, best of 15): tier 1 32.6 to 33.9 ms against 34.0 to
   34.8 ms after 29.
31. **Stage 1 runs only in a test build where the product reads none of it**: where every field of `Stage1` but
   `refused` is a test build's and no escape of stage 1's sets `refused`, its runs (at the loop's head, in `s1_past`,
   after the loop) are the test build's alone, its checks' masks. The token loop reads the bytes, as the lean lexer
   reads only the block masks its loop asks; Kotlin's stage 1 made every class and fill of the grammar a block, read by
   nothing. Measured (same run, m1max, best of 15): tier 1 22.9 to 23.7 ms against 32.0 ms after 30; the whole AOT
   parse 263 to 285 ms against 264 ms, unchanged.

What stays: tier 1 computes two marks into the token's code, as the lean lexer does: `head_ctor`'s line break (its
`nl`) and `real` (its real literal's kind, a count of a number's bytes). The lookahead of `as_split` and `is_split`
stays a function a state (`next_*`).
