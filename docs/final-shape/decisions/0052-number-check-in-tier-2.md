# 0052: A token's syntax is checked in tier 2, as simdjson's stage 2 parses a number

Status: accepted (user, 2026-11-07: "keep it as close to simdjson as possible").

## Intro

simdjson's stage 1 does not look inside a number. It finds the word, and stage 2's
`parse_number` reads its bytes in a few steps: a sign, the integer, a fraction, an exponent.
JSON's number syntax had gone into tier 1 as nine error masks. They took tier 1 from 41
operations to 95, its loop from 178 instructions to 450, and its carried state from 4 bits to
15. So the check moves to tier 2, where simdjson has it.

## Prior work consulted

- **0049 (two tiers):** validators per token, in tier 2. **Agrees;** this restores it, without
  a built-in validator: the check is the language's grammar, compiled.
- **0051 (the tree as a grammar):** a number's syntax as tier-1 masks. **Superseded** on that
  point.
- **0047 (simdjson's shape, ratchets):** the ratchets fall to 37 operations and 174 loop
  instructions, below where they were before the masks.

## Decision

1. **`check KIND = G`** in Bitlex: `G` is a grammar over the token's bytes, in the tree
   section's notation, its names byte classes and its literals bytes. A token that fails it
   sends the file to tree-sitter.
2. **The compiler makes a byte state machine** from it (Glushkov's positions, subset
   construction, merged states): for JSON's number, 8 states, the steps of `parse_number`.
3. **The compiler turns that machine into straight-line operations** over the token's 64-byte
   window: a `fill` per run of a self-loop (the integer, the fraction, the exponent), a shift
   per arm, and logic. It is a block program (`CheckRow`): printed by `rir`, checked by
   `validate`, in the digest. Tier 2's arm for the token runs it, lowered to the IR with the
   rest of tier 2. No Rust validator, no call.
4. **JSON's words are one token,** as stage 1 leaves them: `true`, `false` and `null` retype
   it, and any other word is a number, which the check refuses if it is not one.
5. **No per-byte loop, blocked by the system** (user, 2026-11-07: "ANY PER BYTE LOOP IS
   UNACCEPTABLE, and they should be caught and blocked by the system"):
   - a machine with a cycle through two or more states would need one: the compiler refuses it;
   - a check is a block program, whose operations have no loop;
   - a token longer than the window (63 bytes) goes to tree-sitter, not to a loop;
   - tier 2's IR has one loop, over the stream's entries; its verifier refuses any other.
