# 0051: The tree section is a grammar; tier 2 is its state machine

Status: accepted (user, 2026-11-07: "yes, please make that change, then complete all of the
updates to all of the artifacts, designs and plans that it entails"). The design is
`docs/parser-generators/18-tree-as-grammar.md`.

## Intro

Tier 2 is simdjson's stage 2: a state machine over the token stream, which branches on
something concrete and goes straight through. Its states are the grammar. The old tree
section said where nodes start and end, not what may follow what, so it could give neither
the states nor the grammar check. It now states each node's content, as tree-sitter's
`grammar.js` does.

## Prior work consulted

- **0049 (two tiers):** tier 2 per token. **Agrees;** its tier 2 is now generated from a
  grammar, not rows. Its validators "lower to good code": a number's syntax is tier-1 masks.
- **0050 (tier 2 on Metal):** a dependency analysis of the rows' conditions, backward keys, an
  8-byte stack effect. **Superseded in part:** the rows' conditions are gone, every rule reads
  forward, and the stack carry is the depth (closes matched by nearest smaller value,
  `01-branch-free-design.md` §3). Its plan stores, barriers and verifier stay. Metal comes
  after the CPU.
- **Lessons ledger, ast-index-spike indexes:** no grammar or parser lessons; #28 (CPU and GPU
  time only) holds for the cost ratchet.

## Decision

1. **The tree section is a grammar:** `node K [nested] = E` and `choice N [supertype S] = …`,
   with fields as `f: X`. The rows keep only the tokens.
2. **The compiler builds one DFA per level** (the root and each `nested` node), inlining the
   rest, with open, leaf and close actions on each transition, calls for nested nodes, and
   refusals for everything else. LL(1) per level, or the compiler refuses it.
3. **Tier 2 on the CPU is that machine** over tier 1's one extracted stream, with a memory
   stack of return states and open ords, as simdjson's. No search, no count, no call.
4. **The tape is written directly,** at speculated widths, written once more at most.
5. **Metal later,** from the same automaton (doc 18 §5).
