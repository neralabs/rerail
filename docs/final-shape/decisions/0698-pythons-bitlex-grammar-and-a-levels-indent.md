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
