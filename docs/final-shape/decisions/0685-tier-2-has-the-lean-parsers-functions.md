# 0685: Tier 2 has the lean parser's functions

Status: being built (the user, 2026-11-25: "the rocks need to be done because qualitatively we have better bones in
the generated parser, not because you set some technical criteria"). Follows 0681. The bar: rr's generated parser
(`rr_aot/src/tier2.rs`) read beside the lean parser (`rr_syntax/src/lean.rs`) has its functions and their shape.

## Intro

0681 made tier 2's code moves, but the generated parser kept the grammar's functions: one per rule. The grammar has
rules a parser written by hand does not: `directive` is a choice of ten node rules, each of which opens its node,
takes the `.` and its word, reads its body and closes. The lean parser has one `statement` that reads the `.` and the
word, picks the node's kind from a table, opens it once, and one `directive` that matches the word to each body. The
same holds of `literal` (seven rules: `literal_tail`, `paren_subject`, `paren_inner`, `negation`, `comparison_tail`,
`pattern_tail`), of a declaration's and a functor's modifiers (one loop written in each, the lean parser's one
`rest_of_line`), and of a token class (every punctuation, 31 words in each test where the lean parser asks
`anon()`).

So the generated parser has 65 rules' functions to the lean parser's 45, and its code repeats what the lean parser
says once.

## Decision

The generator writes the lean parser's functions, not the grammar's rules:

1. **A rule with one caller is its caller's code.** A rule called from one place, not a level (`nested`, which saves
   and counts its caller's state) and not the root, is written where it is called, its node opened and closed there,
   as the lean parser's `statement` opens `Include` in place. A call is a move (`Op::Call`), so the inlining is of
   moves.
2. **A choice of nodes opens its node once.** Where each arm of a decision opens a node and closes it last, the
   node's kind is a table on the decision's word (`let kind = match …`), the node is opened before the decision and
   closed after it, as the lean parser's `statement` and `directive`.
3. **A token class is one test.** A test of every word of a choice of tokens (`punctuation`) is that choice's test,
   not its words.
4. **Code the parser repeats is one function.** Moves that are the same in several places (the modifiers' loop) are
   one function, called from each, its differences (a field) its parameters, as the lean parser's `rest_of_line`.
5. **A leaf grown a token at a time is one leaf over the tokens.** Where the rules take a token as a leaf of kind `K`
   and then only join tokens to it (merges, and calls of functions that only merge), the run is `leaf_open`, the
   tokens passed, `leaf_close(K)`, as the lean parser scans (`braces`, `raw_line`) and writes one `leaf_over`.
6. **Arms whose code is the same are one test.** Once the takes are final, arms next to each other whose code is the
   same are one arm, a last arm whose code is the `else`'s goes, a `match` of one arm is an `if`, and a loop whose code
   is `if d { x } else { break }` is `while c && d { x }`, as the lean parser's `while on_line { … }`.

Each step is judged by the generated functions beside the lean parser's, and its record names which functions
changed and how.

## Steps

1. Calls are moves; single-caller rules are inlined. (Done: `Op::Call`, `Op::Open`, `Op::Close`; `G::rule` keeps each
   rule's code as it is written at a call (`inline`), and `G::inlined` writes a rule called from one place there, the
   smallest first, up to `INLINE_LINES` (20): a longer one is an idea of its own, as the lean parser's `violation`
   and `raw_line`. rr's calls form has 52 rules' functions, from 65 (13 gone: `edit_declaration`,
   `type_declaration`, `pragma`, `policy_document`, `fixtures_block`, `code_column`, `tag_column`, `fix_line`,
   `comparison_tail`, `functor_call`, `lattice_functor`, `parenthesized_expression`, `value`): `directive`'s `match`
   holds the edit, type, pragma and policy directives' code, as the lean parser's `directive`;
   `violation_declaration` writes its code, tag and fix lines in place, as the lean parser's `violation`. Kotlin's
   the same way. The loop form keeps its places. Not yet the lean parser's: `directive`'s arms each open their node,
   and the declaration, functor, violation, lattice and I/O directives, a setting and the literal's parts are still
   functions of their own.)
2. The node's kind as a table; open and close hoisted. (Done: after the rewrites, where a decision's arms each open a
   node or only call a node's rule of one caller, that call opens and closes the node and the rule's method reads
   what is inside it (`G::opened_by_caller`, `moves::open_arms`), as the lean parser's `violation`; then a `match`
   whose arms each open a node and close it last opens it once (`moves::open_once`): `let kind = match …` of each
   arm's kind, `self.open(kind)`, the `match` of the arms' code, `self.close()?`. A level that keeps no state of its
   caller's (no depth counted, no word kept) is written in its caller too. rr's `directive` is now the lean
   parser's `statement` and `directive` in one: the kind table on the word after the `.`, one open, the bodies, one
   close; `violation_declaration`, `lattice_declaration`, `io_directive` and `setting` read inside the node
   `directive` opens. Kotlin's code has no such `match`: unchanged.)
3. Token classes. (Done: each choice of tokens of several words has a table of its own, named for it (`G::anon_table`:
   rr's `PUNCTUATION`, `COMPARISON_OPERATOR`, `ADDITIVE`, `MULTIPLICATIVE`; Kotlin's `SOFT_WORD`,
   `ASSIGNMENT_OPERATOR`, `INTERP_SOFT`), and the words chosen in a rule's own `|` the one `ANON`. A test of a
   choice's words is its table's (`Cond::Class`, `moves::classes`): `PUNCTUATION[self.word() as usize] != 0`, not
   31 words, the few words besides or but listed after it (`… != 0 && !matches!(self.word(), w::LPAREN | w::RPAREN)`);
   a take under it takes untested. An arm of `a || b` whose code is `if a { x } else { y }` is two arms (`or_arms`),
   so a setting's, a declaration's and a functor's tail ask a punctuation, then a `#` word, once each, and take the
   `#` word as `bump_as(k::IDENTIFIER)`. A negated word test is `!=` or `!matches!`. The changes: rr's and Kotlin's
   lists of a choice's words are its table's test, its takes' kinds its table's, in both forms.)
4. Repeated code as functions. (Done for loops: `G::repeated`, before the inlining, finds a loop of 6 lines or more
   (`REPEATED_LINES`) that is the same in more than one place, with the move after it where each place has the same
   (a must-hold of the line's end), and makes it one function called from each, named for what it reads
   (`moves::what_of`: its one field, else its one leaf's kind, else its one rule). Not a loop that reads a local of
   the code around it. rr: `modifiers` (a declaration's and a functor's tail to the line's end, the lean parser's
   `rest_of_line`) and `raw_texts`; Kotlin: `delegation_specifiers`, `type_names`, `quests`, `trail_brackets`. With
   the modifiers one call, the declaration and functor directives are short enough to be `directive`'s arms, as the
   lean parser's. Then `G::lists`: a run of moves around a loop (up to two moves before it, one after) that the rules
   repeat but for the rule it calls and the field it sets (`moves::abstracted`), of 4 lines or more
   (`LIST_LINES`), is one function whose parameters are those (`field: u16`, `item: fn(&mut Self) -> R`), called
   with each place's (`self.list(f::RELATION, Self::io_relation)?`), as the lean parser's `list(close, field,
   item)`. rr's relations, heads, literals, columns, arguments, keys, edits and operations are one `list`, JSON's
   members and values one `list_1`; Kotlin's parameters, conditions and declarations three.)
5. One leaf over a scan (rock 3). (Done: `G::leaves`, after the rewrites, finds for each kind the merges join tokens
   into the functions that only merge into it (`moves::merges_only`, the most there are, since they call each other)
   and each run that begins the leaf and then only merges (`moves::one_leaf`, the outermost first), and writes it
   `self.leaf_open();`, its takes `pass_as`, `self.leaf_close(K)?;`, for a kind each of whose merges is in such a run.
   `leaf_close` is the lean parser's `leaf_over`: the marks a grammar's `leaf` refuses are checked by a call of their
   own, `self.leaf_refuses(M)?;`, where the grammar says so (`G::leaf_end`), as the lean parser checks a policy
   document is JSON, not at every leaf. rr's `raw_line`, `io_parameters`, `raw_braces` and `json` are each one leaf
   (`RAW_TEXT`, `JSON_OBJECT`) over the tokens they read; the merges left join two tokens (`.input`, `1.5`, Kotlin's
   `as?`, `>=`, `return@`).)
6. Arms joined (`moves::joined`, the last rewrite, after `classes` and `G::leaves` made takes alike). rr's
   `raw_line` reads `if A || B || … { pass } else { refuse }` then `while on_line && (A || B || …) { pass }`, not a
   `match` of two arms of the same code in a loop; Kotlin's loops whose arms all `break` but one are `while c && d`
   (`source_file`'s annotations, `import_header`s), and `if c {} else { y }` is `if !c { y }`. Kotlin's tier 2 is 300
   lines shorter; the loop form changes alike.)
7. Known tokens taken untested (`moves::known_takes`, before `joined`). A take whose token the code before it tested
   (an `if`'s, a `match`'s pattern or guard, a `while`'s test, of this token or one further on, past moves that take
   no token: an open, a field, a retag) takes it untested: rr's `directive` arms take the `.` and their word as
   `self.implied()?` (`bump_as(k::DOT)` where the `.` is stored), as the lean parser's `statement`; `@`, `code`,
   `options`, `tag`, `fix`, `safe`, a `<` `>` pair and `()` likewise. `id` keeps its test, as the lean parser's.
8. Arms joined across arms they exclude, and methods written in their one caller once short (`moves::joined`,
   `G::late_inline`). Two arms of the same code with arms between join where none between passes the later's test,
   or none passes the earlier's (`moves::disjoint`: the word at one place, of no common word, or one held scan's
   close asked of no common word); the joined arm takes the place that keeps the order. Then a method the rewrites
   made short (up to `INLINE_LINES`), called from one place, of no parameters, locals, depth or early `Ok`, and not
   one leaf over a range (`moves::one_leaf_only`: the lean parser keeps `raw_line` and `json` as functions), is
   written there, and its caller's arms joined again. `if A || B { x; match self.word() { A => …, B => …, _ =>
   refuse } }` is an `if` chain, its `_` never reached; a take after `wrap` or a relabel takes untested. rr's
   `literal` is now the lean parser's: a `(`'s close asked for `~` (the subject in parentheses), else the term and
   then `if` a comparison operator `wrap`, the operator, the right term, `else if` `~` the pattern; `literal_tail`,
   `paren_inner`, `io_directive`, `code_part`, `r_statement` and `block_statement` are written in their callers.
   rr's calls form has 45 rules' functions, the lean parser's count, and two `list`s.
9. One test asked once, and one function for one code (`moves::joined`, `G::same_methods`). `if A || B || C { if A
   { x } else if B { y } else if C { z } else { refuse } }` is `if A { x } else if B { y } else if C { z }`: rr's
   `r_word` asks each kind once, as the lean parser's `match self.kind()`. Methods whose code is the same but for
   their names are one, the first, called for each: rr's inside of a brace pair (`raw_text_x_0`), read for a raw
   text and for a JSON object alike once each is one leaf, as the lean parser's one `braces`.
10. One `primary`, and one leaf over two tokens (rock 6). The grammar's `primary` is one choice, as the lean parser's
    `primary` and simdjson's one `switch` on a value's first byte (its `-` and digits one number): `numeric`,
    `unsigned` and `minus` are gone. `numeric` was a level only so that a float's mark (`float_at`) was under no
    field (`grammar.rs`: a mark makes no node); `word`, a function of its own in the lean parser, is that level now,
    and reads a float, a number, a name, a string or a code literal as the lean parser's `word`. A unary's operand is a
    `primary`: `-` and a number is one leaf, so none follows a unary's `-`. Then `moves::two_token_leaves`, in
    `G::leaves`: a token taken as a leaf and joined to the next (`merge_as`), or a decision each of whose arms joins
    it to a kind, is one leaf over the two, its kind chosen first (`let kind = …`), as the lean parser's
    `leaf_over(kind, i, i + 2)`: rr's `-1` and `.input`, Kotlin's `as?`, `>=`, `?.`, `return@` and a label's `@`.
    Neither tier 2 merges a token now.
11. A literal's term the default, a group unless a term follows (rock 4's printing). A choice's last alternative that
    begins with a call of a level is its default (`else { self.term()?; }`) also where it has marks, where none of
    them can hold at a token another arm's test can, nor its words at another arm's kind's (`G::apart`,
    `G::first_holds`). Then `moves::into_default`: an arm of an `if` whose code is its `else`'s `match` default goes
    into it, and the `match` arm whose word it may hold at asks it does not (`w::LPAREN if !past_lparen…`), as the
    lean parser's `group.is_ok() && !term_follows`. A token's choice of words is `matches!(self.word_at(j), A | B)`
    (`G::elem`), as the lean parser's `matches!(k - P0, EQ | NE | …)`. rr's `literal`: where the `(`'s close is
    followed by `~`, the subject in parentheses and the pattern; else a `match` of the word: `!` a negation, `(` a
    group unless a term follows its `)`, else the term and its comparison or pattern. `paren_subject`: `(`, a nested
    one or the term, `)`.
12. An optional part asked by the word after it (rock 8's printing). A part that must take a token, tested by words
    and kinds alone, of more than one test, before a word it cannot begin with, is read unless that word follows
    (`if self.word() != w::RBRACK { … }`), as the lean parser's `list` asks `is_p(close)` and simdjson
    `*peek() == ']'`. rr's JSON rules are simdjson's states: `json_document` a `{`, the members where a key follows,
    the `}`; `json_array` a `[`, the values unless the `]` follows, the `]`; `json_value` one `match` on the token, an
    array the default.
13. One function for one part of the language, written where the lean parser writes it (the user's list of
    2026-11-25: `include`, `io_parameters` and `io_inner`, `column_list`, `functor_type` and `suppress_line`, `-1`).
    - **One statement.** The grammar's statements are one choice, as the lean parser's one `statement`: a block or a
      directive at a `.`, an `#include`, else a clause. A block's statements are the same but that a block is
      refused, so the two share `plain_statement` (an `#include`, a directive, a clause): a mark refused takes no
      token, so the state machine (`grammar.rs`) cannot let a refusal go first over the same mark's way into a
      block. `#include` is read in one place. A `.end` outside a block is refused where the file's statements are
      read. A rule's call is a choice's default as a level's call is (step 11), a rule that must take a token
      refusing where none of its tests holds: `statement`'s last arm and `block_statement`'s are
      `else { self.plain_statement()?; }`, as the lean parser's `test` refuses a nested block and then calls
      `statement`. Where the default's word may also be another arm's kind (Kotlin's soft words in a template, which
      the lexer gives `interpolated_identifier`), or its test is under a flag, it has no default: the arms keep their
      order.
    - **The tokens in an I/O relation's parentheses: one leaf of one loop.** `parameters: leaf raw_text
      io_parameters`, `io_parameters` the tokens with pairs counted, `io_inner` a pair's inside: no `merge` written
      by hand (a `leaf`'s rule may hold `any except …`, `bitlex::syntax`). A leaf whose rule reads one or more
      tokens by a loop (`G::leaf_loops`) is that loop, the leaf refused where it read none: rr's `io_relation` is
      `leaf_open`, `io_inners`, `if self.i == self.leaf_from { refuse }`, `leaf_close`, as the lean parser's scan to
      the `)` and its `leaf_over`. A leaf whose rule refuses no mark checks none (`leaf_refuses`, the rules it
      reaches).
    - **One type, one braced column.** `functor_type` is gone: a functor's return type is a `type`, a level of its
      own, as the lean parser's one `ty`. `braced_column` (a name between braces, else one raw text) is read for a
      code column, a tag column and `suppress … over`, as the lean parser's one reader.
    - **A node's function of one caller written there** (`G::late_inline`, `node_method`): its node opened and closed
      in its caller, as the lean parser's `violation` opens `SuppressLine`. `column_list` stays a function: it has
      four callers, as the lean parser's `columns`.
    - **A test the code already asked is not asked again** (`moves::known_takes`): an `if` whose first test holds by
      what an enclosing test asked is its first arm's code; `-1`'s second token is a number its test asked
      (`pass_as(true)`), and its kind asks only the float's mark. A leaf of two tokens whose last decision's arm
      asks the pass's test is the `else` (`two_token_leaves`). `else { if … }` is `else if …`.
    - Stays: `literal` and `paren_subject` (steps 11, 12) read as they did.

## Rocks 4 and 8: the grammar keeps its shape

The user, 2026-11-25, after the pros and cons of each shape and what simdjson does: "do these".

- **A `(` at a literal is decided by reading ahead** (`paren_term`, `paren_tilde`, `paren_closes`), as 0673 chose,
  not by trying a group and going back as the lean parser does. The grammar is also `grammar.rs`'s state machine
  (the interpreters, Cranelift, Metal), which reads a file once and cannot go back; simdjson never goes back either.
  What is left is the generator's: how `literal` prints its tests and `paren_subject`.
- **A policy document is checked as JSON by the grammar's rules** (`json_document`, `json_member`, `json_value`,
  `json_array`), in the one pass, as simdjson's DOM parser checks while it reads. The lean parser's `serde_json` over
  the scanned braces reads the bytes twice, and one grammar cannot call another in the state machine. What is left is
  the generator's: those rules printed as simdjson's object, array and value states.
