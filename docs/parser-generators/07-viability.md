# Parser generators: viability across the whole output

Status: measurement, 2026-11-06. It covers the lexers in `lexers/` and the tree stages of
`02-python.md`. The lexers now run, in the reference evaluator `tools/bitlex`, and §1 and §3 give
what running them measured.

## Intro

How much of what tree-sitter produces can the branch-free path produce, and how many files must
still go to tree-sitter? This document answers that in two parts.

1. **Which files fall back.** Each lexer has explicit error conditions. They were measured on the
   corpora by looking for the same conditions in tree-sitter's trees.
2. **Which nodes are covered.** Every node kind tree-sitter emitted on the corpora is listed with
   its count, and mapped to the lexer rule or tree stage that produces it.

The lexers have been run on all three corpora, and their tokens compared with tree-sitter's leaves
(`03-bitlex.md` §5). Whether the tree stages produce the same nodes is not measured yet: they have
not been written.

## 1. Files that fall back

Files tree-sitter itself parses with errors always go to tree-sitter: 46 of 13,088 data-layer
Python files, 61 of 2,054 CPython files, and 1 of 14,669 data-layer JSON files. The rates below are
among the rest.

The first lexers used machines; the lexers as run have none (`08-without-automata.md`). The
counts per condition come from tree-sitter's trees. The last two rows are what running the lexers
gave.

| condition | data-layer Python | CPython 3.15 | first lexers | lexers as run |
|---|---:|---:|---|---|
| bracket depth 3–4 inside an interpolation | 38 | 7 | fallback | handled: a count, no bound |
| f-string inside an interpolation | 17 | 5 | fallback | handled: instance `f2` |
| comment inside an interpolation | 0 | 2 | fallback | handled |
| non-ASCII identifier | 0 | 6 | fallback | handled: `ucode` |
| byte-order mark, U+FEFF / U+2060 / U+200B | 0 | 1 | fallback | handled: `uspace` |
| f-string reusing its own quote (PEP 701) | 0 | 5 | fallback | handled: outer `rounds` |
| newline in a one-line f-string's interpolation (PEP 701) | 0 | 2 | fallback | handled |
| backslash before `{{` in f-string text | 0 | 1 | not modelled | handled: the `'\'` leaf |
| triple-quoted string inside an interpolation | 0 | 1 | fallback | fallback |
| number run into a word (`1or`) | 0 | 1 | fallback | fallback |
| a quote run whose grouping does not settle | 0 | 1 | | fallback |
| f-string nested three deep | 0 | 0 | | fallback |
| **files falling back, measured** | **0** | **3** | | |
| **share** | **0%** | **0.15%** | | |

All other files that tree-sitter parses get exactly tree-sitter's tokens: 13,042 of 13,042 in the
data layer and 1,990 of 1,993 in CPython.

These conditions occur often but need no fallback:

| condition | data-layer Python | CPython |
|---|---:|---:|
| string content across lines | 4,195 files (32%) | 1,046 (52%) |
| backslash-newline in a string | 45 | 141 |
| triple-quoted f-string | 76 | 87 |

**JSON:** 0 of 14,668 files fall back, and all 14,668 get exactly tree-sitter's tokens. No
data-layer JSON file has a comment. The comment rules were checked against tree-sitter-json on
hand-written cases (§2), with the same differential driver.

**An earlier count was wrong.** It reported "non-ASCII whitespace between tokens" in 42 CPython
files. Those were gaps between the escape children of a `string_content`, inside strings. With
whole strings treated as tokens, the count is one file: a byte-order mark before a comment.

## 2. JSON: every node kind

The data-layer JSON corpus has 169.2 M nodes. 72.8% are leaves.

| kind | count | produced by |
|---|---:|---|
| `"` | 47.2 M | `lex` events (or `toggle(q)` when the file has no comment) |
| `string_content` | 24.1 M | mask `content` |
| `,` `:` `[` `]` `{` `}` | 46.8 M | code mask and class |
| `number` | 3.0 M | word run; 8 mask rules for the pattern |
| `false` `null` `true` | 2.5 M | `keywords … required` |
| `escape_sequence` | 0.65 M | always two bytes |
| `comment` | 0 | `lex` machine |
| `string` `pair` `array` `object` `document` | 45.9 M | `01-branch-free-design.md` §5 |

Comment facts, each checked on tree-sitter-json 0.24.8:

- `//x\r\n` is the comment `//x\r`;
- `/*/ */` and `/**/` are comments;
- `/*a*//` is a comment followed by an error;
- a comment's parent is the lowest common ancestor of the tokens around it, or `document` at
  either end. Between a key and `:`, it lands in `pair`; after a value and before `,`, it lands
  in the `object` or `array`.

## 3. Python: every leaf kind

On the data-layer Python, 66.3% of 33.5 M nodes are leaves. Every leaf kind on either corpus is
listed here.

| leaf kinds | data-layer count | produced by (`lexers/python.bitlex`) |
|---|---:|---|
| `identifier` | 7.29 M | words, with `ucode` for non-ASCII |
| 44 operator and punctuation kinds, from `,` (1.89 M) to `<<=` (1) | 10.0 M | `munch op` |
| `string_start`, `string_end` | 1.79 M | `reflect` scans per line, and `f1`/`f2` |
| `string_content`, `escape_sequence`, `escape_interpolation` | 0.91 M | masks over `s_text` |
| `type_conversion` | 4,063 | `!` + letter in an interpolation |
| 32 hard keywords, `def` (179 K) to `global` (152) | 1.63 M | `keywords` |
| soft keywords: `type`, `case`, `match`, `__future__`, `_` | 14,748 | `keywords … soft`, decided with the tree |
| `none`, `true`, `false` | 0.33 M | `keywords` |
| `integer`, `float` | 0.17 M | number masks; 7 mask rules for the patterns |
| `ellipsis` | 17,377 | `munch` |
| `comment` | 79,758 | a fill from `#` (and `cmt` inside interpolations) |
| `line_continuation` | 0 here; 521 in CPython | mask `lcont` |
| `'\'` inside `string_content` | 0 here; 9 in CPython | a backslash that starts no escape |
| `module` as a leaf: an empty file | 52 | the empty tape |

`print`, `exec`, `<>` and `chevron` do not occur in either corpus. The lexer has them for
Python 2 code.

Two non-leaf kinds are made of tokens:

- **`'is not'` and `'not in'`** (46,027): one anonymous node over two keyword leaves. Made in the
  tree stage.
- **`string_content` with escape children** (13,569): the content token gets its escapes as
  children. Made in the tree stage.

## 4. Python: inner nodes by stage

The 103 inner kinds on the data-layer corpus, grouped by the stage of `02-python.md` that makes
them. Shares are of all nodes.

| stage | kinds | data-layer nodes | share | CPython share |
|---|---:|---:|---:|---:|
| expressions: binding powers (§6) | 30 | 4.52 M | 13.5% | 13.6% |
| statements and blocks: lines and widths (§4) | 38 | 2.73 M | 8.1% | 9.2% |
| bracket pairs by depth (§5.3) | 17 | 2.10 M | 6.3% | 6.8% |
| strings: `string`, `interpolation`, `format_specifier` (§5.5) | 5 | 0.96 M | 2.9% | 3.9% |
| types (§5.2) | 5 | 0.95 M | 2.8% | 0.1% |
| patterns (§5.2, §5.6) | 8 (9 in CPython) | 27,098 | 0.1% | 0.2% |
| **all inner nodes** | **103** | **11.29 M** | **33.7%** | **33.8%** |

- **Expressions** is the largest group: `call` (1.29 M), `attribute` (1.00 M), `keyword_argument`,
  `assignment` and the operators.
- **Types** are frequent only in typed code: `type` (784 K) and `generic_type` (155 K) are
  annotations.

## 5. What is and is not known

| part | lexer | tree stage |
|---|---|---|
| JSON | complete, with comments | designed (`01` §5); comment parents follow from extents |
| Python tokens | every leaf kind seen: 96 in the data layer, 100 in CPython; tokens equal to tree-sitter's on every file that does not fall back | |
| Python layout, brackets, blocks | | designed (`02` §4) |
| Python expressions | | designed (`02` §6) |
| Python soft keywords, targets, patterns | lexed as words | designed in outline (`02` §5) |
| Python comment placement | lexed | rule observed from `scanner.c` (`02` §7); to be tested |
| supertype memberships, aliases | | from the grammar, as the cursor reports them; to be tested |

The fallback rates above bound how many files can take the fast path: 100% of data-layer Python
and JSON without syntax errors, and 99.85% of CPython. Whether they *do* take it depends on the
tree stages matching tree-sitter, and only the differential test shows that.

The parts most likely to need fallback rules after testing:

- the 9 grammar conflicts (`00-problem.md` §5.4);
- `print` as a statement or a call;
- comment placement before a dedent.
