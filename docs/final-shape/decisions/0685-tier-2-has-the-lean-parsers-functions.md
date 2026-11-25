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
   lean parser's. Not yet: code that differs in a field only is not one function with the field its parameter.)
