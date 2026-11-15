# 0234: Markdown's inline texts are read by a lean parser, a small GLR over the grammar's conflicts

Status: decided (user, 2026-11-14: "please do both of these", on lean parsers for Markdown's inline
grammar and for Vue).

## Intro

A Markdown file is two grammars. tree-sitter-md's block grammar reads the file's structure; each
paragraph's `inline` node (and each table cell) is a guest text that tree-sitter-md-inline reads:
emphasis, links, images, code spans, HTML tags. The block grammar has had a lean parser since 0237.
Every inline text still went to tree-sitter. In a README, that is most of the file's bytes.

This decision adds the lean parser for the inline grammar (`rerail-markdown-inline-lean`). It is
registered at rank 0 for `GrammarDialect::MarkdownInline`, so the guests a Markdown file embeds are
read by it, and a text it refuses goes to tree-sitter. There is one behavior, as for every lean
parser: tree-sitter's tape, or a refusal.

The inline grammar is unlike the others because it is ambiguous by design. Each of these has more
than one reading, and tree-sitter keeps a parse version for each one:

```
[a] (b)      a link `[a]`, then `(` `)` as text   -- or `[` `]` `(` `)` all as text
*a           a lone delimiter (hidden)            -- or the start of emphasis, never closed
![x][y]      one image with a label                -- or an image, then a link
<b c=d>      an HTML tag                           -- or `<` `=` `>` as text
```

A version is dropped when it meets a token it cannot take. Of the versions that reach the end,
tree-sitter keeps the one of the highest *dynamic precedence*: the sum of the grammar's
`prec.dynamic` over the tree's nodes.

## Prior work consulted

- **0237 (lean Markdown block parser).** **Followed:** its frame: stage 1 masks
  (`block.rs`), the external scanner ported (`scan.rs`), keys by name, mapped to the grammar's ids
  in the engine (`tape::markdown_inline_lean`). Its `inline_texts` test writes the inline texts
  the oracle reads, cut as the engine cuts them (`guests::markdown_inline_text`).
- **0214, 0226 (lean SQL).** **Followed:** the engine's token automata (`automata.tsv`,
  `token_automata_text`) decide which tokens a node's tape leaves out, by the same walk.
  **Departs:** 0226 writes each node as it is read. Here the parse ends only when the versions
  do, so each version keeps its events (a node's start, end, and leaves) and the winner's are
  written once. The same rule holds as in 0226: a token is written only once its fate is known;
  none is written and then removed.
- **tree-sitter's GLR parser (`parser.c`, 0.27.1, read for this).** **Followed:** versions advance
  token by token; two that reach the same state merge; the survivor is the one of the higher
  dynamic precedence (`ts_parser__select_tree`). tree-sitter keeps at most 6 versions
  (`MAX_VERSION_COUNT`).
- **"No per-byte loops."** **Followed:** words, runs, blanks, digits, backticks, dollars and NULs are found
  by stage 1's masks. Autolinks, entity names and attribute names are found by `class_end`, a
  new 8-bytes-a-step class scan in `rerail_simd8::swar`, next to a new `run_end`.

## Decision

- **A version is a stack of frames.** A frame is a construct open around the parse: a sequence of
  elements (`Seq`, with the delimiter whose lone opener it does not take, and whether it is in a
  link's text), emphasis, strong emphasis, a link's text, a label, an inline link's tail, an
  image's description, a code span, an HTML tag. A token in a frame gives zero, one or several
  successor versions. Several means a conflict the grammar declares, and each successor is one
  reading.
- **The score is the grammar's dynamic precedence.** Emphasis and strikethrough score 1. Strong
  emphasis scores 2 plus its inner emphasis's 1. A shortcut link scores 10, a collapsed link 20, a
  full reference link 30, and an inline link 20, plus 10 with a destination. A shortcut image scores
  30, any other image 40, plus 10 with a destination. An HTML tag scores 100. Two versions in the
  same state at the same place merge, and the higher score's tree is kept.
- **A token is lexed in the version's own state.** The scanner is called first, with the
  external tokens the state takes. Otherwise the lexer takes the longest match among the state's
  tokens. Versions forked by one token lex the next token together, with the union of their
  valid sets, as tree-sitter lexes in the state before the fork (`V::group`).
- **Refused, rather than guessed:**
  - *A tie.* tree-sitter breaks a tie between two different trees by comparing its internal trees,
    including hidden nodes and its symbol numbers (`ts_subtree_compare`). This parser does not
    keep those, so a text whose winner tied is refused. An example is `![c][d]`: one image with a
    label, or an image and then a link, both scoring 40.
  - *More than 6 versions.* Past 6, tree-sitter drops versions by its own ordering. Its count is
    lower than this parser's, since one tree-sitter stack holds readings that shift the same
    token. So the text is refused when this parser's count passes 6.
  - *A delimiter tree-sitter may lex again.* On a shift/reduce conflict, tree-sitter's reduce
    branch becomes a new version, and that version lexes again in the state the reduction leaves.
    In that state the last token's white space no longer counts, so an emphasis delimiter the
    scanner opened may close instead. In `x __b c __d`, the emphasis closes at `__d`. Which
    readings reduce apart depends on how tree-sitter's stacks merged. So a text is refused
    wherever some version's emphasis would close on such a lexing.
  - *Not read yet:* HTML comments, processing instructions, declarations and CDATA (`<!--`,
    `<?`, `<!X`, `<![CDATA[`), wiki links, delimiter runs of 255 or more, and the empty text.

## Results

Oracle (`oracle`, against tree-sitter's tapes of the inline texts of 1,200 Markdown files from the
sample corpus):

| Texts | Same | Differ | Refused |
|---|---|---|---|
| 36,787 (800 files) | 36,398 | 0 | 389 (1.1%) |
| 21,639 (400 files) | 21,528 | 0 | 111 (0.5%) |

Of the first set's refusals, 353 are the version cap. On 337 of those, tree-sitter's own count
passes 6 (measured with tree-sitter's logger over the same generated parser). With the cap lifted,
the refusals are 36 and the differences stay 0: ties, the re-lex rule, and constructs not read.

Cost (`markdown_inline_parse_costs`, the 36,398 texts both read, 3.7 MB, 421,150 nodes; best of
three passes, load average about 100):

| Step | Instructions a node | Cycles a node |
|---|---|---|
| lean: the tape | 4,745 | 998 |
| tree-sitter: parse | 21,637 | 4,833 |
| tree-sitter: the tape | 25,246 | 5,831 |

The tape costs 5.3 times fewer instructions and 5.8 times fewer cycles than tree-sitter's. Per
node it is still far above the other lean parsers (about 800), for two reasons. An inline tape has
few nodes for its bytes, because words and white space are hidden. And each version clones its
frame stack at a fork.

## Open

- **The version cap.** Counting versions as tree-sitter does, one stack for the readings that
  shift the same token, would read most of the 353 texts. That needs to know which readings
  tree-sitter's states hold together.
- **The re-lex rule is broad.** It also refuses common prose such as `a **b *c* d**`, where
  tree-sitter's answer is the score's. A narrower rule would need tree-sitter's merges.
- **Ties:** tree-sitter's structural compare, for the few shapes that tie (images with labels).
- **HTML comments and the other HTML constructs.**
- **The versions' cost:** frames in a small inline stack, not a `Vec` cloned per fork, and the
  events' arena reused across texts.
