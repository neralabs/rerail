# 0226: The lean SQL parser writes its tape as it parses, with no arena

Status: decided (user, 2026-11-14: "yes, take that on next", on removing the arena's second pass).

## Intro

The lean SQL parser (0214) built each statement in an arena: every node and token, with its
children, in parse order. A second pass (`emit`) then walked the arena to write the tape. The other
lean parsers (0163, 0193, 0195, 0196, 0198) have no arena: they write each node to the tape
(`rerail_tape_format::Writer`) in preorder as they read it. After 0219 and the tokens' fates by
kind, the second pass is the largest part of SQL's cost that the other parsers do not pay: about a
sixth of the parse, and the arena's nodes and child lists before it.

This decision removes the arena. The parser writes the tape directly, as the others do.

```
before: tokens -> arena (nodes, child lists, all tokens) -> emit: fates, extras -> Writer
now:    tokens -> Writer, each node at its place as it is read
               punctuation held until the node that takes it closes, then written or left out
```

## Prior work consulted

- **0214 (lean SQL), 0195 (lean Java).** **Agrees:** Java's parser opens a node when it knows its
  kind (`open`), wraps a node found after its first child by `Writer::insert` (`wrap`), and writes
  a comment into the node open when the next token or node is read (`flush`). SQL keeps those
  three.
- **The engine's implied-token automata, and `kinds::FATE` (0214).** **Followed:** the tape leaves
  a token out exactly where the engine's walk does. Only the 31 anonymous kinds (punctuation) can
  be left out: keywords are named and always stored.
- **"Tapes never generate implied tokens."** **Followed:** a token the tape leaves out is never
  written. A held token is written only once its fate is known.

## Decision

- **A node is written when its kind is known.** `open(kind)` pushes it at the next token's start;
  `close` gives its end. Where the kind is found only after a child (an operator after its left
  operand, a cast after its expression), the node is put before that child (`wrap`, the writer's
  insert), and that happens before the operator's own tokens are read.
- **A frame for each node being read.** The parser keeps a stack of the nodes open, and of the
  marks where a node may still be put (a `mark` is a frame with no node yet). A frame knows its
  kind, where it starts in the writer, and the punctuation it holds.
- **Punctuation by its parent's fate.** A punctuation token read where the innermost frame is a
  node whose kind decides that token (`FATE`) is written or left out at once. Elsewhere it is held,
  with its place: in a mark (its node is not known yet) or in a kind whose walk decides it. When
  the frame closes, the held tokens are decided, by the kind or by the walk over the node's
  children as written, and the stored ones written at their places (`Writer::insert_leaf`).
- **Comments** go into the innermost open node when the next node or token is written, so a
  comment before a node's first child is its parent's, as the arena's pass placed them.
- **An empty node** is written at 0..0, as the arena's pass wrote it.
- **Widths:** a tape whose values do not fit the guessed widths is parsed again at the wider ones,
  as the other lean parsers do.
- **A parse that does not hold** writes only after its save point (a transaction tried before
  its statement, `extract`'s `unit from` form). Nodes after it, frames above it and the innermost
  frame's state are all it changes, so `restore` truncates them.
- **An invariant that would loop is a hard check.** A node left open has no size, so a walk over
  its parent's children would not end. `end_marks` asserts in every build, not only in debug.

## Results

- **Oracles unchanged.** Corpus: 129 of 224 clean files the same, none differ, 95 refused. Hand
  sets: 12 of 13, 21 of 21, 24 of 25 (1 refused), 32 of 33 (1 refused), as before.
- **Cost** (instructions retired over the 129 corpus files, the best of five passes, a tape key):
  5,923 before, 5,545 now (−6.4%). Cycles under heavy machine load (load average 70–90): the best
  of eight runs 1,038 before, 1,051 now; the typical run about 1,400 before, 1,300 now.
- **Where the arena's cost went.** The second pass and its child lists are gone. What the writer
  pays instead is inserts: a node whose kind is known only after its first child (a binary
  operator, an invocation after its name) is put before that child, and a held token that is
  stored is put at its place. About 6,100 inserts a pass over the corpus, each shifting 5 nodes on
  average. 81 nodes whose kind is known before their first token are opened up front instead of
  put afterwards, and so is the statement (its transaction form is tried before it is opened).
- **Found along the way, fixed:**
  - per-byte loops: the NUL check (now a stage 1 mask), meta-commands and their send form (stage
    1's blank and line masks, then a bounded SWAR search, `rerail_simd8::swar::first_of2`), a
    dollar-quote's tag and its close (stage 1's word run, then SWAR), the operator's run (8 bytes a
    step by a class table, with a test against the byte-at-a-time form);
  - `last_child`, a walk over the open node's children for each expression: the expression's node
    is at its mark;
  - an unused parameter, unused bindings, an empty branch.

## Open

- The inserts for a binary operator's node, which Java's lean parser also pays. Opening the node
  before its left operand would need the operator known first: a look past the operand.
- `Writer::finish_columns` shrinks the size and tail columns (a copy each), in every lean parser.
