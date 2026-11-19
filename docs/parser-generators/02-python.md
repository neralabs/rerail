# Parser generators: Python without branches

Status: design proposal, 2026-11-06. It extends `01-branch-free-design.md`, whose §2 (columns from
extents), §3 (nearest smaller values) and §4 (stream operations) it assumes.

## Intro

JSON's tokens decide its tree on their own. Python's mostly do as well, but a long tail of cases
makes the same bytes parse differently depending on what is around them:

- a `"` that ends one string or opens one nested inside an f-string;
- a comment whose parent depends on the indentation of the next line of code;
- `(a, b)` that is a `tuple` on the right of `=` and a `tuple_pattern` on the left;
- `match` that is a statement keyword in one line and a variable in the next;
- `a.b(c)[d].e + 1`, where seven nodes start at the same byte.

This page takes each case and gives it a branch-free mechanism: per-byte or per-token facts,
prefix scans, nearest-smaller-value queries, gathers and scatters. Each tree shown below is
tree-sitter-python 0.25.0's actual output (`[start..end]` are byte offsets). The corpus figures
are from the data-layer repository: 13,042 Python files that parse without errors.

## 1. The shape of the problem

| Measure (data-layer Python) | Value |
|---|---|
| nodes per source byte | 0.20 |
| tape image per source byte | 1.75 B (8.71 B per node) |
| leaves | 66.3% of nodes |
| identifiers | 21.7% of nodes |
| `,` `(` `)` `.` `=` `:` | 25.6% of nodes |
| deepest bracket nesting | 14 |
| deepest block nesting | 13 |
| files with f-strings | 36.3% |
| files with a string inside an f-string's interpolation | 5.17% |
| files with that two levels deep | 1 |

How many nodes start at the same byte:

| nodes at one start byte | share of start bytes | cumulative |
|---:|---:|---:|
| 1 | 60.9% | 60.9% |
| 2 | 30.8% | 91.7% |
| 3 | 5.9% | 97.6% |
| 4 | 1.8% | 99.4% |
| 5–8 | 0.6% | 99.99% |
| largest | 75 nodes | |

The pipeline has five stages. Each stage reads the previous one's streams and has no branches:

```text
bytes  ──(1) modes──►  bytes tagged code | string | comment
       ──(2) tokens──► token stream: class, start, end
       ──(3) layout──► logical lines, brackets, blocks
       ──(4) nodes───► extents: start, end, rank, code
       ──(5) tape────► v9np columns (01 §2)
```

## 2. Stage 1: lexer modes

**The problem.** Whether a byte is code, string or comment depends on everything before it. Quotes
inside comments do not open strings, `#` inside strings does not open comments, and Python has
four quote forms, prefixes and f-strings. simdjson's prefix XOR only works for one quote character
with no comments.

**The answer** needs no automaton. `08-without-automata.md` derives it, and `lexers/python.bitlex`
writes it out with the constructs of `03-bitlex.md` §2.7:

- **Strings and comments.** Quotes act on the three states code, in `'` and in `"` as
  reflections, so which string a byte is in is a toggle and a count mod 3 (`reflect`). Short
  strings and comments end with their line, so each line is lexed three times, as if it started
  in code, inside `'''` or inside `"""`, and the line ends pick one (`hypotheses`). A `'''` inside a
  short string or a comment is found by a fixed point within the line (`rounds`).
- **F-string fields.** Depth is a count. Whether the bracket frame around a byte is a replacement
  field, and whether that field has passed its `:`, come from matching by depth (`count.frame`,
  `count.since`). The roles of braces are a fixed point over rounds, so format specs nest to any
  depth.
- **Python 3.12 quote reuse.** An outer `rounds` feeds the interpolation code it found back to the
  string scan, which then ignores the quotes in it.

Before these run, each byte gets its local facts:

- **Escapes:** simdjson's odd-backslash-run mask.
- **Triple quotes:** runs of one quote character cut into threes from their first byte.
- **Prefixes:** shifted letter masks.
- **Doubled braces:** runs of `{` and `}`, decided at their first byte.

The lexer's tokens equal tree-sitter's leaves on all 13,042 data-layer files tree-sitter parses,
and on 1,990 of 1,993 CPython files (`03-bitlex.md` §5). Quote reuse, as tree-sitter parses it:

```text
y = f"{f"{x}"}"
  string [4..15]                  outer f-string
    string_start [4..6] "f\""
    interpolation [6..14]
      expression: string [7..13]  same quote, nested
        string_start [7..9] "f\""
        interpolation [9..12] …
        string_end [12..13] "\""
    string_end [14..15] "\""
```

## 3. Stage 2: tokens

Once the modes are known, token boundaries in code are local facts:

- **Identifiers and keywords.** A token starts where the byte class changes. An identifier's first
  8 bytes and its length form a hash key into a table of the 35 keywords and the soft keywords
  (`match`, `case`, `type`, `print`, `exec`, `_`). Soft keywords keep a "soft" class until stage 3
  decides them. Bytes of 0x80 and above count as identifier bytes. A non-ASCII byte that cannot
  start an identifier, outside strings and comments, sets the error bit.
- **Operators.** Maximal munch (`**=`, `//=`, `->`, `:=`, `...`, `!=`) is a small automaton over
  runs of operator characters. Runs are short, so it is a table lookup on up to three bytes.
- **Numbers.** `1e-5` and `1.5j` contain `-` and `.`, but `0x1e+2` is `0x1e`, then `+`, then `2`. A
  `+` or `-` continues a number only after `e` or `E` in a decimal literal. "Decimal" is gathered
  from the token's first two bytes.
- **Line continuations.** A backslash followed by a newline, in code. It is a node in some places
  and not in others:

  ```text
  x = 1 + \           binary_operator [4..15]
      2                 line_continuation [8..10] EXTRA   a node here
  x = "e" \           concatenated_string [4..40]
      r"\q"             string, string                    no node here: the external scanner
                                                          skips it before a string start
  ```

  So it gets a node only when the next token is not a string start. That is a one-token lookahead
  fact.

Token positions are compacted into the token stream: about 0.13 tokens per byte. From here on,
"position" means a token index, and comments are removed from the stream. Section 7 shows that
their placement follows from the extents.

## 4. Stage 3: lines, brackets, blocks

**Brackets.**
- Depth is a `scan(+)` over `( [ {` (+1) and `) ] }` (−1) in code.
- Pairing is the nearest smaller depth (01 §3). The corpus's deepest nesting is 14, so the
  vector-max state fits 16 lanes.
- A mismatched pair (`(]`) or an unclosed bracket sets the error bit.
- Interpolation braces are pairs too, from stage 1.

**Logical lines.** A NEWLINE is a newline byte in code, at bracket depth 0, not after a line
continuation. The external scanner tests "inside brackets" by asking which tokens the parser
accepts (`00-problem.md` §5.2). For files without syntax errors, that is depth > 0. Blank lines
and comment-only lines are not logical lines.

**Indentation.** A logical line's width is the bytes from its line start to its first token, with
a tab counting 8 and a form feed resetting to 0, as in `scanner.c`.

**Blocks.**
- **A block header** is a logical line whose last top-level token is `:` and whose next logical
  line is indented deeper. The block runs to the next logical line indented no deeper than the
  header: a next-smaller-value query over widths. The deepest block nesting in the corpus is 13,
  so 16 lanes again.
- **Errors:**
  - a first body line not indented deeper than its header;
  - a dedent to a width that no open block has, which is an equality test against the query's
    result.
- **INDENT and DEDENT** are never materialised, since they are not tape nodes.
- **A body on the header's own line** (`if x: pass`) is a block that starts after the `:` and
  ends at the NEWLINE.

**Clause chains.** An `if_statement`'s extent includes its `elif` and `else` clauses, which are
indented like the `if`. The same holds for `try` with `except`, `else` and `finally`, and for `for`
and `while` with `else`. One trick folds this into the same query: give each line the key
`2 × width + continues`, where `continues` is 1 for a line led by `elif`, `else`, `except` or
`finally`. A header's statement then ends before the next line whose key is at most `2 × width`.
A clause line at the same width (key `2w + 1`) does not end it, and a new statement does. Which
statement an `else` belongs to is gathered from the chain's first line.

**Decorators.** `decorated_definition` starts at the first `@` line of a run of decorator lines at
one width and ends with the `def` or `class` after them. The run's start comes from a segmented
backward scan.

```text
@d1                       decorated_definition [0..24]
@d2(x)                      decorator [0..3]; decorator [4..10]
def f(): pass               definition: function_definition [11..24]
```

## 5. Stage 4: the same tokens, different nodes

Most of Python's difficulty is here. Each case below is decided by facts about a token's context,
and those facts are of three kinds:

- **neighbour facts:** the class of the previous or next token, a shift;
- **segment facts:** whether something occurs at the top level of the enclosing bracket or line,
  a segmented OR-scan;
- **enclosing facts:** a property of the enclosing bracket or region, a gather through the
  pairing from stage 3.

An enclosing fact that propagates inward through nested brackets (as targets do, §5.2) is the
vector scan of 01 §3 with a value per depth. A bracket reads the entry at its depth − 1, which is
its parent's value.

### 5.1 Soft keywords

| Line | Tree-sitter's reading | Deciding fact |
|---|---|---|
| `match = 1` | assignment to `identifier` | next token is `=` |
| `match(x)` | `call` | no top-level `:` ends the line |
| `match x:` with an indented body | `match_statement` | the line starts with `match`, ends with a top-level `:`, and opens a block |
| `type X = int` | `type_alias_statement` | `type`, then an identifier, then optional `[`, then `=` |
| `type = 3` | assignment | next token is `=` |
| `print "a", b` | `print_statement` | `print` followed by a token that starts an expression and is not `(` |
| `print (x)` | `call` | next token is `(` |
| `print >>f, x` | `print_statement` with a `chevron` | next token is `>>` |
| `x.print(match.group, exec)` | `identifier` each time | previous token is `.`, or the word is not first in its statement |

### 5.2 Targets and patterns

The same bytes become different kinds on the two sides of `=`:

```text
a, b = c, d         left: pattern_list           right: expression_list
(a, b) = [c, d]     left: tuple_pattern          right: list
*a, b = c           left: pattern_list(list_splat_pattern, identifier)
for a, b in c:      left: pattern_list
def f(*args, **kw)  list_splat_pattern, dictionary_splat_pattern
f(*b, **c)          list_splat, dictionary_splat
```

**The target region** of a statement is:
- everything before its last top-level `=`, for an assignment;
- or the part between `for` and its `in`;
- or the target after `as` in `with` and `except` (as `as_pattern_target`).

The region passes inward only through tuple and list displays, not through trailers:

- in `a[i] = 1`, `i` is an expression;
- in `(a, (b, c)) = x`, both tuples are `tuple_pattern`.

So "in the target" for a bracket = (its parent is in the target) and (it is a display, not a call
or subscript). That is the per-depth vector scan above.

**The trap.** "Top-level `=`" must exclude a lambda's defaults. In `f = lambda x=1: x`, the second
`=` is at bracket depth 0 but inside `lambda … :`. Stage 4 therefore pairs each `lambda` with its
`:` as if they were brackets. Nested lambdas nest: `lambda: lambda: 1` opens twice and closes
twice. The region between a pair counts as one level deeper for every "top-level" test.

**Supertypes differ by context too.** An `identifier` in a target is reported under `pattern`,
and in an expression under `expression` and `primary_expression`. The region fact selects the
memberships in the node's code (decision 0025).

### 5.3 What a bracket pair becomes

| Bytes | Context | Node |
|---|---|---|
| `( … )` | after an operand | `argument_list` (a trailer) |
| `( … )` | after an operand, holding only `x for …` | `generator_expression`, as the call's `arguments:` field, with no `argument_list` |
| `( … )` | no comma, no `for`, one item | `parenthesized_expression` |
| `( … )` | a top-level comma, or empty | `tuple` (`tuple_pattern` in a target) |
| `( … )` | a top-level `for` | `generator_expression` |
| `( … )` | after `with`, holding `as` | `with_clause` owns the parentheses |
| `[ … ]` | after an operand | `subscript` trailer: the value and every top-level item get `subscript:` fields, slices inside |
| `[ … ]` | otherwise | `list`, `list_comprehension`, or `list_pattern` in a target |
| `{ … }` | a top-level `:` or `**` | `dictionary`, or `dictionary_comprehension` with `for` |
| `{ … }` | otherwise | `set` or `set_comprehension` |
| `{ … }` | in an f-string | `interpolation` |
| `[ … ]`, `{ … }` | in a `case` pattern | `list_pattern`, `dict_pattern` |

Each row is a table lookup on a few bits per bracket pair:
- "after an operand", a neighbour fact;
- "top-level comma", "top-level `for`", "top-level `:`" and "empty", segment facts;
- "in a target", "in a case pattern" and "in an f-string", enclosing facts.

Only the outermost of these nodes at each bracket is the bracket's. Postfix wrappers start earlier
(§6.2).

### 5.4 What a token becomes

| Token | Meanings | Deciding facts |
|---|---|---|
| `:` | block header, `pair`, `slice`, `lambda`, annotation, format spec, `case` | the enclosing bracket's kind; the lambda pairing; the line's head keyword; stage 1's mode |
| `*` | multiply, `list_splat`, `list_splat_pattern`, keyword-only marker, `wildcard_import`, `except*`, `splat_pattern` | whether the previous token ends an operand; the region; the statement head |
| `-` `+` | unary or binary | whether the previous token ends an operand |
| `in` | comparison, or the `for … in` keyword | the first `in` after a `for` at the same depth is the keyword: a segmented "for pending" flag |
| `not` `in`, `is` `not` | one comparison operator node with two token children | adjacent-token facts |
| `if` `else` | `conditional_expression`, `if_clause`, statement keyword | after a `for_in_clause` in the same bracket, `if` starts an `if_clause`; otherwise a ternary `if`, and each `else` pairs with the nearest unpaired ternary `if` at its depth, as brackets do |
| `=` | assignment, `keyword_argument`, `default_parameter`, `typed_default_parameter`, `type_alias_statement` | the enclosing bracket's kind (call, parameters); the lambda region; the statement head |
| `as` | `as_pattern` in `with` and `except`, `aliased_import`, `case` alias | the statement head |
| an identifier | `identifier`, or wrapped in `dotted_name` (imports, case value patterns), or wrapped in `type` (annotations) | the region |

Two-token operators are an unusual tape shape: an anonymous node that has children.

```text
x not in y is not z
  comparison_operator [0..19]
    identifier [0..1]
    operators: 'not in' [2..8]     anonymous, two children
      'not' [2..5]
      'in' [6..8]
    identifier [9..10]
    operators: 'is not' [11..17]
      'is' [11..13]
      'not' [14..17]
    identifier [18..19]
```

### 5.5 Strings

- **Adjacent strings** form a `concatenated_string`, which starts at the first string's start and
  outranks it. A run is "a string token whose next token is a string token", a segmented scan over
  the token stream. 9.75% of files have one.
- **String internals.**

  ```text
  f"a{b!r:>{w}}c{{d}}"
    string_start "f\""
    string_content "a"
    interpolation [7..17]
      '{'
      expression: identifier "b"
      type_conversion "!r"               one named leaf for two bytes
      format_specifier [11..16]
        ':'
        format_expression [13..16]       an interpolation inside the format spec
          '{'  expression: identifier "w"  '}'
      '}'
    string_content [17..23]              not split at the escapes:
      escape_interpolation "{{"          they are its children
      escape_interpolation "}}"
    string_end "\""
  ```

- **Escape sequences** are children of `string_content` too (`"a\nb"`). This differs from JSON,
  where `string_content` is split at each escape. Raw strings have no `escape_sequence` nodes.
  An escape's length is a table lookup on its letter: `\x` takes 4 bytes, `\u` 6 and `\U` 10;
  `\N{…}` runs to the next `}`.

### 5.6 Match patterns

A `case` line has its own grammar of wrappers, chosen by the "in a case pattern" region:

```text
case [1, *r] | {"k": v} if v: pass
  case_pattern
    union_pattern
      list_pattern  '[' case_pattern(integer) ',' case_pattern(splat_pattern) ']'
      '|'
      dict_pattern  '{' key: string ':' value: case_pattern(dotted_name(identifier)) '}'
  guard: if_clause
```

Only 0.35% of files have a `match` statement. Patterns can go to tree-sitter until this part is
written.

## 6. Stage 4: expressions as binding powers

### 6.1 The operator tree

Within one bracket segment, each operator token gets a left and a right binding power from the
grammar's precedence table. This is a Pratt parser's table:

| Operators | Binding |
|---|---|
| `lambda` | prefix, lowest |
| `if` … `else` | ternary, right-associative |
| `or`, `and` | left |
| `not` | prefix |
| `<` `>` `==` `!=` `<=` `>=` `in` `not in` `is` `is not` | one n-ary chain |
| `|`, `^`, `&`, `<<` `>>`, `+` `-`, `*` `/` `//` `%` `@` | left |
| `-` `+` `~` | prefix |
| `**` | right; binds tighter than a prefix on its left, but takes a prefix on its right |
| `await` | prefix |

A prefix operator has an infinite left binding power: it never takes a left operand. An operator
at token `i` takes as its left operand everything back to the nearest operator `j` with
`rbp(j) < lbp(i)`, and as its right operand everything up to the nearest `k` with
`lbp(k) < rbp(i)`. Both are nearest-smaller-value queries over about 20 levels, the vector-max scan
of 01 §3. Ties for associativity are broken by setting `rbp = lbp ± 1`.

A real example, where `**` takes a prefix on its right while a prefix on its left takes `**`:

```text
-a ** -b * c
  binary_operator [0..12]           '*': the loosest; spans everything
    left: unary_operator [0..8]     prefix '-': lbp ∞, ends at '*' (looser than unary)
      argument: binary_operator [1..8]   '**' binds tighter than the '-' before it
        left: identifier "a"
        right: unary_operator [6..8]     a prefix after '**' (lbp ∞)
    right: identifier "c"
```

**Comparison chains** are one node, as `a < b < c == d` shows:

```text
comparison_operator [0..14]   a  '<'  b  '<'  c  '=='  d
```

Only the first comparison of a run emits a node. A comparison is first when the nearest earlier
operator at least as loose is not a comparison, which is the same query. The others become
`operators:` children of that node.

**Boolean operators** are binary and nest left:

```text
not a == b and c or d
  boolean_operator [0..21]          'or'
    left: boolean_operator [0..16]  'and'
      left: not_operator [0..10]    'not' takes the whole comparison
        argument: comparison_operator [4..10]
```

### 6.2 Postfix chains and many nodes at one byte

Trailers (`.name`, `( … )`, `[ … ]`) bind tighter than any operator. An atom and its trailers form
one operand for §6.1. Every trailer's node starts at the atom's start:

```text
a.b(c)[d].e + 1     seven nodes start at byte 0, outermost first:
  expression_statement [0..15]   statement layer
  binary_operator [0..15]        operator layer: '+'
  attribute [0..11]              postfix layer: '.e'      the last trailer is outermost
  subscript [0..9]               postfix layer: '[d]'
  call [0..6]                    postfix layer: '(c)'
  attribute [0..3]               postfix layer: '.b'
  identifier [0..1]              atom
```

**Ranks.** A node's rank (01 §2) is its position among the nodes that start at its byte. They come
in fixed layers, outermost first:

1. statement wrappers;
2. operator nodes;
3. postfix nodes;
4. the atom (a bracket's node, a string run's wrapper, then the string);
5. the atom's first token.

Each layer counts its own nodes at each start. The rank is the sum of the outer layers' counts
there, plus the node's rank within its layer.

- **Postfix nodes:** the rank within the layer is the number of trailers after it in the chain,
  a reversed segmented count.
- **Operator nodes:** the operators that share a left boundary nest so that the later one is
  outer. Its left operand contains the earlier one. So the rank within the layer is the number of
  later operators with the same left boundary. That is the `rank(key)` operation added in §9: on
  the CPU, one branch-free counter per key; on Metal, one pass of a stable counting sort.
- **Long left-associative chains** (`s + t + u + …`) put up to 75 nodes at one byte in the
  corpus. The formula has no bound, so no fixed slot count is needed.

### 6.3 Fields and memberships without a tree

Each node's field and memberships are decided by its parent's rule (left, right, operator,
function, arguments, key, value, and so on). Here the parent knows where each child is:

- **its first child is `ord + 1`;**
- **a child starting at a different byte `s` is `C[s]`**, the first node at `s`. No node at `s`
  can enclose the child without overlapping the parent.

So each parent scatters its children's fields to `ord + 1` and to `C[s]` for its other operands.
The memberships come from the grammar slot the child fills (`binary_operator.left` holds a
`primary_expression`). This is more exact than decision 0025's table keyed by (parent kind, field,
kind), which missed 8–152 nodes per corpus. The differential test confirms it.

## 7. Stage 4: comments

Comments are extras, attached by tree-sitter wherever its LR state is when it reads them.
Measured cases:

```text
x = 1  # c                expression_statement [0..5]; comment [7..10] in module
                            → a trailing comment is outside its statement

if x:                     if_statement [0..37]
    # lead                  comment [10..16]        before the block: child of if_statement
    y                       block [21..37]          the block's extent grows to include …
    # trail-in                comment [27..37]      … a trailing comment as deep as the block
  # trail-mid             comment [40..51] in module  shallower: outside
# trail-out               comment [52..63] in module
z

if x:                     block [10..21]
    a
# c                         comment [12..15]  inside the block: the next code line
    b                                         is still in the block

if a:                     if_statement [0..47]
    b                       block [10..11]
# between                   comment [12..21]  child of if_statement: the block
elif c: …                                     closed, but the elif continues it
```

**The rule** follows `scanner.c`:
- A block's extent is first its statements. It is then extended over the comment lines that
  follow its last statement, but only when the next logical line closes it.
- The extension covers those comment lines while the running minimum of their widths is at least
  the block's width: a segmented prefix-min scan over each run of comment lines.
- Compound statements end where their last clause's block ends, so the extension propagates
  outward.

**Then nothing more is needed.** A comment's preorder position follows from its start byte, and the
nodes whose `[start, end)` contain it count it in their `size`. With block extents extended
correctly, `x = 1  # c` falls outside the statement, `# lead` falls inside the `if_statement` but
before its block, and `# between` falls inside the `if_statement`, which the `elif` extends. Every
case above comes out of 01 §2's formulas. The comment rule's general form (runs with mixed widths
over several closing blocks) is the first thing the differential test should check.

## 8. Errors and why a fast-path tape is right

**Error bits.** Every stage sets error bits, and one OR per file routes it to tree-sitter:

| Stage | Errors |
|---|---|
| 1 | an unterminated string; an f-string form the lexer leaves to fallback (`07-viability.md` §1) |
| 2 | an invalid byte or number |
| 3 | mismatched brackets; a bad dedent |
| 4 | a token pair the binding-power table does not relate (two operands in a row, as in `a b`); a statement no rule recognises; an operand outside its slot's supertype (`a == not b`, where a comparison operand must be a `primary_expression`) |

**A final check.**
- Each node's sequence of child codes must match its grammar rule. Tree-sitter's rules are
  regular expressions over symbols, so each one compiles to a small automaton.
- That automaton runs as a segmented scan over each parent's children, which are reached as
  §6.3 says.
- A tree that passes is a parse of the file under the grammar, so the fast path never emits a tape
  for a file the grammar rejects.

**What this does not settle.** When the grammar allows more than one tree, the rules must pick
tree-sitter's. Tree-sitter's conflicts and dynamic precedence decide that, and only the
differential test proves the rules agree. A wrong choice is a wrong tape, not a fallback. That is
why the test compares every image byte for byte before any rule ships.

## 9. Operations Python adds to 01 §4

| Operation | Used for | CPU | Metal |
|---|---|---|---|
| `rank(key)` | rank among earlier elements with the same key (§6.2) | a counter per key | one stable counting-sort pass |
| segmented `scan(min)` | comment-run widths (§7) | as `scan(+)` | as `scan(+)` |
| per-depth vector scan | enclosing facts (§5) | a `last[depth]` vector | a vector-state scan |
| bounded hash lookup | keywords (§3) | 8-byte compare | table in threadgroup memory |

## 10. Bounds, and what to measure first

The bounds that make this branch-free, from the corpus:

| Bound | Corpus maximum | Lanes | If exceeded |
|---|---:|---:|---|
| bracket depth | 14 | 16 | error bit |
| block nesting | 13 | 16 | error bit |
| string nesting in interpolations | 2 (1 file) | 1 | error bit |
| bracket depth inside an interpolation | not measured | 2 | error bit |
| precedence levels | about 20 | 32 | fixed by the grammar |
| nodes at one start byte | 75 | none needed | unbounded by construction |

Not yet measured: the share of bytes that are mode bytes (stage 1's work), and how many files use
features in the order they would be implemented. Planned order:

1. statements and expressions;
2. strings and f-strings;
3. comments;
4. clause chains and decorators;
5. match patterns.

Coverage is the share of files whose image matches byte for byte. It should be reported after each
of these steps.
