# 0673: `.rerail`'s tier 2 takes the lean parser's forms: peeks, statements, aggregates

Status: in progress, steps 1 to 9 done (the user, 2026-11-24: "address all of these", ten gaps between tier 2 and the lean
parser). Follows 0668. As in 0668, a step is judged by one question only: is the generated code closer to the lean
parser's shape? Speed decides nothing.

## Intro

After 0668, tier 2 (`rr_aot/src/tier2.rs`, written by `bitlex_rust` from `rr_parser/rr.bitlex`) reads expressions as
the lean parser (`rr_syntax/src/lean.rs`) does, but it is still about 3.7 times the lean parser's length. The gaps that
remain are these:

1. **Marks instead of looking ahead.** Each mark has four functions (`m_`, `h_`, `take_`, `mark_`), pattern functions
   (`pat_`), a cache and a `taken` bit. The lean parser peeks instead: `self.is_word_at(0, b"at") &&
   self.kind_at(1) == STR`, `self.on_line(&mut seen)`.
2. **Statements and directives.**
   - The lean parser reads `.` and the word after it, opens the directive's kind, and runs one `match`.
   - Tier 2 opens a `setting`, and each directive's `_tail` retags it.
   - `setting`'s body is written twice, once under `if self.m_line()`.
   - The rest of a line is a match over 31 punctuation words.
3. **JSON.** The lean parser reads `{…}` as balanced braces and checks a policy document with `serde_json`. Tier 2
   parses JSON's grammar.
4. **Cover forms instead of going back.** The user chose option C of the page "Tier 2's going back": a peek past the
   matching `)` (step 5).
5. **Aggregates.** The lean parser has one `aggregate` and `agg_part`. Tier 2 has ten rules, the remembered level word
   and a mark.
6. **A bare term is checked late,** at the statement's end (`settled`), where the lean parser fails at once.
7. **Tokens as functions.**
   - Operators are rules of their own.
   - A token kind has three tests (`at_X`, `at_X_at`, `at_X_rec`), where the lean parser writes `self.kind() == STR`.
8. **Seven decisions number their arms.**
9. **Runtime helpers.**
   - Generic takes whose const parameters gate marks and refusals.
   - `settle`, `flush` and `pass_comments`.
   - The `refusing` mask, and `deeper()` in every nested rule.
10. **The device form, `tier2_loop.rs`, has not been compared with the lean parser.**

## Step 1: a mark of tokens is a peek (done)

A mark taken at a token sets its own `taken` bit and the bits of the marks declared before it. The bits clear when
the token is taken. A test of a mark at a token reads its bit, so that a mark taken there no longer holds. That order
is the line breaks' business: a statement that ends at a line break takes `line`, and the next statement at that token
does not see it.

A mark of tokens alone (`tokens "fix" identifier`) needs none of this where no test can read its bit. The writer
(`bitlex_rust::peeks`) finds such marks. A mark is a peek when all of these hold:

- It is hidden: no take of a token asks it (`open_mark`).
- It has no question (`drops`, `loses`, `refuses`), does not settle, and passes nothing.
- Its definitions are runs of tokens with no repeat.
- Every mark that may set its bit is taken only right before a token is (`loose_marks`, `takes_first`). These are the
  marks declared after it, and those that pass it. A token is taken first by a token, a field, a wrap or a relabel, or
  a level whose first element is one. Between the take of the mark and the take of the token, no test comes, so no
  test reads the bit.

A peek is one condition, written where a decision tests the mark: `self.word() == w::FIX_40 &&
self.at_identifier_at(self.i + 1)`. Where a rule takes it, it is `if !(…) { return self.refuse(); }`, except right after
the decision that tested it (`no_retests`), as the lean parser takes the token it peeked at. A peek has no function, no
cache, no bit and no pattern function.

`rr.bitlex` declares `newline` beside `line`, before the marks of tokens. A statement takes `newline` where it ends,
without taking the token, so a mark declared before it could not be a peek. Moved, it no longer passes the marks of
tokens, and taking one of those passes it only where the token is taken next.

Eleven of rr's marks are peeks. Left as marks:

- `float_at`, `line` and `newline`: tier 1's and the gaps'.
- `json_bad`: a JSON level refuses it.
- `end_on`, `no_value` and `io_bare`: not hidden; each refuses the file where no rule takes it.

rr's `tier2.rs` is 3,822 lines after this step, from 4,219; the loop form loses about as much. One of Kotlin's 41
marks, `dotted`, is a peek; the others do not meet the conditions above.

## Step 2: a token kind is `self.kind() == t::K`; a choice of tokens is one take (done)

Gap 7. The writer (`bitlex_rust`) has no `at_K`, `at_K_at` or `at_K_rec` functions any more.

- **Kind tests.** A token kind's test is written where it is asked, as the lean parser's `self.kind() == STR`:
  - `self.kind() == t::IDENTIFIER`, or `matches!(self.kind(), …)` where a kind has several record kinds;
  - a row's hard words excluded with `self.word() != w::…`;
  - `self.kind_at(self.i + 1)` in a peek;
  - the record's own kind in a gap's question.

  `kind` and `kind_at` are runtime helpers beside `word` and `word_at`, using the record kinds' constants (`t`).
- **Choices of tokens.** A choice of tokens alone, no level, each token a word whose leaf is its own kind
  (`additive = '+' | '-'`), has no function (`anon_choice`). Where it is used it is one take whose leaf's kind is the
  word's (`ANON`, a table by word), as the lean parser's `anon()`:
  `self.tk(matches!(self.word(), w::PLUS_28 | w::MINUS_27), ANON[self.word() as usize], f::OPERATOR)`. A choice of
  one token is that token's take. rr's `additive`, `multiplicative`, `comparison_operator`, `angle_close` and
  `paren_close` are gone. `punctuation` holds a `#` word taken as an identifier, so it stays a rule.

## Step 3: a token tested once more: no peek its arm's word covers, and no numbered arms (done)

Gap 8.

- **A covered peek.** A peek whose first words all go to its own arm, as word tests, has no test of its own: where it
  holds, its first word does, and that word takes the arm. One exception keeps it: another mark that may hold at those
  words and takes another arm, because there the marks' order decides. So `code_part` tests `{` once, and
  `braced_column` decides between the name and the raw braces.
- **An `if` chain.** A decision numbered its arms (`let arm`, then `match arm`) wherever one arm was reached by
  several tests. The tests are asked in this order: the marks, then the words (no two words hold at once), then the
  kinds. Where each arm's tests are one run in that order, the decision is an `if` / `else if` chain, one condition
  an arm, as the lean parser writes it. The words can go in any order, so the word an arm before takes is put first and
  the word an arm after takes is put last.

None of rr's decisions numbers its arms now. `block_body` is `if` the `.end` peek, `break`, `else if` a statement's
first token, the statement; `agg_value` is `count`, then its other values, then a name.

rr's `tier2.rs` is 3,573 lines. Tests for steps 1 to 3: rr 58, the 8 corpus tests, Kotlin 70, JSON 26,
parser_tests 95, Bitlex 11, clippy.

## Step 4: a statement is decided by its `.` and the word after it (done)

Gap 2. `rr.bitlex`'s statement is the lean parser's `statement`:

- `statement` is a `.test` or `.fixtures` block, else a `directive`. `directive` takes one arm per directive, each by a
  peek at `.` and its word (`decl_on`, `io_on`, `policy_on` with its `{`, …), else a `setting`. Each directive is a node
  of its own that starts with its `.`. There is no `setting` retagged by a `_tail` rule, and no `setting` body written
  twice.
- A `.end` outside a block is refused where the file's statements are read (`end_on refused`), as the lean parser's
  `Err(())`. `refused` is a grammar element with no way on (`Gram::Alt` of none). Its state refuses every token, a
  nested level's too (`bitlex_tier2`).
- A block's statements do not hold a block: `block_statement` refuses `.test` and `.fixtures` there, as the lean
  parser's `test` does, with no flag.
- A setting's words are read as the lean parser's are: `{` a JSON object, any other token a word, to the line's end.
  The lean parser asks whether a word is on its line from the first byte of the word before. So a JSON object over
  lines ends the setting. `json_flat` holds at a `{` whose tokens, its `}` too, have no line break before them. Where it
  holds the setting goes on; a JSON object where it does not is the setting's last word.

  A run of a token mark may say `without M` (or `with M`) of its tokens (`syntax.rs`). The run's tokens are tier 2's,
  so `line`, which `json_flat` reads, stays a mark tier 2 asks of the bytes (`lazy_marks`): a token mark past the
  code's bits does not make the marks it reads coded (`bitlex_masks`). The JIT asks a lazy mark at any record a token
  mark reads (`Lazy`, `lazy_mark` at a record), as the Rust writer's `bit_at` does.
- A covered peek (step 3) is kept only for a mark tested after it that takes another arm. One tested before it is asked
  first either way. So `statement` tests `.end`'s peek, then the `.` word, and no directive's peek.

The old form of the token rules' test strips the directive marks: it compares tier 1's starts and lasts only. The
ladder generator picks no arm that cannot be derived (a `refused` one).

rr's `tier2.rs` is 3,582 lines, about as before: eleven directive nodes and their peeks replace the `_tail` rules. Kotlin's
generated code is unchanged. Tests: rr 58, the 8 corpus tests, Kotlin 70, JSON 26, parser_tests 95 (the ladder's four
new cases of a setting's JSON), Bitlex 11, clippy.

## Step 5: a `(` is decided by the token after its `)`; one aggregate; a bare term fails where it ends (done)

Gaps 4, 5 and 6. The lean parser reads a group, and goes back where an operator or a comparison follows its `)`. Tier 2
reads ahead instead (option C): a mark of tokens may scan to the matching `)` (`nest ("(" to ")")`) and test the token
after it.

- **A literal at a `(`.** `literal` is the lean parser's: a negation, a group, else a term and its tail. Three marks pick
  the reading at a `(`:
  - `paren_term`: an operator or a comparison after the `)`. The literal is a term.
  - `paren_tilde`: a `~` after it. The literal is a pattern, and the subject in parentheses is `paren_subject`, nested
    ones too (`paren_closes`).
  - Any other `(`: a group.

  The readings `parenthesized` and `parenthesized_strict`, `group_term`, `group_body` and `group_conjunction` are gone.
- **A bare term.** It fails where the literal ends, as the lean parser's `if !call { return Err(()) }`. The rule says
  so with `ending literal K refused`, one line for each kind a term can be but a call. The Rust writer joins them into
  one test of the kind made last (`made_kind`). `(f(x)) ~` is refused the same way (`ending paren_inner call refused`).
  So nothing is checked at the statement's end: `provisional` and `settled` are gone, from the Bitlex syntax, the
  reference, the interpreter, the JIT and the Rust writer.
- **Aggregates.** `aggregate` is the lean parser's: `count` and its body, else its word, its value, and `by` with its
  keys, `sep` and the body. A value or a single key holds no named argument (`no_named`, set in `aggregate` and kept
  in everything inside), as `agg_part`. A term decides an aggregate with three peeks:
  - `count_on`: `count :`.
  - `agg_call`: an aggregate word, `(…)`, then a `:`. The lean parser reads a call there and goes back.
  - `agg_word`: an aggregate word and a token that can start a value.

  Ten rules are gone: `agg_argument_list`, `agg_value`, `count_value`, `word_in_value`, `more`, `more1`,
  `aggregated`, `word_agg`, `count_agg`, the `aggregate_value` reading. So are the remembered level word, and the
  marks `empty_parens` and `no_value`.
- **An I/O directive's relations.** They are none where a `.` is next (`io_none`), else at least one, as the lean
  parser's `list(DOT, …)`. The refusing mark `io_bare` is gone. `io_none` is declared before every mark a statement
  tests at its `.`: taking it sets the taken bits of the marks before it, and a directive's peek must not lose its own.
  Kotlin's `bt_cref` still uses refusing token marks.
- **Which marks are peeks.** Step 1 ruled out a peek wherever a mark at or after it was taken anywhere but right before
  a token. That was coarser than the rule it stands for. A peek's taken bit is read only by a test of that peek, at
  the same token, after a later mark's take and before the token's. The writer now computes exactly that
  (`set_then_asked`):
  - each element's first questions (`first_asks`);
  - what each rule may be asked after it ends (its callers', to a fixed point);
  - the bits each take sets that a test may then read.

  Every rr mark of tokens is now a peek but the five that scan to a matching bracket (`json_flat`, `paren_term`,
  `paren_tilde`, `paren_closes`, `agg_call`), and `json_bad`.
- **Decisions.** No decision of rr's numbers its arms. Two changes to step 3's `if` chain make this so:
  - A word or a kind of another arm goes before the last run of marks when none of those marks can hold at it. A
    kind's tokens are its keywords rows' words, soft ones too. So `literal` tests its peeks, then `!`, then `(`, then
    the term's tests; `minus` tests `float_at`, then a number, then the operand's.
  - A choice of tokens taken as one kind (`aggregate_word as variable`) is one take of any of its words, as `anon()`.
- **The reference.** An option whose first token is next is taken (`predicts`). That now leaves out a way that
  `unless F` guards while `F` is set, as the machine's decision does. So `sum x : {…}` is an aggregate there too, not a
  named argument `x: {…}`.

Tests:
- The deep-nesting test nests 30,000 deep. The JIT bounds the calls open by the stack's bytes (`STACK_BYTES`), not
  their count: a paren's frames are smaller now, and 3,000 deep fits, parsed as the engine's tape.
- `a_reading_is_checked` reads JSON's grammar with a reading added, since rr has none.

rr's `tier2.rs` is 3,293 lines, from 3,582. Kotlin's is 8,503 lines, from 9,099: 15 of its marks are peeks now, and one
fewer decision numbers its arms (14). Tests: rr 58, the 8 corpus tests, Kotlin 70, JSON 26, parser_tests 95, Bitlex 11,
clippy.

## Step 6: a policy document is JSON's grammar read into one leaf (done)

The lean parser reads a policy document's `{…}` as balanced braces into one `json_object` leaf, then checks that its
text is JSON. Tier 2 read JSON's grammar with `merge json_object` on every token: five rules, each written twice, one
for the outermost object and one for those inside.

Bitlex now has `leaf K R`. Rule `R` reads the tokens, and they are one leaf of kind `K`, from the first token's start to
the last token's end. rr's policy document is `document: leaf json_object json_document`, and the JSON rules are JSON's
own: `json_document`, `json_member`, `json_value`, `json_array`, with no merges.

- **The reference and the machine.** Both read `leaf K R` through one rewrite (`desugar_leaves`, in Bitlex's
  `syntax.rs`). It makes two derived copies of the rules `R` reaches. The first takes its first token `as K`, and the
  second merges every token into `K`. A refusal (`refuse M in L`) on a level reaches its copies too. A rule that
  `leaf` reaches may make no node.
- **The Rust writer** keeps `leaf` as the lean parser's `json` does: `self.leaf_open(); self.json_document()?;
  self.leaf_close(k::JSON_OBJECT)?;`. The rules `R` reaches take each token with `pass` (`word_pass` for a word). A
  pass refuses a token as a take does, and writes no leaf and no comment. `leaf_close` makes the one leaf, and the
  comments inside it are its text, as a merge's are. Reading JSON's grammar is the check that lean does with
  `serde_json`. The loop form (`one_loop`) writes the same.
- **No empty `else`.** A decision whose default did nothing but clear `marked` kept an empty `else {}` once
  `no_marked` removed that line. `no_empty_elses` drops it: 25 of them in Kotlin, 6 in rr.

rr's `tier2.rs` is 3,308 lines; the JSON rules are four, each written once. Kotlin's is 8,478 lines, from 8,503 (the
empty `else`s). Tests: rr 58, the 8 corpus tests, Kotlin 70, JSON 26, parser_tests 95, Bitlex 11, clippy.

## Step 7: one take, a leaf's text checked once, depth counted where the grammar recurses (done)

Gap 9 is the runtime's helpers. The lean parser has one `p` and one `id`, checks a policy document's text once it has
read it, has no depth count, and has no helper it does not call. Tier 2 now does the same where the grammar lets it:

- **A leaf's refused marks are checked once.** Every `refuse M in L` of rr's is under a `leaf`, so `leaf_close` checks
  the leaf's tokens for the marks (`leaf_refuses`). That is the lean parser's `serde_json` check, after reading. No
  call saves the marks refused, and no take tests them, so the `refusing` field is gone. A grammar that refuses a mark
  outside a leaf keeps the general mechanism. No rule outside a leaf may call a rule under one, since that rule's
  takes write nothing: the writer refuses such a grammar.
- **One take.** `takes` gives each take the tests its token needs (`M`: a mark no rule takes; `F`: a refused mark). In
  rr every take is `M` false and `F` false, and in Kotlin every one is `M` true and `F` false. Where a grammar's takes
  are all one pair, `one_take` drops the const parameters, and `take` asks only what that pair asks. rr's `take` is now
  the lean parser's `bump`: the span, the advance and the last end.
- **The depth is counted where the grammar recurses.** A call outside every cycle of the rules' calls nests no deeper
  than the grammar is long, so only a cycle's calls need counting. In the grammar's order, a level counts the depth
  (`deeper`) if it is still on a cycle once the levels before it that count are taken out (`depth_cuts`). If a cycle
  holds only rules that are not levels, every level counts, as before. rr counts at 11 levels, down from 19, and
  Kotlin at 6. The loop form counts at the same levels.
- **No helper nothing calls.** `drop_uncalled` matched only the runtime's text as written, so a helper that an earlier
  replacement had edited was never dropped. `no_dead_methods` drops any method that nothing calls, in the final text.
  That removes rr's `settle` and `open_mark`, and Kotlin's unused `bump`.

`flush` and `pass_comments` stay. They are the lean parser's comment cursor (`push`'s `next_comment` test, and
`comments_before`), written once each: the first places the comments before a node, and the second moves past them.

rr's `tier2.rs` is 3,237 lines, from 3,308, and `tier2_loop.rs` is 3,968, from 4,036. Kotlin's `tier2.rs` is 8,394,
from 8,478. Tests: rr 58, the 8 corpus tests, Kotlin 70, JSON 26, parser_tests 95, Bitlex 11, clippy.

## Step 8: the loop form's places in the calls form's shapes; an exit no arm can hold at is not tested (done)

Gap 10: `tier2_loop.rs`, the device's form, had not been compared with the lean parser. It has to be one loop: a place
is a part of a level, a call pushes the place it returns to and jumps, and a level's end pops that place. Within that,
each place's code can be the lean parser's, as a rule's is in the calls form. It was not, for three reasons.

- **The shaping passes ran on the calls form only.** `ifs_and_whiles`, `no_marked` and `no_empty_elses` now run on both
  forms. So do the new passes below. The loop form has 53 fewer `self.marked = false` lines, `if` and `while` for
  one-arm matches, and no empty `_ => {}` arms.
- **Nothing after a `return`.** A place's arm that refuses (`return self.refuse();`) was followed by its jump.
  `no_code_after_return` drops what follows a `return` in its block.
- **A level's end is one pop.** It was a comment, `let ret = …`, then `st = ret;`, 26 times. Where the call saved only
  the place, it is now `st = stack.pop().unwrap_or(0) as usize;`, as a rule's `return`.

The comparison also found two gaps in the decisions, which both forms share:

- **An exit no arm can hold at is not tested.** `test_block` tested the twelve statement starts (`.end`, `.test`, …)
  before `exactly` and `uses`, and leaving did what passing every arm did. An exit gets no test where three things hold:
  leaving does what no arm does (`exit` is `none`); the mark has no questions and no `marked` or `asks` gate; and no
  arm's test can hold at its first words. That means no arm word among them, no arm kind whose tokens may be one of
  them, and no arm mark tested after it that may start with one. `test_block` is now lean's: `if exactly { … } if
  uses { …; while , { … } }`.
- **A kind joins its arm's earlier run.** In `functor_declaration`'s and `declaration`'s rest of line, `punctuation`
  is reached by its words, and again by `hash_word` after another arm's kinds. The loop form's arm (a call, then a
  jump) was too long to write twice, so the decision numbered its arms. A kind now joins its arm's earlier run where
  no test between can hold at its tokens: no mark, no word that is one of its tokens' words, and no test of a kind
  with the same tokens (a token has one kind). The arm is then one condition, `matches!(self.word(), …) ||
  self.kind() == t::HASH_WORD && …`, as the lean parser tests an arm's tokens together. No decision of rr's numbers its
  arms in either form. Kotlin has 12, from 14. Its `statement` is decided by marks that are not peeks, in an order
  its grammar sets, which is outside rr's ten gaps.

One place jumps to another and does nothing else (`6 => { st = 8; … }`). It is the start of `block_body`'s loop, and
it stays.

rr's `tier2.rs` is 3,210 lines, from 3,237, and `tier2_loop.rs` is 3,716, from 3,968. Kotlin's `tier2.rs` is 8,317,
from 8,394. Tests: rr 58, the 8 corpus tests, Kotlin 70, JSON 26, parser_tests 95, Bitlex 11, clippy.

## Step 9: a token tested is taken with no test; `punctuation` is lean's `anon()`; a repetition is a `while` (done)

What gap 7 left:

- **A token tested is taken with no test.** A take after an `if`, `else if`, `while` or guard that tested the same
  condition tested it again, as in `if self.kind() == t::STRING { self.tk_as(self.kind() == t::STRING, …) }`. There
  were 13 in rr's `tier2.rs`, 12 in its loop form, and 15 in Kotlin's. `no_taken_retests` passes `true` there, as
  `one_arm_a_take` does for a word's arm. None remain.
- **`punctuation` is the lean parser's `anon()`.** A choice of tokens, each its own leaf, was taken where it is used
  (step 2) only if every alternative was a word. `punctuation` has one more alternative, `hash_word as identifier`,
  so it was a function of 32 arms. `anon_choice` now also takes one alternative that is a kind of token taken as
  another kind. Its take is the lean parser's `anon()`: a listed word is its own leaf (`ANON`), and any other token
  of the kind is an `identifier`. The function is gone.
- **A repetition is a `while`.** `loop { if C { … } else { break; } }` is now `while C { … }` (`loops_as_whiles`), as
  lean's `pragma` reads its strings. The `loop`s that remain end at a mark (`m_line`, `m_newline`) before testing
  their tokens. The lean parser writes those as `while` with the line's test in the condition, and here the mark's
  test is the first arm of the decision inside the loop.

rr's `tier2.rs` is 3,079 lines, from 3,210, and `tier2_loop.rs` is 3,614, from 3,716. Kotlin's `tier2.rs` is 8,305, from
8,317. Tests: rr 58, the 8 corpus tests, Kotlin 70, JSON 26, parser_tests 95, Bitlex 11, clippy.
