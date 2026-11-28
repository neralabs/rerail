# 0698: Python's Bitlex grammar, and a level's indent

Status: building (the user, 2026-11-25: "now, I want to switch to Python. Please get the .bitlex file for Python
both correct and performant. Once again, we proceed by addressing differences with the lean version.").
Follows 0163 (the lean Python parser), 0305 (Kotlin's grammar against its lean parser) and 0685 (tier 2 has the
lean parser's functions).

## Intro

Python has a lean parser (`rerail_python_lean`, 0163) that writes tree-sitter-python's tape directly, and a Bitlex
lexer that is a document only (`docs/parser-generators/lexers/python.bitlex`). No test compiles that document, and
it uses constructs Bitlex has since removed (back fills, 0416). Kotlin and `.rr` each have a grammar, lexer and tree
in one `.bitlex` file. It compiles to tier 1 and tier 2, and its tapes are checked node for node against the lean
parser's.

Python needs one thing those grammars did not: layout. A block is the lines indented past its header's, and it ends
at the first line indented less. tree-sitter's scanner keeps a stack of widths, and gives INDENT, NEWLINE and DEDENT
tokens from it. The lean lexer does the same. None of those tokens is a node on the tape. They only decide where
statements and blocks end.

## Decisions

1. **A crate of its own.** `rerail-python-parser` (`native/crates/python_parser`) holds `python.bitlex`, compiled as
   Kotlin's is (`rerail_kotlin_parser`). Its tests compare each tape with the lean parser's: kinds, fields, supertypes,
   the extra flag and spans. A file the grammar does not read is refused, to the lean parser. The corpus is a folder
   of `.py` files (`RERAIL_PYTHON_CORPUS`). Tier 2 as Rust gets its own crate once the grammar reads a corpus, as
   `rerail-kotlin-aot` holds Kotlin's.
2. **The lexer is written anew** in the constructs the compiler has: regions for strings and comments, the token
   loop's rows, keywords, pairs. The document keeps its place as the design study it is.
3. **Line breaks inside brackets are blanks, in tier 1.** A bracket depth over code (`count`) leaves a line break
   inside `()`, `[]` or `{}` no mark, as tree-sitter's scanner gives no NEWLINE there. So an expression that goes
   on over lines (`(a\n + b)`) is read with no mark in it.
4. **A statement ends at `newline`**, a mark at each line break of code outside brackets and not after a line
   continuation, and at the file's end. Blank and comment lines between two tokens are one mark.
5. **A level's indent.** Each level holds a width, its indent. A nested level starts with its caller's indent and
   gives it back when it ends, as a remembered word is (`remember`). A level named by `indented` (`indented block`,
   a statement of the tree, as `remember` is) sets its indent, at its first token, to that token's column: the word
   `INDENT`, after the `PUSH` of each call into it. The interpreter keeps it beside each return; the JIT keeps it in
   the return's high 16 bits (`Mem::Indent`), and refuses a column past them.
6. **Two questions compare a column with the indent.** `, deeper` holds where the column of the token the mark is
   before is past the asking level's indent. `, shallower` holds where it is less. Each level asks with its own
   indent. So a level that ends at a `shallower` mark (`ends`) leaves it to its caller, which asks again with its own
   indent:
   - `mark indent = …, deeper`: a block's start, after its header's `:`. The header takes it (`':' indent
     consequence: block`), so it is asked with the header's indent; the block then sets its own. A block that took
     it itself would ask it with the indent it had just set.
   - `mark dedent = …, shallower, ends`: the end of every block wider than the line, one level after another.
   Each mark's question is a table of the program (`markcolumn K C`, `Words::mark_columns`).
   A block's statements then go on while the next line's column is at least the block's indent, which is
   tree-sitter's rule. A line indented past the block but not after a header is a statement of the block, as
   tree-sitter's scanner gives NEWLINE there and no INDENT. A dedent to a width between two blocks' widths ends the
   inner one only, as the scanner pops widths while the line is less.
7. **A column** is the token's start less its line's start: the byte after the last line break before it. The bytes
   between the record before a token and the token are blanks and line breaks only, so tier 2 finds that line break
   in that gap, a word at a time, as `first_on_line` finds a terminator. A line whose indentation holds a tab or a
   form feed is refused for now, since those count as tree-sitter's scanner counts them (to the next multiple of
   8; back to 0).
8. **The Rust tier 2 refuses an `indented` grammar for now** (`bitlex_rust::tier2`). The JIT and the interpreter read
   it; Metal runs the JIT's tier 2 on its records, so it needs nothing of its own.
9. **Strings: one reader over four regions.** A region per quote kind and length (`s1`, `d1`, `s3`, `d3`). A raw
   string closes as any string does, at a quote no backslash escapes, so it is no region of its own. `in_raw`, a fill
   from a raw opener, says which strings' backslashes are text. One reader (`string_start`) reads them all, as
   Kotlin's reads its strings and raw strings: a triple's two more quotes are a part (`quote2`), and every escape is
   one part (`escape_sequence`, `last` its last byte). A backslash that starts no escape is a part and a leaf of its
   own (lean's `Backslash`). The closer is `string_end`. A prefix (one or two prefix letters at a word's start, then a
   quote) is a token of its own; the tree merges it, the opener and `quote2` into one `string_start` leaf (`merge`).
   Not read yet, so refused: f-strings, `\N{…}`, `\N`, `\u` and `\U` in bytes strings, carriage returns in strings,
   a prefix of three letters, a triple opened or closed by four quotes or more, and a raw string of backslashes
   only, whose end the lean lexer starts at its first backslash.
10. **The provers evaluate each value once per choice** (`bitlex_prove::Proof::memo`). A block program's ops are a
    graph: a mask that many others read (a region's fill, `escaped`) was evaluated once for every path to it, at every
    step of the search. Python's string masks made one question run for minutes. Each value at a lag is kept for the
    bytes and carried values chosen now, and dropped when a choice changes. Its first evaluation sets the search's
    open choice where it was unknown, so the answers and the search's steps are the ones it had.
11. **A node's supertypes are its slot's and its kind's** (lean's `class(kind, ctx)`). tree-sitter gives a node the
    supertypes from the slot it stands in down to its kind: in an expression's slot an `identifier` is an `expression`
    and a `primary_expression`, a `comparison_operator` an `expression` only; in a call's `function` slot an
    `identifier` is a `primary_expression` only. So a node holds one slot, the outermost supertype the grammar
    reached it through (its one `NODE_SUPER`), and its classes are a table of the program's (`superclass S K C…`,
    `grammar::super_classes`): for each supertype `S` and each kind `K` that may stand in its slot (its choice's
    symbols, through the choices in it, and the kinds a node's rule retags it to), the supertypes from `S` down to
    `K`. A kind the table has no row for in its slot is in no supertype.
    - **A wrap moves the slot.** The wrapper stands where the moved node stood: it takes the moved node's field and
      its slot, and its classes are its own kind's in that slot; a wrapper that is no member of the slot (a
      `keyword_argument` where an expression was, an `expression_list`) has none. The moved node takes field `F` and
      the slot `in S` names (`wrap K F [as M] [in S] REST`); none without `in`, as no field without `F`.
    - **In the JIT the slot is the top bits of the row's field** (`SLOT_SHIFT`), so it moves with the row and a wrap
      copies it with the field. A field written to a node made earlier (a relabel, a reread) keeps them. The slot is
      part of the node's code key, so the classes are found once per code, where `finish` writes the code table, not
      once per node. This replaces the list of (node, supertype) records the passes, shifts and drops kept in node
      order, and its room checks. A grammar of more supertypes than `MAX_CLASSES` is refused, as before.
    - The Rust tier 2 (`bitlex_rust`) still keeps the path's supertypes: no grammar it compiles wraps a node in a slot.
12. **Two rules or choices of one name are refused** (`grammar::build_shaped`). A choice named as the root node made the
    follow analysis's fixpoint run forever.
13. **Statements, imports and primaries.** The simple statements, imports (`from __future__` a statement of its own,
    by a token mark, `__future__` a soft keyword), and a primary's attributes, calls and subscripts, each a wrap of the
    primary so far. An assignment's target is read as an expression and moved into its `left`, slot `pattern`. A
    trailing comma is a token mark before the closer (`trail`), as Kotlin's: the lean parser keeps that comma and the
    closer. In a type, a name before `[` (a token mark, `generic`) starts a `generic_type`; anything else is an
    expression.
14. **`retag K inner G A`: the open wrap's first node takes a field.** On entering `A`, the innermost open node, a
    wrap, becomes a `K`, and the node it holds first (the one it moved) takes field `G` (`NODE_FIELD m=5`). A
    parameter `a: int` is a `typed_parameter` whose name has no field; `a: int = 1` a `typed_default_parameter` whose
    name is `name:`, and only the `=` past the type says which. The moved node is the open node's first row in the
    JIT (the head marks refuse a wrap of a wrap, as `inner` does), the open frame's base in the interpreter; the
    "moved" state can not serve, since the type's own wraps (`a.B`) move other nodes first. The Rust tier 2 refuses
    it. A wrap's heads in the supertype table include the kinds its body retags it to.
15. **An indented level's node keeps the comment lines as deep as its indent** (`CLOSE m=2`). tree-sitter's scanner
    gives no DEDENT before a comment line at least as deep as the block (`first_comment_indent_length`): the block
    holds it, and the dedent comes at the first line less deep, comment or code. The dedent mark's bit is at the
    first line break, so those comments are pending after it when the block closes: its node's close settles the
    leading ones with a column at least the level's indent, and leaves the rest to its caller, which asks again
    with its own. It is the indented level's own node only: a statement it closes on the way out (an inner `if`)
    takes none. `newline` and `dedent` settle (`, settles`), so a comment before a statement's line break is the
    statement's block's. Whether a dedent comes at all stays the next code line's column, as the scanner's.
16. **Operators by precedence, as Kotlin's.** Each level's operator is a wrap of the operand so far, whose right side
    is a `nested` choice of the tighter levels: `power` (`**`, right-associative), the unary operators, then `*`,
    `+`, `<<`, `&`, `^`, `|`, each a `binary_operator` in slot `primary_expression`. A comparison wraps a primary
    (`comparison_operator`, slot `expression`), its operators `operators:`; `not in` and `is not` are one node over
    their two leaves (`wrap 'not in' _ 'in'`: a wrap's kind may be quoted). `not`, `and`, `or` and the conditional
    expression are in slot `expression`. `expression` is `nested`, and so is the comparison's rest: a wrap stays at
    its caller's level, and the part after it is a level of its own, else the follow analysis finds a call whose
    uses differ in its hidden marks.
17. **A supertype's rows merge across its choices** (`grammar::super_classes`): each `(supertype, kind)` pair is one
    row, whatever choices name the supertype, as the table is sorted by it.
18. **A call-only state may close first.** Tier 2's verifier lets a state that only calls begin with `CLOSE`, as a
    level ending at a wrap's end then enters the next.
19. **A number is one `integer` token; a float is a test of its bytes** (`mark real = empty, at integer when
    count(n_dot | exp_at) at least 1`, as Kotlin's `real`). Its bytes: the word from its first digit, a `.` and the
    word after it, a `.` before a digit not after a word (`.5`), an exponent's sign and the word after it. A hex
    number takes its digits, an exponent its `e`, a last `j`; `_` between digits. Octal, binary, `L` and the rest
    are refused. A back fill (whether a later `.` makes the number a float) is not Bitlex's (decision 0416).
20. **A mark's waiting field and supertypes go past it** (`grammar::build`, `SPECIAL` states). A mark makes no
    node, so where a slot's first token is a mark (`real integer as float` in an expression's slot) the field and
    supertypes go to a state past the mark, its position's with them, and the next token's leaf takes them. Where
    the level may end past the mark, the grammar is refused.
21. **An extra's reader gives its mark where a mark tests a token** (`Reader::retag`). The token loop passes over an
    extra's tokens by its mark, which only a reader gives; a comment whose first byte (`#`) gives its kind had no
    retag. Where a mark's token test exists, the extra's reader now gives its own kind with the mark.
22. **An f-string is regions and their tails** (`python.bitlex`). Each of the eight quotes (`'`, `"`, triple, with an
    `f` or `t` prefix or not) is a region. An f-string's region closes at its quote or at an interpolation's `{`
    (`or i_open`: a `{` that is not part of a `{{`). Past the interpolation's code, a tail region (`ts1 = rbrace ..
    sq_c or i_open resumes fs1`) continues the string's token record (decision 0600). A format spec is one region,
    `spec = colon .. next(rbrace) after fs1 fd1 fs3 fd3`: it opens only where the last region closed at its
    template's `{`. The tape is the lean parser's: `string_start` (the prefix and quote), `string_content` with
    `escape_interpolation`, `interpolation` (`expression:`, `type_conversion:`, `format_specifier:`), and
    `string_end`. The braces and the spec's text are implied tokens of their nodes. What is not read yet is refused
    by an escape, never a wrong tape: a nested spec, a raw f-string, `=`, a `:` in brackets, braces or `#` in an
    interpolation's code, an empty interpolation or spec, a `}` alone, a run of three braces.
23. **One reader reads resumed and plain regions** (`bitlex_masks`). A first byte gives one reader, so `"` reads the
    plain string and the f-string's tails at once. The resumed regions share one template, and the plain ones have
    none. Where the two kinds are mixed, a resumed region opens only past its template (`prev(f_K)`).
24. **An `after` region's reader opens at its taken opener** (`o_K`). A spec's `:` is a code token's too, so the
    reader opens only where the walk opened the region, and the code's `:` is the other rule's. A plain region's
    reader keeps its opener's bytes, which the token loop tests without the walk.
25. **A start mask an arm reads stays made** (`BlockNative::readers_tags`). A reader's tag replaces its start masks
    on the CPU, but where an arm's opener, closer, last or template is such a mask, the token loop reads its row.
26. **The token loop's punctuation shortcut gives way to a reader's tag** (`native_cpu_block`). Where a byte is a
    reader's opener as well as punctuation (a spec's `:`), the tag is taken first.
27. **A node moved into a slot is one of the slot's kinds** (`NODE_SUPER m=1`, both tiers). A wrap moves the
    expression read so far into its slot (an assignment's `left` into `pattern`). Where the slot has no row for the
    node's kind (a list as a target, which tree-sitter reads as a `list_pattern`), the file is refused at that pc.
28. **A union over a generic is a `union_type`** (`choice types = type (wrap type _ wrap union_type _ '|' type)*`),
    as the lean parser's annotation reads it: an expression operand takes every `|` itself (`int | None` a
    `binary_operator`), so only after a generic does a `|` start a union. Each step is a `type` of its own around a
    `union_type`, left to right. An annotation, a return type and a generic's arguments are `types`.
29. **A wrap inside a wrap of the same node goes inside it** (`stream.rs`, `WRAP`). A wrap's moved node is the head
    of its base's chain where it wraps the node so far; one run inside an open wrap moves the node that wrap moved,
    so its wrapper stands just outside that node in the chain, and its size counts none of the wrappers outside it.
    The JIT's wraps nested so already.
30. **`try`, `with` and `for`.** A handler's `except` and its `:` are leaves, its block and `finally`'s have no
    field; `with` is a leaf, its items a `with_clause`. `as` binds an `as_pattern` in the expression's slot (its
    target a primary). A loop's target is a `pattern`, a name and its attributes and subscripts, or a
    `pattern_list` of them: not an expression, whose comparison takes `in`. `expressions` is `nested`, as the loop
    reads it before `:` and a statement before its line break. Not read yet, refused: `async`, a parenthesized
    `with` (a token mark no state takes), a tuple or list as a target.
31. **A comprehension is its bracket's node retagged** (`relabel body retag list_comprehension comprehension ']'`).
    Past the first item, a `for` makes the item `body:` (the node made last: an expression, or a dictionary's
    `pair`) and the open node a `list_comprehension`, `set_comprehension`, `dictionary_comprehension` or
    `generator_expression`; a call's only argument's generator stands for its `argument_list`. A `for_in_clause`
    keeps `for` and `async` as leaves; its right side, and an `if_clause`'s, is an expression with no conditional,
    as the next clause's `if` is no `else`'s. The kinds a relabel's rest retags the node to are in the supertype
    table (`grammar::super_classes`), as a node's own rule's are.
32. **`await`, `...` and `async`.** `await` is a primary over a primary (slot `primary_expression`). `...` is a token
    of its own (`ellipsis`) from the first dot of a run, as the lean lexer reads it; two dots are two operators.
    `async` is a leaf before `def`, `for` or `with`, the node retagged by the word after it.
33. **A list whose one item is a splat is refused.** tree-sitter reads `[*b]` with the splat a primary (`[*b.c()]`:
    the call's object), not a list item.
34. **Targets, slices, lambdas, yields and concatenations.**
    - A statement's first expression is moved into a `pattern_list` at its comma, the rest patterns (a
      `list_splat_pattern` among them), and only an assignment follows. A bare `a, b` statement (its expressions
      the statement's own) and a tuple target are refused. A target list's trailing comma is a leaf, as a list's
      (`trail` before `=`).
    - A slice is a node whose `:` are leaves, the expression before it moved in.
    - A `lambda` is in slot `expression` (not a primary), its parameters `lambda_parameters`.
    - `yield` is a node, not an expression, its keyword and `from` leaves; a statement or an assignment's right
      side.
    - Strings side by side are a `concatenated_string` of them, the first moved in.
35. **A parenthesized `with` clause by a token mark** (`w_paren`, after `with` at its `(`): a run to a comma at the
    parentheses' depth with more after it. Then `(` is a leaf, `)` implied, and a trailing comma's `,` and `)`
    leaves, as the lean parser's. The lean parser asks for a `:` after the `)` too; a pattern holds one run, so
    where none follows (`with (a, b) as c:`) the clause's reading refuses at the `as`.
36. **A region's first part takes its record's marks** (`stream.rs`, the parts a reader splits a record into). The
    parts' own pass measures the first part's gap from the file's start, so a string after a block's indent took
    that line break's newline, indent and dedent, and its level ended before a concatenation. The JIT's parts had
    the record's.
37. **A subscript's trailing comma** keeps `,` and `]` as leaves (`trail_bracket`), as a list's.
38. **A header's body is a `suite`**: an indented block, or the line's simple statements retagged `block`
    (`def f(): ...`, `if a: b`), as the lean parser's one-line bodies.
39. **`a := b` is a wrap in the expression's slot** (`walrus`): a `named_expression`, its name with field `name`
    and its value with `value`.
40. **Tuple and list patterns** (`tuple_pattern`, `list_pattern`) are patterns, so a loop's target takes them, and
    a target list's items after the first. A statement's first `(` or `[` is one where a token mark
    (`target_paren`) finds an `=` after its closer at its depth; else it is an expression. `(a) = b` is a
    `tuple_pattern` too, as the lean parser's.
41. **A parenthesized tuple takes splats** (`paren_item`), first or after a comma.
42. **Type parameters** (`[T]`) after a class's or a function's name are a `type_parameter`, as a generic's.
43. **A splat that begins an unbracketed expression list, or is a list's only item, is a primary** (`star_item`):
    `list_splat` around a bare name, its postfixes after it (`[*a.b()]` an attribute of the splat), as the lean
    parser reads `x = *a, b`, `return *a, b` and `[*a]`. A list's first splat stays a `list_splat` among its items
    where a token mark (`l_many`) finds a comma after it at its depth.
44. **`*a, b = c`**: a statement's first `*` is a `list_splat_pattern` in a `pattern_list`. `()` is an empty
    `tuple_pattern`; `except*` has its `*` a leaf; a lambda's lone `*` is a `keyword_separator`.
45. **`type X = …`** is a `type_alias_statement` where `type` comes before a name (`t_alias`), its word and `=`
    no leaves, each side a `type`.
46. **`match`** is a statement where its word comes before a `:` on its line at its depth (`m_stmt`, a run of
    tokens `without newline`); else it is a name. Its body is a `block` of `alternative: case_clause`s, and the
    patterns are the lean parser's: each pattern in a `case_pattern` (a union's alternatives not), `_` alone an
    empty one, names `dotted_name`s, `p as x` a `case_pattern` around an `as_pattern`.
47. **A node may start at the end of the record before its first token** (`node K attached`, a comment's
    included), as the lean parser's match body starts at its `:`'s end. `Act::Open`'s `attached`, `OPEN m=1`:
    the interpreter's `last_end`, the JIT's `Mem::LastEnd`. The Rust writer refuses an `attached` node, not
    lowered there yet.
48. **A run's `~(…)` takes `with M` and `without M`** (syntax, evaluator and tier 2): any other token, with or
    without the mark. Decision 0673 step 4 had them on a run of kinds and words only.
49. **A match's subjects and a keyword pattern's union**: `match a, b:` has a `subject` field on each, and a
    keyword pattern's value may be a union, its `as` wrapping the keyword pattern. `match` before its `:`
    (`match: int = 1`) is an annotation's name (`unless before "match" ":"`).
50. **Octal and binary numbers** (`0o7`, `0b1`) are an `integer`, their letter after their `0`; a digit past
    the base, or no digit, is an escape.
51. **A quadruple quote is a triple's own**: the escape for a triple closed by four quotes asks only a
    triple-quoted string of that quote (`"a''''b"` is text).
52. **Braces in an f-string's text pair from the first of each run** (`escaped()`, as backslashes do): `{{{a}}}`
    is an `escape_interpolation`, an interpolation and another; an odd run's last `{` opens the interpolation.
    The tail's `}` opener is no text's, so a run after it pairs from the next.
53. **A raw f-string's tails are raw too**: from its opener to the next f-string's that is not raw.
54. **An interpolation's braces are brackets**: the depth counts its `{` and its tail's `}`, so a line break in
    it (a triple-quoted f-string's) is a blank. A comment or a brace in it is still an escape.
55. **A class pattern's trailing comma** keeps `,` and `)` as leaves (`trail_paren`).
56. What is left of Python's escapes is the regions' walk's: an interpolation's code holds no bracket count (a
    `:` in its brackets, a brace in it) and no second f-string (a nested one ends the outer one's template).
57. **The regions' walk counts a bracket level in a template's code** (`regions H level O to C max N else D`,
    `block::Level`). The user asked for the level to be tracked. From a template's opening to its string's
    end, each `O` outside the regions adds one and each `C` takes one away (none at zero). A kind with
    `after` opens only at level zero: `{a[1:2]}` has no format spec, and `{ {k: v} }`'s first `}` is the
    dict's. An `O` past `N` (1 to 15) is `D`, an escape in Python's grammar (`i_deep`, `N` 15). The escapes
    for a `:` in an interpolation's brackets and a brace in an interpolation are gone.
58. **A walk with a level has no rounds form.** No fixed point of masks counts brackets. The rounds' body
    counts no level and makes `D` empty, so it is the walk only where no template's code has a bracket.
    The evaluator walks such regions (`Eval::walk_regions`), as the interpreter's walk and the JIT's do.
    The JIT keeps the level in four bits above the last template's kind in the walk's state. The lowering
    to Rust refuses a level, as it refuses an `attached` node.
59. **Templates nest, at most four deep** (`regions … level … nest M depth A B`, `Level::nest`). The user asked
    for a bound of 1, 2 or 4, to keep the state small; it is 4. A plain kind's region in a template's code
    that closes at its `or` opens a template inside it: `f"{', '.join(f'{a}' for a in b)}"`. The outer
    template's kind and level wait, and go on when the inner string ends. One past `M` is `D` at its closer,
    the same escape as a level past `max`.
    - The JIT keeps a frame of eight bits a template, the open one's lowest; a push shifts it up.
    - `A` and `B` are each region's depth at its opener, two bits: the templates around it, or for a tail,
      around its string.
    - The token loop keeps the record a template left open by that depth (`Reader::nest`). The JIT's
      `open_slot` holds a word a depth, chosen by the depth bits in the read's flags (`DEPTH_SHIFT`). A
      token read again finds its own depth's word, as one slot did before. Tier 1's interpreter already
      kept a stack, now bounded by `M`. Tier 2 counts open strings, so it nests as it is.
60. **A level's rounds count its frames, so Metal's form is the walk's** (`regions()`). Before, the rounds counted no
    level: its deep and depth values were empty, and its tails opened after any template. On the posthog corpus that
    form differed from the walk in 181 of 1642 files. Now the rounds hold two counts, as bit-planes:
    - `nest`: the templates open (opened at a plain kind's second closer, closed at a tail's own closer);
    - `lv`: every frame in templates' code, each template's and each bracket's. A bracket counts only in a template's
      code (`nest` above 0), and not at a byte a kind opens at (a tail's `}`).

    Each template kind has a carry: "the innermost frame before the byte is a template of this kind". It is computed
    per level `L` of `lv`, as "the count before is `L`, and the last frame opened at `L` was this kind's template",
    with a `prev` on every way. A tail opens only where a kind it names holds; this replaces the "string ended" carry
    that a walk without a level keeps. The depth bits are `nest`'s planes before the opener (one fewer for a tail).
    `deep` is `lv` past `max`, `nest` past its bound, or a close with a template's frame on top.

    The rounds form fixed two rules, and the three walks (the interpreter's, the evaluator's, the JIT's) now keep
    them as well:
    - `max` bounds all of the open templates' frames together, not each template's brackets;
    - a close with no bracket of the code's to close is deep (the walk had ignored it).

    The Python tests and the corpus check every output as rounds against the walk.
61. **Python's `max` is 7.** The regions group is 168 operations without the counts, and with them 657 at `max 15`,
    418 at 7 and 379 at 6. The counts barely change the rounds: on the corpus, 2.49 rounds a block before, 2.53
    after; 6.10 and 6.33 where 32 blocks round together, as Metal's SIMD group does. At 7 the corpus reads every file
    it read at 15.
62. **Python as Rust: `rerail-python-aot`**, as Kotlin's crate: `tier1.rs` from `bitlex_lex_rust`, `tier2.rs` from
    `bitlex_rust`, checked by python_parser's `rust_is_current` (`RERAIL_BLESS=1` writes them). Every Python test
    compares it with the interpreter and the lean parser: tier 1's records (`tier1_differs`), then the tape, and a
    refusal where the interpreter refuses. What the lowerings lacked, and how each is lowered now:
    - **A level** (item 58 refused it). The token loop keeps `level`, `frames` and `depth` as the walk does, and the
      walk's `frame(…)` masks are one pass over the count's bytes. A template opened inside another pushes the outer
      one's kind, level and held record first, then becomes the template: the other order pushed the new kind, and
      the outer template never came back after an f-string in a dict's braces.
    - **The walk's `deep`** is the escape, not one of the hidden set's parts: a string or comment is not refused for it.
      A walk a refusal reads is never gated to test builds.
    - **Supertypes**, as the interpreter's `NODE_SUPER`. A node's slot is in its key's bits from `CLASS_SHIFT` while tier 2
      parses: a call to a `supertype` choice sets the slot of the node made last (`slot_made`, the outermost call
      last); a wrap takes the moved node's slot, and the moved node takes the wrap's `in S` (`slot_moved`). A kind its
      slot does not hold refuses the file there (`f() = 1`, `(a, b), c = d`). At the end `Writer::map_code_keys` maps
      each code's key to its classes by slot and kind (`classes`, from `super_classes`). The tests read the format's
      key, not the lean parser's layout.
    - **`indented` levels and `, deeper`/`, shallower` marks**: a level's function saves its caller's indent and takes
      its first token's column; a mark's column question is asked at each ask, past the token's cache; the node
      closes with the comments pending at least as deep as its indent (`close_indented`).
    - **A closer's record kind** where the records keep it at one reader only (`interp_open`, in both of a string's
      readers): that piece's kind in the other reader, not its byte's (`{` is punct).
    - **`at starts` assumes must hold in the token loop**, which test builds check at every start. Python's two were
      false: a name's start excludes a string prefix's first letter, and an operator's excludes an ellipsis's and a
      conversion's second byte. The token loop asks the rows whose start reads a region's `open` first, so a tail's
      `}` is a tail and not punct.
63. **The AOT's checks run in the tests, and the corpus reads it.** The parsers' test builds depend on their AOT crates
    with `test-hooks` (Python, Kotlin, JSON; rr's already did), so the generated checks (`code_l` against the walk's
    `code`, the `at starts` assumes) run in every test. They had not, and the corpus found two faults they hold:
    - a read that resumes a string clears its bytes from the loop's code, as a record does;
    - after an element, a repetition goes on as a `*`, also for a `+` (`sites`, `code`). Read as the `+` itself, the
      rest could not end, so a block's last statement had none of the marks that end the block in its follow. The
      optional `else` of a `while` inside it was then asked without `dedent`, and took an outer `if`'s `else` after a
      dedent of two levels.

    The corpus test checks the AOT where the interpreter matches the lean parser: tier 1's records, then the tape. It
    matches on all 1642 posthog files. The comparison with the lean parser is python_lean's example `compare_aot`, which
    builds the AOT without its checks.
64. **Tier 1 makes the masks of its regions in passes over the words, as stage 1 makes the rest.** Each mask made
    of the regions had been a pass of its own over the file, into a vector of its own: 142 of them in Python's tier 1,
    84 in Kotlin's. The lean lexer reads each byte once. The masks now go into the pass written last (`Gen::fuse`):
    each word of a mask is a local of the pass's step (`w_NAME`), a `prev` reads the word before it (`p_NAME`), and
    `fill`, `escaped` and `toggle` keep their carries (`c_NAME`) from step to step, as stage 1's nodes do. A vector is
    kept only for a mask that code outside the pass reads. A mask starts a new pass where it would read words not yet
    made: a `next` of a mask in the same pass, or a mask that the code after the pass's start writes or makes, such as
    the walk's masks, the token loop's `code_l` or another pass's masks. Python's tier 1 now has 11 passes and
    Kotlin's 4. JSON's and rr's had no masks of regions. A pass is one statement of the walk (`gate_walk`), and makes
    each of its masks.
65. **Tier 2 asks the marks that end a part only at a token whose word an arm has.** Lean has `NEWLINE`, `INDENT`
    and `DEDENT` as tokens, so `while self.peek() == T::And` asks one token. Tier 2's marks are questions about the
    token (`m_newline`, `m_dedent`, its column), and its loops and options asked them first, at every token:
    `while !(newline || dedent) && word == AND`. Now:
    - a loop's or an option's word or kind test comes first, and its end's marks are asked after it (`while word == AND
      && !(…)`). A `match` of the word asks them in each arm's guard, its `_` the loop's end
      (`moves::exits_after_words`);
    - a loop that ends an `indented` level (`block = statement+`, `match_block`) ends at the level's ending marks
      alone, as lean's `while !matches!(peek, Dedent | Eof) { statement }`. It no longer asks a statement's whole first
      set at every statement. A token that is neither refuses in the statement, as in lean. An expression level is no
      such loop: its callers' tokens (`)`) end it too.
66. **A string's parts are found in one scan of their starts, to the string's closer.** Tier 2's split of a string
    asked each part's mask apart with `first_bit`, which reads to the mask's next bit wherever it is, the file's end
    where there is none. That was once a part, for each of a string's parts, for every string, so it was quadratic in
    a file's strings. `first_of` reads the union of the parts' masks, a word at a time, up to the closer and no
    further, as the lean lexer's string reader looks no further than its quote. Each part then asks its own mask at
    the one byte found. `advance`'s look for a template's `}` that resumes a string reads up to the next record in the
    same way.
67. **The token's code is kept as the token is reached (`cur`), for every grammar.** `word()` and `kind()` had asked
    whether the token was past the last record and whether it was a piece of a split record, then read either one,
    at every ask. Now they read `cur`, one field, as the lean parser's `peek` reads its token. `arrive` sets it after a
    record's split, `take` sets it at a piece after another, and `advance` sets it after resuming a string. Past the
    last record it is `END_CODE` (word 0 and kind 255, which no token has). Grammars with no string read in parts had
    this already; it is now the runtime's own.
68. **The masks of the loop's code are a lane of the token loop, not passes after it.** The escapes made of `code`
    alone (Python's six of its numbers) read the loop's `code_l`, and were 7 passes over the file after the loop. The
    loop goes over the blocks in order, and a region token clears its bytes from `code_l` before the loop moves past
    them. So every block before the loop's is final. The lane makes those masks a word at a time behind the loop
    (`Gen::lane_passes`, `render_passes`):
    - at each block the loop comes to, the lane makes each word whose next block the loop has passed (`lane + 2 <=
      b`);
    - each of its passes makes the word one behind the pass before, whose `next` it reads;
    - after the loop, it makes the words left;
    - an escape is tested as its word is made (`refused |= w_X != 0`), so no vector is kept for it.

    This is the first step to one pass over the file. The walk's masks, which the string escapes read, and stage 1
    still run before the loop.
69. **The token loop writes the regions' masks as it reads each region.** These are the masks the walk makes: each
    region's opener `o_K` and fill `f_K`, and `hidden`. They are written with the walk's own writes, in `lw_…`
    vectors (`Gen::walk_writes`, `with_walk_writes`). A grammar gets them when every region is read by a row (`read`,
    or `read … parts`), none has an `else` form, and it is not lexed in the one pass (0619).
    - `hidden` ends at the region's own last, not at the token's. A template's `${` is the token's closer, two bytes,
      but the region closes at its `$`.
    - A test build asserts each `lw_…` equals the walk's mask on every file not refused.

    This is the second step to one pass. Next, the regional escapes and tier 2's regional masks are made from these
    in the lane, and the walk runs only in test builds.
70. **A compile asks every prover question through the kept answers, and writes the Rust tiers only for a caller
    that reads them.** A test's compile of Python took 18 s on the M1 Max, and Kotlin's Rust-writing compile 25 s,
    with every answer already kept. Three causes:
    - `last_row` and `token_ops_back` asked the provers directly. They skipped the answers (0414) and rebuilt the
      prover's operations for each row of each check: 11 s of Python's compile. They now ask through
      `Lowering::never` and `settled_never`, as `token_row` does. Kotlin's shipped answers hold the new questions.
    - The Rust writers ran in every compile. Only `rust_is_current` reads their text. `Symbols::rust` asks for it:
      Python's and Kotlin's `tiers_rust` set it, and the parsers' `compiled`, JSON's and the C directives' do not.
    - Tier 1's writer expanded a named mask again at each reader in `in_region_of`, so a mask DAG became a tree:
      22 s of Kotlin's. `Gen::region_memo` keeps each named mask's answer by region, part and context. `region_gen`
      goes up at each change of `regional` or `fills`, so the memo never answers from an older state.

    A test's compile is now 3.7 s: bitlex 1.5 s, Cranelift 2.1 s. nextest runs each test in a process of its own,
    which compiles the grammar again, so Python's suite takes 26 s and Kotlin's 51 s.
71. **A test binary's processes compile the grammar once.** nextest runs each test in a process of its own. The
    Bitlex compiler's half of each compile (`bitlex_output`) is now kept in the answers directory
    (`answers::kept_output`), and the rest of each process reads it. The key is the grammar text and the test
    binary's path, size and time, since the binary's code is the compiler, so a rebuild misses. The first process to
    miss compiles under the family's lock, and the others wait, then read. Only `compiled()` reads the kept output:
    the tests of the shipped answers and of a cold compile call `compile_timed` and still compile.

    A test's own work is now Cranelift's code for both tiers, 2.1 s of its 2.3 s.
72. **A number is a run over a class, and a check at the token.** This is the lean lexer's `number()`: a number's
    bytes are run, then its grammar is read once. About 15 derived masks and six escapes are gone.
    - `token based = start based_at run word` and `token integer = start int_start | dot_start run num_run`. The
      starts are `assume … at starts`, so the loop reads three byte terms.
    - `check based` and `check integer` hold the numbers' grammar: hex, octal and binary digits after `0x`, `0o` and
      `0b`; a decimal's `.`, its exponent and its `j`/`l` suffix; `_` between digits.
    - The float mark is `count(dot & number | exp_l & number & ~based_n)` at `integer`. A `.` operator is no
      number's byte, so the mark never holds at one.
    - `based` and `integer` share a kind, as no tree rule tells them apart. The tree reads a based number as
      `based as integer`.
    - Tier 1 as Rust is about 420 lines shorter. Its row arm calls `check_K` at the token
      (`bitlex_check_rust::Checks`), the same writer tier 2 as one loop uses.
73. **A check may loop through states that its bytes tell apart.** `check_parts` splits the check's machine into
    its strongly connected parts. A cycle whose states each have their own entering bytes is lowered as one fill
    over the cycle's bytes. Each state's mask is that fill, masked by its own bytes. A cycle in which one byte
    enters two states is still refused.
74. **A kind that two rows share is named by both.** Tier 2's symbols list a moved row's name as a second name of
    its kind, so a tree rule may name either. A moved row with a check keeps its moved byte in the check's set, when
    the check takes any of its real first bytes.
75. **A pass carries a mask's last word only where one of its live masks reads it.** The dead carry warned in
    Python's tier 1, and Kotlin's had two.
76. **A token's tests wait for the blocks they read.** The token loop runs a part at a time between blocks, for the
    tokens that start in the blocks already final. A delayed output's word is final only after the next block is
    made. A test whose output is read from the rows, not from the bytes, read the token's last block before it was
    final. A float such as `1024.0` across a block's edge lost its `.` there. `tested` now takes the part's
    `Limit`: a token whose last block is not final waits for it, as a run's search past the final blocks does.
77. **A token starts in code, and a quote or a `#` at a start opens its region.** These are rr's assumes:
    - `code = ~empty`, `o_str = quote` and `o_cmt = hash` at starts;
    - an ellipsis's start is three dots, the first after none, without `code`.

    Tier 1 as Rust no longer works out, at each token, which region it opens: a row's start never asks again
    whether a region opened there. A conversion's start (`conv_at`), a tail's `}` and a spec's `:` depend on the
    interpolation's level. Tier 1 as Rust refuses an assume that reads a region's mask, so those wait for the
    f-string reader.
78. **A name runs over its word, and a byte no token begins with is a `bad` token.** As rr's:
    - `token identifier = start word_start run word`. `name_more` and its assume are gone.
    - `class junk`: the control bytes, `$`, `?`, the backtick, `\` and 0x7f. `token bad = start bad_first pair empty`,
      with `bad_first = junk` assumed at starts. The tree has `untaken bad`, so no rule takes one and the file is
      refused, as the lean lexer fails at a byte its operators do not take. The `error … by tokens` is gone.
    - A byte past ASCII is no `junk`. Its escape refuses the file in code, and its bytes stay free for the kinds a
      reader moves (0509). With them in `junk`, `spec_text` had no byte left.

79. **An ellipsis is a run of dots, and its check is three.** `token ellipsis = start ell_at run dot`, with
    `check ellipsis = '.' '.' '.'`, in place of the run over `ell`. A pairs triple, as the lean lexer's table has it,
    cannot hold `...`: a triple's first two bytes must be a pair, and Python's `..` is two tokens. Four dots or more
    are refused to the lean parser, which reads `....` as an ellipsis and a `.`. None is in the corpus.
80. **The loop skips a class.** `skip ws`, as rr's, Kotlin's and JSON's, in place of `skip code & (blank | lf)`.
    Between tokens the loop is always in code: a region's bytes are its token's.
81. **A string's escape is a part read by its check.** `check escape_sequence = '\\' (u_l hex hex hex hex | uu_l
    hex hex hex hex hex hex hex hex | x_l hex hex | digit [digit [digit]] | esc_one)` is the lean lexer's `escape`,
    one byte at a time. A reader's part `escape_sequence = esc check else '\\'` reads it at a backslash: where the
    check matches, an escape through the match's last byte; where it does not, a lone `\`. The masks e2 to e_last
    are gone.
    - Bitlex (`Parts::checked`) makes two masks of the check: `P_ok`, where the check matches from, and `P_last`,
      the last byte of each match. The masks path reads the two parts, `E & P_ok` through `P_last` and `E & ~P_ok`
      one byte.
    - A check read this way has no loop. A choice's arms start with distinct bytes, and a `[…]` is not first, so
      the lexer reads each choice at the next byte without going back. Its `check` statement is the part's, not a
      token's.
    - The writers that read a part as the lexer does get the kind, the grammar and the other part from
      `Parts::checked`.
82. **Tier 2 as Rust reads a checked part as the lean lexer does.** A reader's checked part is one part at `E`
    (`Reader::checks`). At it, `check_P(&src[e..])` gives the escape through its match's end, and none gives `K`'s
    one byte, as the lean lexer's `backslash` calls `escape`. The check's function is `bitlex_check_rust`'s, the one
    tier 1 uses for a token's check. Tier 1 gives tier 2 the `E` mask, not `E & P_ok`, `E & ~P_ok` and `P_last`, so
    it no longer makes the check's masks.
83. **A string is a reader in the regions, `read string { … }`.** It says what the lean lexer's `string` reads, and
    Python's grammar no longer writes `or`, `resumes`, `after`, `level` or `nest`:
    - `prefix pfx max 2 after word`: one or two letters starting a word, then a quote;
    - `flags raw = raw_l, bytes = bytes_l, format = f_l, template = t_l`: a quote after a prefix with such a letter
      (`raw_q`, `bytes_q`, `format_q`, `template_q`);
    - `quotes sq | dq triple` and `escape bslash`: one quote or three, and a closer no backslash escapes;
    - `interpolation format lbrace .. rbrace spec colon level …`: where `format` holds, the code from a lone `{`
      to its `}`, a spec from a `:` at its level zero, and the level and nest of its code.

    Bitlex (`string_reader`) lowers it to the kinds the walk reads: each quote's (`s1`, `d3`), the format ones
    (`fs1`, closed at the `{` too), their tails (`ts1`, which resume them), `spec`, and their unions (`o_str`,
    `f_str`, `o_f`, `f_f`, `o_t`, `f_t`). The masks path runs the same walk. `Regions::readers` keeps the reader
    for the writers that read strings as the lean lexer does. Kotlin's templates keep `or` and `resumes`.
84. **A string's token starts at its prefix.** `token string_start = start str_at prefix pfx max 2 read …` reads
    the prefix letters as the string's own first bytes, as the lean lexer's `string_prefix` is one leaf with its
    quote. `str_prefix` and the tree's `str_prefix merge string_start` are gone. `word_start` still excludes the
    prefix letters (`~p1 & ~p2`).
    - Bitlex (`Parts::prefix`, `Parts::reach`): each region's row starts where its opener is reached,
      `o | next(o)&P | next(o, 2)&P&next(P)`, and its last is the opener. The content after it is `f_K & ~o_K`.
      Without a prefix the rows stay as they were (`start & o_K`).
    - The masks path: the reader's opener is that reach, and its closers exclude the openers, because the search
      for a closer starts at the byte after the token's start, before the quote. `Reader::open_last` is the
      openers' output. The first piece runs from the token's start to its first bit, and the parts start after it.
      Every split reads it: `records::read_parts`, the JIT's `split_enter` and `read_in_parts`, tier 2 as Rust's
      `split_R`. The JIT's depth bits of a template that nests (`Reader::nest`) are read at the opener, not at
      the prefix, which lies in the outer template's code. It is renumbered to an output with the readers' other
      masks and printed as `recopen K O`.
    - Tier 1 as Rust reads the region from its opener (`first_bit(&o_K, s)`) and the record from the token's start.
85. **The string reader writes a string's pieces, tokens and refusals.** Its clauses now say all that the lean
    lexer's `string` decides, so a writer that reads strings as the lean lexer does reads them from one statement:
    - `start K`: a triple's two more quotes, a piece. `text K`: the text. `close K`: the closing quotes.
    - `escape K = E check else L unless F`: an escape read by `check K`, else an `L`; none in a raw string.
    - `braces K`: `{{` and `}}`.
    - `interpolation F K = O .. J = C level …`, `conversion K = B L`, `spec K = S`.
    - `refuse "W" at A [if F | unless F] [X]`: what is not read, where `A` is: the opener, the fill, the text, an
      escape, a raw string of backslashes only, a brace alone, an interpolation's code, its end, an empty one, its
      spec, the spec's start, past the level's bounds. `X` is a mask of the bytes alone. A name `Q…` in it is each
      quote's.

    Bitlex (`string_reader`) writes what the grammar wrote, as text it reads after the regions: the pieces' masks
    (`in_raw`, `t_open`, `s_body`, `body`, `esc`, `fb`, `i_at`, `icode`, `esc_ib`, `conv_at`) and an escape of each
    refusal. `token K = start E read R` writes the four tokens (the string's in parts, the tail's, the spec's, the
    conversion's) in its place, in the grammar's order of rows. Tier 2 as Rust is the same as before, word for word.
    Two masks are now read as their bytes say, with the same answers:
    - `conv_at`'s `code & !` is `~hidden & !`;
    - an interpolation's `=` is `eq & ~prev(eq | bang | lt_gt)`, not inside `icode`, at the byte before its end.
86. **A line break is a statement's end where the loop's bracket depth is 0.** The marks read `lf & code &
    depth.eq(0)`, and `in_br` and its frame pass over the file go. The masks path reads `C.eq(k)` from the count's
    planes. Tier 1 as Rust reads it as the lean lexer knows its depth at a line break:
    - A gap mark `X & C.eq(k)` is `g_C == k && …X over the gap…`. `g_C` is the count as the token before left it:
      no bit of the count is in a gap, and the token's own brackets count after its gap.
    - The count goes on at a string token, where its `up` or `down` reads a region's opener, fill or hidden bytes where
      they hold (Python's `o_t`, `prev(f_f | f_t)`; not rr's `~region`). It goes down one where `down`
      holds at the token's start (a tail's `}`). It goes up one where `up` holds at its last byte, as each closer
      leaves it (an interpolation's `{`). Neither may hold in its body or at its closer's other bytes; a closer that
      holds at none of this region's closers is not one of its (as `closer_last` reads them).
    - Its closer is kept (`close`) only where the count reads it.

    A test has a line break straight before a closer, in code and in an interpolation; it fails where the gap reads
    the count after the token.
87. **Tier 1 as Rust reads a string reader's strings in its one pass, and writes their pieces as records.** A grammar
    with `read R { … }` is lexed in one pass of the token loop (`bitlex_lex_rust/reader.rs`), as JSON's and rr's
    are, and as the lean lexer's `string` reads a string, from stop to stop of stage 1's masks:
    - The string's arm reads its prefix's flags at its quote and its triple, then its text to its closer or an
      interpolation's `{`. Its stops are the quotes, the escape, and in a format string the braces.
    - It writes each piece: the opener (its record, from the prefix to the quote), a triple's two more quotes, the
      text, each escape by its check (else `\`), `{{` and `}}`, the closer or the interpolation's `{`. The first is
      the loop's record, with its marks; the others are written after it, of their kind alone. A pause (stage 1 has
      not made the blocks) writes nothing, and the token is read again.
    - The reader's state is in the loop: the template open (its quote, its flags, its code's bracket level), the
      templates around it (`nest`), every template's frames (`max`), and where the interpolation's code began.
    - At a template's level 0, its `}`, its spec's `:` and its conversion's `!x` are the reader's rows, picked
      before the rows' pick. The tail's `}` record has no word and no marks, and the gap is the next token's, as the
      interpreter resumes it. A token of code in a template counts its brackets, as the walk does.
    - Each refusal is tested in the arm where its place is: the opener, the fill and the text over each segment,
      an escape, a raw string's backslashes, a `}` alone, the code since the `{` (to the file's end too), an end,
      an empty one, the spec and its start, the bounds. A refusal by a mask first lets stage 1 make the blocks after;
      a count's is the file's.
    - The count of brackets (`depth`) goes up at the interpolation's `{` and down at its `}` in the arms.

    A piece is no token of code, as the lean lexer's (`IOpen`, `Text`) are not: a lookahead counts no
    interpolation's `{` as a bracket, and no `,` of a string's text. The interpreter's parts carry the words of
    their bytes, which its tier 2 takes and no lookahead of its reads, because it reads the records.

    Tier 2 as Rust reads the pieces as records: it no longer splits a string (`split_0`, `resume_1` and their masks
    go). The interpreter's token stream is `stream::tokens`, which is `run_seen`'s split of the records, the parts'
    codes the records' pass's. The tests compare tier 1's records with it, a piece after the first by its kind.
    Tier 1 reads the source padded, as rr's: `parse(buf, n)`, `lex(buf, n)`, `padded`.

    Three things the writer reads generically:
    - A row's token test reads its kind's `in KIND` assumptions, as its run does. The integer's float test is
      `dot | exp_l`, not the walk's `number`.
    - A row's start reads a string reader's regions as none: its arms read them.
    - The regions are numbered for the counts' reads, but the loop opens none.

88. **Python's tier 1 as Rust has no stage 1: each block's masks are made where the loop reads them.** As rr's and
    JSON's, the loop makes each block's words from one load and its classes (`block_classes`), and a read past the
    block reads windows of the bytes. No mask of the whole file is made before the loop, and the reader's arms read
    their stops, quotes and refusals the same way. What the bytes form needed:
    - **The escapes no token reads** (`sinks`: a tab or a form feed in a line's indentation, or starting the file)
      are tested in the loop, over each gap and its token (`at` to the token's last byte), and over the gap after the
      last token once after the loop. Every byte is read once. Stage 1's check of them after its blocks goes.
    - **A search of `A & F`, `F` a `fill`** reads `A`'s bits (its block's word, then windows), and tests `F` at each
      bit by walking back over the bytes `E` does not hold to one `S` holds, as the lean lexer reads a line's indent
      at its line break. `fill(S, E)` at a byte holds where `S` holds there, or `E` does not and it holds a byte back.
    - **`escaped(B)` at one byte** is whether the run of `B`s before it is odd. A window searched from a byte inside
      the token starts its carry the same way, so its windows need not start at the token's first byte. A window
      reading `escaped(B)` a byte on shifts the window's escapes down a byte, with the carry out as its last bit, and a
      window reading the same escapes twice computes them once.
    - **A pairs mask a byte back** (`prev(two_start)`, `two_start = two & ~escaped(two)`) is `escaped(two)`: a byte is
      escaped exactly where the byte before is a `two` not escaped itself.
    - **A conjunction with a side that holds nowhere** (`i_open & prev(empty)`) holds nowhere, so it reads none of
      its other side.
    - **A complement a few bytes on or back** in a block's word masks with the bytes valid there, not the block's own:
      `~prev(~empty)` holds at the file's first byte.
    - **The counts' scans** of the tokens' bytes stay where the loop reads a count past them: `depth`, for the line
      breaks' marks (`depth.eq(0)`) and the string reader's bound.
    - **The reader's code** reads one mask a call: the stops by the format flag, each quote's mask by the quote, a
      brace's run by which brace it is.

    Tier 2 asks the token marks the loop no longer records (`trail`, `future`: `UNRECORDED`), as rr's tier 2 does.
    The tests compare the records without those bits, as rr's do.

89. **A token's start is tested at its first byte, not as a mask of its block.** The loop picks a token's row as the
    lean lexer's `token` does: by its first byte's group, then each row's start tested at that byte, of the bytes
    there and around it (`bit(&M, s)` read as `M` at `s`). A block's word of `M` stays only where `M` has no test of
    one byte. In Python's tier 1, the block's masks go from 63 to 19 and its classes from 30 to 19: what is left is
    what the runs read (a word's, a number's, the dots', a brace run's), the string reader's stops and refusals, the
    bracket count's search of a token's bytes and the two indentation escapes.

90. **A string's text is read once, by one function each of its arms calls.** As the lean lexer's `string` loop,
    the reader's text from stop to stop is one function (`string_text`): from its first byte to its closer or an
    interpolation's `{`, its pieces written after the string's record, each of the kind its caller gives, and `None`
    where the file is refused. The string's arm and a tail's (`}` at a template's level 0) call it. It reads the bytes
    through windows, as the reader's state does. Where the loop reads stage 1's masks, each arm reads the text in
    place still: that form pauses for stage 1's blocks, which a function cannot.

91. **The loop declares no state it never reads.** Where the loop reads the bytes, a declaration before it goes
    where no line of `lex_on` after it reads its name (a binding of a `let` or a comment is no read), with the
    comment before it. In Python's tier 1 that is the regions' state the string reader takes the place of
    (`template`, `held`, `opened`, `level`, `frames`, `depth`, `close`). The reader's own flag that an interpolation's
    `{` ended the text is `to_code`, apart from the regions' `opened`.

92. **F-strings in a lexer of strata** (the user's choices, for the strata design under review: tier 1 as masks
    built stratum by stratum, comments and strings, bracket depth, significant line breaks, then tokens). An
    f-string's interiors are lexed recursively, by the same strata as code, where the interior is determined
    reliably. A nested f-string (an f-string opened inside an interior) is a rare path, and may refuse the file to
    tree-sitter: nested f-strings are not common.

93. **The strata spike** (the user's choices).
    - **A hand-written SIMD lexer of strata** is written beside the lean Python lexer, on the `std::simd` interface
      (`rerail-simd`, the uglykl CPU SIMD branch's clone of it on stable Rust), so that the two can be compared for
      speed and for accuracy.
    - **A nested f-string bails:** the file is refused (to tree-sitter). It is a spike; the rare path is not decided.
    - **The bracket depth's bound (31) is a rule:** a file past it is refused, as now.
    - **The strata are made with bit operations,** the regions' openers and their terminators included.

94. **The strata spike, as built** (`rerail-python-lean`'s `strata`, beside the lean lexer; its own tests, and
    `examples/compare_strata` over a directory of files).
    - **Its output is the lean lexer's** (`Lexed`), compared token by token and extra by extra. Both read numbers,
      operators, escapes and string prefixes with the same functions (moved out of the lean lexer's methods).
    - **The bracket depth is clamped at 0 as the lean lexer's counter is,** made by a walk over a block's brackets'
      bits: on the corpus 0.37 cycles a byte, against 0.50 for a prefix sum and a prefix minimum of the block's 64
      lanes on `std::simd`'s interface, which is removed. The walk reads it as one bit of `depth0`.
    - **An f-string's escaped braces (`{{`, `}}`) are stepped a pair at a time** at the text's stops, not found by
      an odd-run mask: a run of `}` can start inside an interior (`f"{x}}}"`), where an odd-run's parity is wrong.
    - **`\N{…}` in an f-string is refused**, as a nested f-string is: the name's braces are not an interpolation's.
    - **A string's prefix letters are read at its quote,** a few bytes; the block's `{`, `}`, `:` and `!` are made
      from its bytes only where an f-string reaches it. A block's classes are 11 masks.
    - **The walk reads rr's run:** from a token in code, the block's starts are one mask up to its first byte that is
      not code (stratum 2's) or ends a line, so no layout and no string's prefix comes between the run's tokens.
    - **At a line's end the scanner's look is taken once:** the layout tokens take no bytes, so it is the same for
      each.
    - **As measured** (the 1644-file corpus, an M1 Max): 1635 files lex the same, 0 differ, 9 are refused (each a
      nested f-string). The strata take 24.0 instructions and 8.18 cycles a byte, the lean lexer 31.2 and 8.02: 0.98x
      its speed, both lexers timed a pass each in turn. The strata below the walk take 2.25 cycles a byte (classes
      0.96, regions 0.93, depth 0.36), the walk 5.9 against about 7.4 for the lean lexer's.

95. **The strata by the plan** (the user's choices: item 94's departures are rejected; "bit tricks only, no loops
    except an outer block loop"; both region variants built and measured).
    - **The depth is the plan's prefix sum,** in SIMD registers: a block's 64 lanes as four `i8x16`, four shifted
      adds each, then the carries across registers. The clamp at 0 is a prefix minimum, taken only where a block's
      sum goes below 0. The bracket walk is removed.
    - **Escaped braces by masks:** a `{` is escaped by the odd-run trick over the `{` runs (`lbu`), carried from the
      text's start; a lone `}` by the same trick over the `}` runs past the interpolation's closer. The pair stepping
      is removed.
    - **A string's prefix by masks:** the prefix letters' masks of the block and the one before, and the nearest
      byte below the quote that is not a prefix letter (`leading_zeros`). They are made only where a word byte lies
      just below a quote.
    - **Variant A, the plan's region step:** an outer loop over blocks; in a block each region's opener is the
      lowest live bit, its closer one mask search, all with mask operations; an f-string's interiors by the same
      step.
    - **Variant B, no step inside a block:** triple quotes by a three-state machine, one-line strings by a
      three-state machine reset at a bare line end, comments by an addition carry chain. The machines are bit-sliced
      and composed by Kogge-Stone in six rounds. Where no quote of one kind lies in a string of the other, each
      kind's strings are its quotes' prefix XOR (simdjson's), the plan's one-step shortcut. B reads an f-string's
      interior as text; each layer's guess is checked by masks, and a file whose check fails, or whose walk finds a
      string inside an interior, is read by A.
    - **The walk reads a string to stratum 2's closer** (`send`): its text stops only at that closer, a backslash
      and a format string's braces. Each string is read once by the masks, not again by its quotes.
    - **As measured** (the 1644-file corpus, an M1 Max): both variants 1635 the same, 0 differ, 9 refused (each a
      nested f-string); B reads 1641 of the 1644 files itself. Instructions and cycles a byte: the lean lexer 32.6
      and 8.5, A 28.5 and 8.9, B 28.1 and 8.8; timed a pass each in turn, A 0.95x the lean lexer's speed, B 0.96x.
      Classes 1.26 cycles a byte, regions A 0.92 and B 0.78, depth 0.45, the walk about 6.3 (the sampled profile:
      the walk 71% of the time, classes 14%, regions 9%, depth 5%).

96. **The walk's tokens from their start bits; stratum 1 by nibble tables** (as built, after a profile of B).
    - **Profiled** (the M1's bottleneck counters per stratum, the time profile by line): the walk took 6.3 of B's
      8.8 cycles a byte, 2.4 of them discarded (mispredicted) work; stratum 1 was bound by its vector operations.
    - **A run's tokens are its start bits taken one by one** (`starts &= starts - 1`), so no token's end waits on
      the one before: a name ends at its run of word bits (on into the next block's where it reaches the end), an
      operator after its byte. Which of the two a token is, and a name's keyword (a perfect hash of its first 8
      bytes, its hit a mask of the comparison), are selections. A token at stratum 1's `slow` bit is read by the
      step: a digit, a quote, a byte past ASCII, and a byte a two- or three-byte operator may start at. A run goes
      on into the next block while it meets no stop.
    - **Stratum 1 makes two class bytes a block by simdjson's nibble tables** (`rerail_simd`'s `lookup16`), so
      that a mask made of several bytes (`word`, the brackets, `slow`) is one bit test.
    - **A line's end read inside the run** (its layout one newline where the next line has the same indent) was
      measured equal and is not kept.
    - **As measured:** 1635 the same, 0 differ. B 1.01x to 1.04x the lean lexer's speed in turn (was 0.96x), A
      0.99x to 1.02x; stratum 1 1.34 cycles a byte with the `slow` mask, the walk about 5.5.

97. **The lexer of strata from the middle: a chunk lexed from its entry** (0715's pass 2, as built; the design page
    "Superposed Lexer Design").
    - **Stratum 2 from the middle is variant B's.** Its carries are a few bits (the two machines' states, a
      comment's carry, a line end's bit, a triple closer's taken bits), and an f-string is a plain string to it: its
      interpolations are read by the walk inside the string's token, so the f-string nest never crosses a chunk's
      start. A text B reads wrongly is read serially by A, as `lex_with` reads it.
    - **What crosses a chunk's start** (`strata::middle::Entry`): stratum 1's three escape bits into block
      `from - 2`, B's carries and the depth into block `from - 1` (the walk reads the depth below its first item, and
      stratum 2 a string's prefix in the block below its quote), and the walk's state at its first item from byte
      `64 * from` on: where it starts, whether the line has a token, whether it is a block's header, the indents.
    - **Each stratum runs over a range of blocks from its carries** (`classes_in`, `regions_b_in`, `depth_in`); the
      whole text's run is the range of all its blocks. The walk stops at its first item past the chunk's end, and
      an item it started is read to its end. The strata are made over a window from block `from - 2` to one past
      the chunk's end; where the walk reads past the window (`Masks::short`), the window is made again twice as
      long.
    - **As tested:** each chunk lexed from the exit of the one before is the serial lexer's records, on the corpus
      (1644 files) at 1, 2, 3 and 16 blocks a chunk: 1641 the same, 0 differ, 3 read by A. Where both refuse, the
      byte each refuses at is not compared (each its own order of reading). The serial lexer's speed is as before
      (B 1.04x the lean lexer's in turn, the host loaded).

98. **Passes 1 and 3 for strata 1 to 3** (as built: each chunk's summary by running B from every lane).
    - **Stratum 1's carries need no lane:** an odd run of backslashes, a `'\r'` an odd run escapes, and an odd run
      of `{`, each ending just below the block, are read from the bytes below it (`s1_at`).
    - **Variant B's carries are 144 lanes:** the triples' machine's state (3), the one-line strings' (3), the
      comment's carry, the line-end bit, and the triple closer's taken bits (4). B's layers each read as if the ones
      above were not there, so the states do not exclude each other: a quote in a comment moves the strings'
      machine, and an entry of a one-line string and a comment's carry at once was found on the corpus when the
      lanes were six region states.
    - **The depth is a clamped counter:** a range's run is `max(d + net, net - min)` out and `max(d + max, k)` at
      its deepest (the refusal past 31), and two ranges' runs compose to one (`Depth::then`).
    - **A chunk's summary** is each lane's exit lane and depth run, none where B's reading from it is wrong; pass 3
      composes the summaries by Hillis and Steele's scan, as JSON's stage 1 does, each round's chunks in any order.
    - **As tested:** each chunk's carries and depth by the scan are those the chunks lexed one after another leave,
      on the corpus at 1, 2, 3 and 16 blocks a chunk (1632 files, 0 differ; the other 12 refused or read by A), and
      on random texts with each pass in seeded random orders.
99. **The walk from the middle: the whole lexer by passes, each chunk on its own** (as built: `lex_middle`).
    - **The walk's place in a chunk (`probe`)** depends neither on its layout nor its indents (a layout token takes
      no bytes). It is found by entering the walk at the last place below the chunk where it goes on as the serial
      walk does, and walking it to the chunk's start. Such a place is a blank in code, a string's or a comment's
      start, a line continuation's end, or the text's start. B's code mask does not mark each item's end: a quote
      in a comment marks the comment's line end a string's.
    - A place inside an interpolation's code may not be one the serial walk goes on from: the walk then refuses or
      finds B's reading wrong, and the place before it is taken. Where the chunk's own block and the one below hold
      no place, the chunks further back are asked, at twice the distance each time.
    - A walk from every place refuses before the chunk (an f-string nested in an interpolation, which B refuses): the
      chunk is entered after a refusal. Either the chunk that holds the refusal refuses the text, or the chunk is not
      entered. It is never given a wrong entry.
    - **The walk's summary** has a lane for each of `open` and `header`'s 3 states, entered at the chunk's place
      with indents it does not know. The indents are a symbolic stack (`Sym`): the levels the chunk opened, and a
      filter `t` (the entry's levels above `t` closed, a dedent each, counted where the entry is known). It also
      keeps the reads the walk made of the entry's top, each a range that holds none of the entry's levels.
    - Two summaries compose to one: `b`'s reads are checked against `a`'s own levels and narrowed by `a`'s filter,
      and the levels and counts follow. A read that does not hold for every entry leaves the lane not known, and the
      chunks after it are not entered.
    - **Pass 3 for the walk** is the same scan (Hillis and Steele, each round's chunks in any order). Applying it to
      the text's start gives each chunk its `open`, `header` and indents, and the tokens and extras before it. Pass
      2 lexes each chunk from its entry, and its records are placed at those counts.
    - **The check:** each chunk's exit must be the next chunk's entry by the passes, and each chunk's records must
      start at the scan's counts; otherwise it is an error.
    - **As tested:** the corpus at 1, 2, 3 and 16 blocks a chunk gives 1644 of 1644 files the serial lexer's records
      (or its refusal, or variant A), with no chunk not entered and none wrong. The test texts and random texts
      (quotes, f-strings, comments, escapes, brackets and indents across blocks' edges) were run with each pass in
      seeded random orders.
    - **What these passes are, as built:** each chunk's summary runs B and the walk from every lane, one lane after
      another. Item 100 makes strata 2 and 3's pass 1 the superposed form.
100. **Pass 1 superposed for strata 2 and 3: every lane at once, no branch** (as built: `block_map`,
    `then_block`, `summary_superposed`, the pass 1 `lex_middle` runs).
    - **B's layers each read only the ones below them,** so a block's map over the 144 lanes is made one layer at
      a time. The triples' machine is made once and run from each of its 3 states. The strings' machine is made
      for each of the triples' 12 states (`t0` and the closer's taken bits). The comments' addition is made for
      each of the strings' 36. B's checks are masks; where one fails, the lane's exit is selected as `WRONG`, a
      lane that stays itself.
    - **The depth's run by masks** (`Depth::of_masks`): the prefix sums of stratum 3, their prefix minimum from 0,
      and the reductions give net, lowest, highest and the largest rise, with neither of `depth_lanes`' two
      tests.
    - **The prefixes' refusal** (a run of prefix letters below an opener that reaches the window's bottom) is one
      addition per lane. It reads `pfx`, a stratum 1 mask made from the block's bytes where they are loaded.
    - **The block loop is the only loop.** The lanes are unrolled, a zero block below the text's start stands in
      for a test of it, and each block's map is composed onto the lanes' runs so far by gathers (`then_block`),
      also unrolled.
    - **As compiled** (`crates/python_lean/scripts/check_strata_branch_free.sh`, aarch64): `block_map` is 28831
      instructions and `then_block` 4532, with no conditional branch and no call; the selections are `csel`s.
    - **As tested:** the superposed summary is B run lane by lane, on every lane of every chunk: the test texts and
      600 random texts at 1, 2, 3 and 16 blocks a chunk, and the corpus at 1, 2, 3 and 16 (1644 of 1644 files).
      `lex_middle` with it gives 1644 of 1644 files the serial lexer's. The serial lexer's speed is unchanged with
      `pfx` made in stratum 1: B 1.02× the lean lexer.
101. **Stratum 1 and pass 3 with no branch** (as built).
    - **Stratum 1 tests nothing per block.** Tabs, form feeds, `'\r'`, backticks and braces are a compare each in
      every block; before, each was made only in a block that had one, after one test.
    - **The last block is not copied.** Each block is one 64-byte load from the lesser of its start and the text's
      last 64 bytes, and its masks are shifted down by the bytes below the block the load starts at. The bytes one
      on are the same lanes one on, with the byte after them read at a clamped index and masked. `bytes` and
      `bytes_from` (the later strata's reads of a block's bytes) are the same load, with the lanes shifted by a
      table lookup.
    - A text under 64 bytes has no 64 bytes to load. It alone is read from its bytes padded with zeros, decided once
      per call (`classes_over::<SHORT>`), not per block.
    - The carries kept at a chunk's entry are a select, and the blocks are written by one `extend` (no test of the
      room per block).
    - **As compiled:** stratum 1's block loop has only the loop's own two branches (the test of an empty range and
      the back edge).
    - **Pass 3's composition is `then_block`.** A summary is a block map's form (each lane's exit, `WRONG` a lane
      that stays itself, and its depth's run), so composing two summaries is the gathers pass 1 composes blocks
      with. The depth's application at an entry is a select (`Depth::apply_sel`).
    - **As measured:** stratum 1's compares in every block cost the serial lexer: B 0.95× the lean lexer, from
      1.02×. The corpus gives the same records and entries as before (1644 of 1644 from the middle).
102. **The walk's token starts in code by masks** (as built: `strata::starts::code_starts`; the walk's pass 1 does
    not read it yet).
    - **Why:** the walk's pass 1 counts a chunk's records as the popcount of its token starts (the design page's
      plan). So each start the serial walk makes is made by masks too, for every byte of a block at once. The
      starts don't depend on the walk's lanes (`open`, `header`, the indents), so they are made once per block.
    - **Operators:** the longest of `operator`'s at each byte comes from masks of the byte and the two after it. The
      walk's choice among overlapping ones is a machine of three states (the next byte starts a token, or one or two
      more are the token's), composed by Kogge–Stone over a window of two blocks.
    - **Numbers** follow `lex::number_at`'s rules as runs found by carry additions: a digit run with single
      underscores, a fraction after a `.`, an exponent and its sign, a suffix, and the digits of base 16, 8 and 2.
      A number takes the `.`, the sign and the word-run starts it reads. A signed exponent's digits start a word run
      but are the number's, so the numbers are made twice, the second time without them.
    - **Names** are the other word runs' starts.
    - **The walk's quirks, as it reads them:**
      - A number that ends inside a word run (`1a`, `0x`, `0b15`) leaves a rest. The rest is a name where the
        walk's run stops before another token starts (the walk's next run starts there); otherwise the walk passes
        over it.
      - A rest that starts a block is a start, since the walk's next block reads its first byte as a run's start.
    - **Rare** (its chunk waits for the one before it; never given a wrong count):
      - a byte the walk refuses, or one past ASCII, in the block;
      - the run of non-blank code the window starts in reaches the block;
      - a rest that crosses a block's edge, or starts with a digit;
      - a `.` that starts a token at a block's last byte before a digit, which the walk reads as an operator;
      - a fraction's `.` before another `.`;
      - a signed exponent whose second reading takes another.
    - **As tested:** the starts in code are the serial walk's on every block that isn't rare: the test texts, 4000
      random texts of names, numbers and operators, and the corpus (223461 of 223857 blocks; 396 rare, 0
      differ).
103. **The walk's token starts in strings by masks** (as built: `strata::starts::string_starts`; the walk's pass 1
    does not read it yet).
    - **Why:** the strings' tokens count toward a chunk's records as its code's do (item 102).
    - **What it makes, from stratum 2's string starts and closers:**
      - a string's start, at its prefix;
      - its interior, from the opener's end (one quote or three, after the prefix's run of letters) to its closer;
      - its kinds (raw, bytes, format) from the prefix's letters, filled through the interior;
      - its escapes, at each backslash no odd run escapes, in a string not raw. In a bytes string `\N`, `\u` and
        `\U` are text. Each escape's length is `lex::escape_at`'s, from masks of the bytes after it (four hex
        digits, two, up to three digits, `\r\n`, the bytes of two);
      - a format string's brace pairs, `{{` and `}}`, by the odd-run trick over each run of braces from its first;
      - its text's runs: the interior's bytes that no token above takes;
      - its end, at the closer.
    - **The carry from a block to the next:** the open string and its kinds, an opener whose quote or end lies past
      the block (a prefix's letters are read into the next block), the bytes an escape or a pair takes past it,
      whether its last byte was text, and the `}` runs' parity.
    - **Rare** (its chunk waits for the one before it):
      - `\U` and `\N{` (escapes of more bytes);
      - an interpolation, and the rest of its string's blocks;
      - a `}` no `}` pairs with (the walk refuses it);
      - a raw string's backslash where its text has not begun, and any backslash of a raw format string (the walk
        reads these by tokens of their own).
    - **As tested:** the starts in strings are the serial walk's on every block that isn't rare: the test texts,
      4000 random texts of quotes, prefixes, escapes, braces and line ends, and the corpus (214698 of 223857 blocks;
      9159 rare, 0 differ). Most rare blocks hold an interpolation, which the next layer reads.
104. **The walk's starts in interpolations by masks** (in `strata::starts::string_starts`).
    - **Where each one ends:** one prefix sum `g` of the brackets over a format string's bytes per block. An
      interpolation from its `{` (where `g` is `G`) ends at the first `}` after it with `g` at `G − 1`, or at the
      first `:` at `G`. A `:` starts its format spec, which ends at the next `}`. A `!` at `G` before no `=` is a
      conversion of two bytes.
    - **In rounds:** each round takes the open interpolation or spec to its end, then the next `{` after it. Five
      rounds are unrolled: the carry's, and four more. A block with a fifth interpolation is rare.
    - **The tokens inside:** the starts in code (item 102) over the interpolations' bytes, from the function the two
      share (`starts_in`). The serial interpolation reads one token at a time, so a number's rest of a word run is
      always a name there.
    - **Carried to the next block:** an interpolation or spec open past the block, its depth there, its bytes of
      code (the next block's window reads them), and a conversion's `!` at the block's last byte.
    - **Rare:**
      - a string, a comment, a backslash or a line's end in an interpolation;
      - a bracket it does not open, closed in it;
      - a conversion with no letter;
      - a `{` or a line's end in a spec;
      - more interpolations than the rounds.
    - **As tested:** the starts in strings are the serial walk's on every block that isn't rare: the test texts,
      4000 random texts (with `!`, `:`, brackets and `f'` added) and the corpus. Rare blocks drop from 9159 to 781
      of 223857, and none differ.
105. **The walk's extras by masks** (as built: `strata::starts::extra_starts`).
    - **Why:** the walk's summary counts a chunk's extras as well as its tokens. Like the starts, the extras don't
      depend on the lanes.
    - **Comments:** stratum 2 keeps each comment's `#` (B's `cstart`), where each variant finds it: variant B from
      its comments' generators less those a comment carries into, and variant A where its walk opens a comment.
      Each is one extra.
    - **Continuations:** each backslash in code is one extra, unless the next line's first byte past its blanks
      starts a string's prefix and quote. The walk reads that case without one. Found by masks: the bytes from which
      prefix letters reach a quote, then those from which blanks reach one of them, two and three bytes past a
      `\` and `\n` or `\r\n`.
    - **Rare:** a backslash before no line end (the walk refuses it); a next line's blanks and letters that reach
      the window's top.
    - **As tested:** the extras are the serial walk's on every block that isn't rare: the test texts, 4000 random
      texts of comments, backslashes, line ends, prefixes and quotes, and the corpus (all 223857 blocks; none
      rare).
106. **The walk's loop tops by masks, in place of the serial probe** (as built: `strata::starts::tops`,
    `middle::first_top`; `probe` and `from_place` removed).
    - **Why:** each chunk's walk enters at its first loop top from the chunk's start on. The serial probe found it
      by walking from a place before the chunk, and a walk branches. The loop tops don't depend on the lanes, so
      masks give them.
    - **What a loop top is:** each iteration of the walk reads the layout at its top, then one item: a run of
      tokens in code (`FAST` is every byte but blanks and line ends, so in code each iteration reads a run), a
      string whole, or an extra. So the tops are the items' ends:
      - a run's end: after the last byte of code that is not a blank, with only spaces and tabs between it and the
        run's stop;
      - a string's: after its closer (one byte, or three where the byte after the first is not code nor a
        string's or a comment's start);
      - a comment's: at its line end, which stratum 2 now keeps (B's `cend`, in both variants), or at the text's
        end;
      - a continuation's: at its next line's start.
    - **A chunk's place:** its first top. A chunk with none takes the chunk after's, by a scan from the last. Past
      the last top the text's end has been read (`done`). Chunk 0 enters at the text's start.
    - **As tested:** at every block's start, the first top by masks is the walk's (its entry by `lex_chunked`, `p`
      and `done`): the test texts, 4000 random texts, and every block of the corpus's 1632 files. The corpus lexed
      from the middle is still the serial lexer's in 1644 of 1644 files, at 1, 2, 3 and 16 blocks a chunk.
107. **The walk's summary by masks, its layout from its events** (as built: `middle::walk_summary_masks`; `lex_middle`
    does not call it yet, and its events are folded one after another, with branches).
    - **Why:** the walk's pass 1 is a chunk's summary from each of `COMBOS`. Its tokens and extras are lane-free
      (items 102 to 105), as are the iterations' tops (item 106). What depends on the lane is the layout, and the
      layout happens at a few places: the loop tops where a logical line ends.
    - **The chunk's range:** from its place to the chunk after's. Its tokens and extras are the popcounts of their
      starts there, plus the text's end where the chunk reads it.
    - **An event:** a loop top in the range whose next byte, past spaces, tabs and form feeds, is a line end or the
      text's end. Read from masks:
      - in brackets or not (stratum 3's `depth0`);
      - the next line's width and the first comment's, from the first byte of content past the line end and the
        first `#` before it. Each width is the spaces and 8 per tab since the last line end, `\r` or form feed,
        by popcounts;
      - whether the next line starts with a quote.
    - **Between events:** whether a token starts since the event before (the walk's `open`), and whether the last
      is a `:` out of brackets (its `header`).
    - **Each lane:** the layout loop's rules over a fixed-size lane: up to 8 levels it opens, `t`, its phantoms, up
      to 8 reads of its entry, the layout tokens, `open` and `header`. It converts to `Sym`.
    - **The dedent check in brackets** reads the walk's last two records, which may be layout tokens, so each lane
      keeps whether layout tokens came after its last token and between the last two. A name and a keyword differ
      there, and the walk's keyword lookup gives which.
    - **Not known** (the lane is `Unknown`): a rare block in the range; a line end whose next content lies past
      the window of the block and the one after; a continuation before a blank line; more levels or reads than the
      lane holds; a dedent check that would refuse.
    - **As tested:** each lane by masks is the walk's (`walk_summary`, the conditions compared as sets) wherever
      masks know it: every chunk of the test texts at 1, 2, 3 and 16 blocks, 1500 random texts of indented lines,
      headers, brackets, comments and blank lines, and the corpus (1641 files; at 1 block a chunk 653880 lanes the
      walk's and 31137 not known; at 16 blocks, 31452 and 13713; none differ).
108. **`lex_middle` takes the walk's pass 1 by masks; a chunk masks don't know waits** (as built: in
    `middle::lex_middle`).
    - **Pass 1** of the walk is `walk_summary_masks` for every chunk. The serial walk (`walk_summary`) stays as the
      oracle.
    - **Pass 3** is the scan as before (`walk_entries_in`). A chunk the scan gives no entry, because a summary
      before it has a lane masks don't know, waits for the one before's entry. It is that chunk's lane applied to
      its entry where masks know the lane; else that chunk is lexed from its entry (`lex_chunk`), as pass 2 would.
      Then the chunks after it go on from there.
    - **As tested:** the corpus lexed from the middle is the serial lexer's in 1644 of 1644 files, at 1, 2, 3 and 16
      blocks a chunk, and the crate's tests hold.
109. **The walk's pass 1 by masks has no branch: one block function, every lane at once** (as built:
    `middle::walk_block`, `layout_block`; `starts::Win`).
    - **One block function** (`walk_block`) does a block's whole share of the walk's pass 1: its tokens' starts in
      code and in strings, its extras, its loop tops, and its layout for the 3 lanes together. Only the loop over
      blocks branches. It reads the block's masks, the block below's and the next one's from the window (`Win`, by
      value), and passes them in.
    - **The layout's events** are at most 6 a block, in 6 steps written out (a loop of them stays a loop). More than
      6 is not known: 0.024% of the corpus's blocks, 3 of its lanes. Each step is selections. The lane's levels and
      reads stay in fixed arrays.
    - **The dedent check's kinds** are worked out when a token is read, from its own block's masks and a few of its
      bytes, by a byte table and the keyword's perfect hash, and are carried as codes. So no step reads a token
      from an earlier block.
    - **The text's bytes:** a text of 64 bytes or more is read by one load from the lesser of the byte and the
      text's last 64 bytes' start. A text under 64 bytes is read byte by byte at clamped indexes, with no copy. The
      block function has one instance for each, chosen once a text.
    - **The closures called more than once** may stay calls. The numbers are an `#[inline(always)]` function over
      what they read (`NumCx`). The interpolations' rounds and the classes' reads are local macros. Each round is a
      closure called once, so clippy's `unnecessary_cast` stops at it: that lint walks a cast local's definitions
      with no memo, and through rounds written out as locals it ran for hours.
    - **As tested:** `scripts/check_strata_branch_free.sh` reads both instances (AArch64): 0 conditional branches and
      0 calls. Every count of items 102 to 107 over the corpus is as before, except 3 lanes not known (blocks of
      more than 6 events). The corpus lexed from the middle is still the serial lexer's in 1644 of 1644 files, at 1,
      2, 3 and 16 blocks a chunk.
    - **On x86-64:** the baseline the engine targets has AVX2, and the build now says so (`.cargo/config.toml`:
      `target-cpu=x86-64-v3` for x86-64). Without it a build was SSE2 only, and `rerail_simd`'s `swizzle_dyn` (its
      `pshufb` path needs SSSE3) was a loop of bytes: 274 branches in `walk_block`, 144 in `block_map`. With it,
      both are 0, as are `then_block`'s.
    - **The block below is chosen by the loop over blocks:** below the text's first block it is an empty block at
      depth 0. A selection of a load against a constant in the block function compiled to a branch around the load
      on x86-64, `select_unpredictable` too.
    - **As tested on x86-64** (a Linux host): `check_strata_branch_free.sh` gives 0 conditional branches and 0 calls
      in `block_map`, `then_block` and both instances of `walk_block`; `classes_over` keeps its loop's two.
110. **The walk's pass 3 in fixed-size lanes, with no branch** (as built: `middle::WLane`, `then_walk`,
    `walk_entries_in`).
    - **A lane:** what it is (it goes on, it refuses, or masks do not know it), its exit, its refusal, up to 8
      levels it opens (`own`), `t`, up to 8 reads of its entry, and its tokens and extras. A lane of pass 1 whose
      levels or reads do not fit is not known.
    - **Lane `a` then summary `b`** (`then_lane`, `Sym::then` in a fixed size): `b`'s lane of `a`'s exit, chosen by
      selections. Each of its reads of its entry, written out 8 times: not known where one of `a`'s levels is in
      it, else the read after `a`'s top, kept once. `a`'s levels at most `b`'s `t`, a prefix since they increase,
      then `b`'s own. The levels `b` closes are dedents. More than the arrays hold is not known, so the chunk waits
      for its entry, as in 0715. A lane that refuses or is not known stays as it is.
    - **The scan** (`walk_entries_in`) is Hillis and Steele's over `then_walk`, as before. The scan by `Sym`
      (`walk_entries_by_sym`, over `compose_walk`) stays as the oracle.
    - **x86-64:** LLVM's cmov converter turns every conditional move with a memory operand into a branch around the
      load, `select_unpredictable`'s too. So x86-64 builds turn that off
      (`-x86-cmov-converter-force-mem-operand=false`, `.cargo/config.toml`).
    - **As tested:** `walk_scan_is_syms`: every entry of the fixed-size scan is the scan by `Sym`'s, over the
      summaries by masks, for the test texts at 1, 2, 3 and 16 blocks and 600 random texts of lines at 1, 2 and 3,
      each scan in seeded random orders. Over the corpus none differs and none is lost to the fixed sizes (59840
      entries known at 1 block a chunk, 20546 at 3, 4557 at 16). The corpus lexed from the middle is still the
      serial lexer's in 1644 of 1644 files. `check_strata_branch_free.sh` gives `then_walk` 0 conditional branches
      and 0 calls on AArch64 and x86-64, and the crate's tests pass on both.
111. **The lanes masks do not know, by cause; a next line past the window read from the chunks after** (as built:
    `middle::WHY_*`, `Far`, `chunk_firsts`, `walk_inputs`, `walk_why_counts`).
    - **The causes:** pass 1 carries why a lane is not known as bits, by selections, not one flag. `walk_why_counts`
      and `compare_strata … why` count them over the corpus. At 1 block a chunk, of 685017 lanes, 26763 were not
      known only because a line end's next line had no content in the block's two-block window ("far"). The
      window has 128 bytes, and a run of comment lines is longer. Next came starts rare in code (2223) and in
      strings (1866).
    - **The next line past the window** (`Far`): the first content past a block's window, its line's width, and
      whether it is a quote or a backslash. Each block's window gives the first content in its upper half, its
      width read there (`far_block`, no branch). A chunk's own is the first of its blocks'. A chunk's after is the
      first of the chunks' after it, filled from the right as the places are (`walk_inputs`). In pass 1, a
      backward loop over the walk's blocks gives each block the first past its window. An event whose window has
      no content after its line end takes the far content's width, quote and backslash. It stays not known when
      its first comment is not in the window, or the far content's line starts below the window it is read in.
    - **Two errors this showed:** the mask below an event's next content was 0 where the window has none (a shift
      by 128, now a selection). And a comment from below the window was content in it: its bytes before the
      window's first comment end (`cend`) with no `#` before it are now comment.
    - **As tested:** a test of a header, then 8 comment lines, then a dedented line, at 1, 2 and 3 blocks a chunk:
      every lane known, and the walk's. The random layout test adds runs of comment and blank lines. Over the
      corpus at 1 block a chunk, 4521 lanes not known of 685017 (31140 before): far 144, code 2331, strings 1875.
      At 3 blocks 3618 (23148 before), at 16 blocks 2487 (13713). No lane differs from the walk's. The scan in
      fixed size knows 143215 entries at 1 block a chunk (59840 before). From the middle is still the serial
      lexer's in 1644 of 1644 files. `far_block` has 0 conditional branches and 0 calls on AArch64 and x86-64.
112. **Rare starts by cause; a run's name start, and strings in an interpolation** (as built: `starts::CODE_*`,
    `STR_*`, `rare_causes`, `compare_strata … rare`).
    - **The causes:** the starts in code and in strings give why a block is rare as bits. Over the corpus's
      228283 blocks, 404 were rare in code because the run of non-blank code the window starts in reaches the
      block. 523 were rare in strings because an interpolation's code held a string: its quote is a byte of code
      with no operator. 474 more carried that along their open string.
    - **A run's name start:** a word run whose first byte is no digit, after no `.`, is a name's start, whatever
      is below the window. An operator ends at a word byte, and no number goes on into such a run. So a run that
      reaches the block holds a name start below it, and the choices from there are the window's own. Only a run
      with none is rare.
    - **A string in an interpolation's code:** B reads it by its quotes (`Walk::string` at a depth). Its quotes
      pair by a prefix XOR from the one the block before left open (`StrCarry::iq`, and its kind, `idq`). Its
      records come by masks: its start at its opener, its text after it where it has some, its end at its closer.
      Its bytes leave the interpolation's code. A block stays rare where such a string has both kinds of quote, a
      prefix or a triple quote, or holds a byte the rounds read (a brace, a bracket, `:`, `!`), a backslash, `#`
      or a line end.
    - **As tested:** the test texts gain strings in interpolations: plain, empty, one past a block's edge, one
      with the other quote, one with a prefix, one holding a `:`. Over the corpus: rare in code 104 blocks (396
      before), rare in strings 101 (780), and no block's starts differ from the walk's. At 1 block a chunk, 1263
      lanes not known of 685017 (4521 before), at 16 blocks 735 (2487). The scan in fixed size knows 191939
      entries at 1 block a chunk (143215 before). From the middle is still the serial lexer's in 1644 of 1644
      files. The block functions keep 0 conditional branches and 0 calls on AArch64 and x86-64.
113. **From the middle, measured: lanes, waits and cycles a byte per pass** (as built: `middle::lex_middle_to`,
    `Stage`, `MiddleCounts`, `compare_strata … passes`).
    - **How:** `lex_middle_to` stops after a pass and counts the chunks, the lanes masks do not know, and the
      waits. `compare_strata … passes` gives the counts per chunk size. With `STAGE` it runs `PASSES` passes to
      that stage over the files lexed from the middle; stage 0 is the setup alone, stage 7 the serial lexer
      (variant B). A pass's cycles a byte is its stage's process cycles (`/usr/bin/time -l`) less the stage
      before's, over the passes' bytes.
    - **Lanes and waits** (the 1644-file corpus; chunks of the files B reads; a wait is a chunk the scan gives no
      entry: the chunk before's lane enters it, or it is lexed again from the chunk before's entry):

      | Chunk size (blocks) | Chunks | Lanes not known | Waits | By the lane before | Lexed again |
      |---|---|---|---|---|---|
      | 1 | 230,899 | 1,263 | 36,400 | 34,772 | 365 |
      | 2 | 115,867 | 1,173 | 18,117 | 17,157 | 335 |
      | 3 | 77,557 | 1,002 | 12,062 | 11,369 | 278 |
      | 16 | 15,216 | 735 | 2,258 | 2,006 | 182 |
      | 256 | 2,125 | 588 | 130 | 56 | 71 |

    - **Cycles a byte per pass** (an M3 Max, one thread, the host at load 30; 1632 files, 14.3 MB, 5 passes):

      | Pass | 16 blocks a chunk | 1 block a chunk |
      |---|---|---|
      | Strata's summaries (pass 1) | 131.2 | 150.7 |
      | Strata's scan (pass 3) | 8.4 | 235.2 |
      | The walk's inputs (firsts, fills) | 5.8 | 26.6 |
      | The walk's summaries (pass 1) | 60.3 | 150.0 |
      | The walk's scan and waits (pass 3) | 8.7 | 160.0 |
      | Each chunk lexed (pass 2) | 20.7 | 36.5 |
      | From the middle, all | 235 | 759 |
      | The serial lexer | 9.7 | |

      At 1 block a chunk the setup's own runs of `lex_middle` leave the serial stage's difference unmeasured.
    - **Where the cycles go** (`sample`, 16 blocks a chunk): in the strata's summaries, `block_map` 2754 of
      about 3000 samples. It runs B's layers once for each of its 144 lanes, each with its own depth by prefix
      sums (28831 instructions a block). In the walk's summaries, `walk_block` 835 samples beside `block_map`'s.
      At 1 block a chunk the two Hillis and Steele scans cost most: each round composes every chunk's summary.
114. **The strata's pass 1 shares its lanes' depths** (the user: "the entire point of superposition is to share
    work"; as built: `middle::block_lanes`, `block_map`, `Sets`, `block_map_each`).
    - **What the lanes share:** a lane's depth reads only the brackets in its code. A lane's code is the same for
      both values of `cext_prev`, so the 144 lanes have 72 codes (`block_lanes`). The codes hold few different
      sets of brackets: over the corpus's 230843 blocks, at most 6 a block, at most 3 in 98.4% (`distinct_codes`,
      `compare_strata … codes`). By the whole code there are up to 9.
    - **The block function:** the sets chosen once each, by selections over the 72 codes written out (`Sets`, at
      most `REPS` = 6). Each set's depth made once, and each lane's read from its set's. The map is written into
      the caller's lane by lane, not returned and copied. A block with more sets is `block_map_each`'s, a depth
      for each code: the loop over blocks takes it there.
    - **As tested:** `shared_depths_are_each_codes`: the shared map's exits and live depths are `block_map_each`'s
      for every block of the test texts and of 3000 random texts of brackets, quotes, comments and line ends.
      Some of those have more sets than `REPS`, so the fallback path is tested too. From the middle is still the
      serial lexer's in 1644 of 1644 files. `block_map` has 0 conditional branches and 0 calls on AArch64 and
      x86-64: 15270 instructions a block on AArch64, from 28831.
    - **As measured** (as item 113, 16 blocks a chunk): the strata's summaries 64.5 cycles a byte, from 131.2. From
      the middle, all passes 168.8, from 235. The rest of `block_map` is its layers: the strings' machine for each
      of the triples' 12 states, and the rest for each of the 36 and 144 beneath them.
115. **Pass 1's layers share more: the sets in vectors, the strings' machine once a triples' state** (as built:
    `middle::Sets::of`, `run_from_2`).
    - **Counted first:** `block_map`'s x86-64 instructions by source line put about 3500 to 4000 in choosing the
      sets (72 codes against up to 6 keys, scalar) and about 3200 in the 13 runs of `machine`.
    - **The sets in vectors:** the 72 codes' brackets as nine `u64x8`. A set's brackets are those of the first
      live code left (a bit set the lanes' checks make). One compare over the nine both clears its codes from
      those left and selects its index into theirs.
    - **The strings' machine once for each of the triples' 3 states,** not each of their 12. `tr_taken` changes
      only bytes 0 and 1 of the quotes it reads, so the machine is composed from byte 2 on (the masks shifted
      down 2). Each value's first two bytes are two steps of the machine's transition, and its runs are read from
      the state they leave (`run_from_2`).
    - **As tested:** the 30 tests pass on AArch64 and x86-64: `superposed_is_per_lane` (B run from each lane) and
      `shared_depths_are_each_codes` among them. From the middle is still the serial lexer's in 1644 of 1644
      files. `block_map` has 12640 instructions a block on AArch64 (15270 before, 28831 before item 114), 0
      conditional branches and 0 calls on both.
    - **As measured** (as item 113, 16 blocks a chunk, the host at load 5 to 12): the strata's summaries 280
      instructions a byte (533 before item 114) and about 55 cycles a byte. From the middle, all passes about
      166 cycles a byte.
116. **Pass 1's later blocks read on tracks: one for each state the lanes are in** (as built:
    `middle::block_map_tracks`, `TRACKS`, `lane_states`).
    - **Counted first** (`compare_strata states`): after a chunk's first block its lanes are in few states. At 16
      blocks a chunk, 98.6% of the later blocks have 3 or fewer different states before them, and 1.2% have 17
      or 18. At 2 blocks a chunk, 85.8% have 3 or fewer.
    - **The tracks:** a chunk's first block is read from every lane, as before (`block_map`). The states its lanes
      leave are found by a set of bits and its first `TRACKS` = 3, by selects. Where there are no more than 3,
      each later block is read from those states alone, and each lane follows its state's track. The layers are
      the same functions for both: `shared`, `t_layer`, `s_layer`, `c_layer`, `lane_exit`. `block_lanes` runs
      them for every state, `block_map_tracks` for its 3. Where there are more, every block is read from every
      lane. That choice is made once a chunk, in the loop over blocks.
    - **As tested:** `shared_depths_and_tracks_are_each_codes` compares the tracks' exits and depths with
      `block_map_each`'s from each lane and from `WRONG`. The 30 tests pass on AArch64 and x86-64. From the middle
      is still the serial lexer's in 1644 of 1644 files. `block_map_tracks` has 686 instructions a block on
      AArch64 (812 on x86-64), against `block_map`'s 12631; 0 conditional branches and 0 calls on both.
    - **As measured** (as item 115, the host at load 13 to 17): the strata's summaries 100 instructions a byte
      (280 before) and about 21 cycles a byte (55). From the middle, all passes about 118 cycles a byte (166).
117. **The walk's layout steps: 2 a block, 4 more for a block of more events** (as built: `middle::FIRST_EVENTS`,
    `layout_more`, `walk_more`).
    - **Counted first:** by source line, about 35% of `walk_block`'s 20487 x86-64 instructions were the 3 lanes'
      layout steps (`line`, `cond`, `in_brackets` and their compares). They ran 6 events a block for every block,
      though a Python line is about 35 bytes. Only 6.7% of the corpus's blocks (16044 of 241179, at 16 blocks a
      chunk) have more than 2 events.
    - **The split:** `walk_block` takes the first 2 events. Where none is left it reads the tokens after them;
      else it leaves them, with the events left, to `walk_more`. `walk_more` reads the window's masks again and
      takes the other 4. Which runs is the loop over blocks' choice, as `block_map_each`'s is (item 114).
    - **A correction to item 116:** its 686 instructions were the symbol of a closure `block_map_tracks` called
      3 times, which the branch check matched first. The tracks are now a local macro's expansions:
      `block_map_tracks` has 2300 instructions on AArch64 and 2702 on x86-64, with 0 calls.
    - **As tested:** the 30 tests pass on AArch64 and x86-64. From the middle is still the serial lexer's in 1644
      of 1644 files. `walk_block` has 10720 instructions on AArch64 and 12914 on x86-64 (20487 before), and
      `walk_more` 6856 and 8074. All have 0 conditional branches and 0 calls.
    - **As measured** (as item 113, 16 blocks a chunk, on the M1 Max at load 2, both builds there): the walk's
      summaries from 22472M to 15310M instructions and from 4346M to 3055M cycles, about 61 to 43 cycles a
      byte. All the passes from the middle went from 8646M to 7343M cycles.
118. **The walk's pass 3 by look-back, as 0715's `entries_waits`; its Hillis–Steele scan removed** (as built:
    `middle::walk_entries_look_back`, `apply_walk`, `walk_after_by_sym`, `IN_FLIGHT`).
    - **Why:** the design page gives the nest's entries by the core's look-back. The walk still had a
      Hillis–Steele scan, which cost most at 1 block a chunk: log2 of the chunks rounds of `then_walk` over every
      chunk. A lane it did not know also spread to every chunk the composed ranges covered.
    - **The look-back:** the chunks go in waves of `IN_FLIGHT` = 32. Each chunk composes the summaries back to the
      last entry known (`then_walk`, no branch) and applies the composition to that entry. `apply_walk` is
      `Sym::apply` in the fixed size, for any entry's indents; `WLane::entry`, its case for `[0]`, is gone.
    - **Waits:** where the composition does not know a chunk's entry, the chunk waits for its predecessor's, and
      the chunks after look back to it. Its predecessor's entry is that one's summary applied to it. Where masks do
      not know that lane, it is the lane in `Sym` (`walk_after_by_sym`), else that chunk lexed again, as before.
    - **As tested:** `walk_scan_is_syms` compares the look-back's entries with the scan by `Sym`, with its waits
      taken by `Sym`: 0 files differ at 1, 3 and 16 blocks a chunk. The 30 tests pass on AArch64 and x86-64. From
      the middle is still the serial lexer's in 1644 of 1644 files.
    - **The counts now** (as item 113's table; "by the lane before" is 0 where the lane in fixed size gives the
      entry first; lexed again includes a wave's first chunk):

      | Chunk size (blocks) | Chunks | Lanes not known | Waits | By the lane before | Lexed again |
      |---|---|---|---|---|---|
      | 1 | 230899 | 1263 | 355 (36400 before) | 0 | 365 |
      | 16 | 15216 | 735 | 180 (2258) | 0 | 182 |

    - **As measured** (2 passes, on the M1 Max, both builds there): the walk's pass 3 went from 4779M to 521M
      cycles at 1 block a chunk, and from 112M to 6M at 16.
119. **The walk's starts in strings: interpolations only where a block may hold one; the byte classes once a block**
    (as built: `starts::interp_may`, `string_starts_w`'s `INTERP`, `walk_block`'s, `Cls`, `Bytes::from`,
    `SBytes::from`).
    - **Measured first,** with Instruments' Time Profiler on the M1 Max: each sample's address through `atos -i` to
      its innermost frame in the lexer's own source. The walk's largest costs were the byte classes
      (`sclasses`, `classes`), the interpolations' 5 rounds (their depth compares, `eqm`) and the layout's `cond`
      and `keep`.
    - **How it was not measured:** toggles that stub a part out change the records. The `passes` mode then
      drops the files whose records go wrong, so it lexes fewer bytes. One such toggle seemed to put 74% of the
      walk in the interpolations' code starts; they were about 10%.
    - **Interpolations where a block may hold one:** a window holds an interpolation only where the block has a
      `{`, or one or its format spec is open into it, or the block before had interpolations' code. About 11%
      of the corpus's blocks (26820 of 241239, counted before the open format spec was added to the test). The loop over blocks picks `walk_block`'s instance by it, a
      choice as `walk_more`'s (item 117). Without it, the rounds and the interpolations' starts are not read:
      they find none. The tests' builds run the rounds there anyway and assert that they found none.
    - **The byte classes once a block:** a byte's classes are of it and the two after it. So the classes at the
      bytes 1 to 5 after each byte are the block's and the next block's masks shifted, not classified again: 3
      `classes` a block for both starts in code (7 before, read again for interpolations' code), and 2
      `sclasses` (7 before).
    - **As tested:** the 30 tests pass on AArch64 and x86-64. From the middle is still the serial lexer's in
      1644 of 1644 files. The 4 instances of `walk_block` have 8080 to 13072 instructions on AArch64 (10720
      before) and 9637 to 13817 on x86-64 (12914). All have 0 conditional branches and 0 calls.
    - **As measured** (3 passes, on the M1 Max at load 4 to 7, both builds there): the walk's summaries went from
      1821M to 1380M cycles at 16 blocks a chunk, and from 9163M to 7239M instructions.
120. **The walk's byte classes carried from block to block; the layout's folds stay scalar** (as built:
    `starts::ClsCarry`, `cls_first`, `cls_next`).
    - **The carry:** a block's `Cls` holds the classes of the block below, the block and the block after, and
      the strings' classes of the block and the block after. All but those of the block after are the last
      block's. The walk's accumulator carries them, and each block classifies only the block after it: 1
      `classes` and 1 `sclasses` a block, not 3 and 2. The loop over blocks reads the carry into a chunk's
      first block. The tests' builds assert that each block's carried classes are its own as read afresh.
    - **The layout's folds, tried as vector compares and not kept:** `cond`'s test that a read is kept already,
      and `line`'s levels at most `m`. First as compares of the arrays loaded as vectors, then with the levels
      and the reads held in vectors (each written by a select on its lane, the top read by a masked reduce,
      against the store of one lane then the load of all). Both passed the tests. Neither cut the walk's cycles:
      1382M and 1397M against 1380M, with more instructions retired (7356M and 7527M against 7239M). The two
      folds are about 4% of the run's samples.
    - **As tested:** the 30 tests pass on AArch64 and x86-64. From the middle is still the serial lexer's in
      1644 of 1644 files. The 4 instances of `walk_block` have 7120 to 9696 instructions on AArch64 (8080 to
      13072 before) and 8899 to 11417 on x86-64. All have 0 conditional branches and 0 calls.
    - **As measured** (3 passes, on the M1 Max at load 4 to 5, both builds there): the walk's summaries went from
      1380M to 1290M cycles at 16 blocks a chunk, and from 7239M to 6650M instructions. At 1 block a chunk, where
      every block is a chunk's first, they went from about 3417M to 3344M.
121. **The walk's costs, profiled alone; one read of the entry a layout step** (as built: `LayoutLane::line` and
    `in_brackets` return their reads, `lane_step` keeps one).
    - **The profile:** Time Profiler on the M1 Max at 16 blocks a chunk, only the samples whose stack is in the
      walk's functions (`walk_block`, `walk_more`, `walk_summary_masks`): 9469 of 19982. Each sample's innermost
      frame in the lexer's own source, with the two it is inlined in:

      | Part | Samples | Of the walk |
      |---|---|---|
      | The layout's lanes (`cond` in `line` and `in_brackets`, `line`, its `keep`) | about 2000 | 21% |
      | The starts in code (`ks3`, `numbers_from`, `starts_in`, `classes`) | about 1400 | 15% |
      | The walk's window (`win`'s reads, `walk_summary_masks`, the strata's `regions_b_in` again) | about 800 | 8% |
      | The strings' classes (`sclasses`) | about 400 | 4% |

    - **The layout's lanes:** their samples are over about 300 addresses, at most 17 on any one, so the cost is
      the number of instructions, not a wait. `cond` ran 12 times a block: 2 steps, 3 lanes, and from both
      `line` and `in_brackets`. An event is out of brackets or in them, so at most one of the two reads is on.
      Where `line`'s is off it leaves the lane as it was, so `in_brackets` reads the same state. So each step
      keeps one read, `cond` 6 times a block.
    - **As tested:** the 30 tests pass on AArch64 and x86-64. From the middle is still the serial lexer's in
      1644 of 1644 files. `walk_block`'s instances have 6704 to 9248 instructions on AArch64 (7120 to 9696
      before) and 8284 to 10779 on x86-64. All have 0 conditional branches and 0 calls.
    - **As measured** (3 passes, 16 blocks a chunk, on the M1 Max at load 5, two runs): the walk's summaries
      1271M and 1246M cycles (1290M before), 6561M and 6325M instructions (6650M).
