# 0053: Tier 2 is the transducer's own instructions

Status: accepted (user, 2026-11-07: "use the transducer pipeline for lowering, not a new code
generator. You may add instructions if necessary").

## Intro

Tier 2 is simdjson's stage 2: one loop over the token stream. At each token it switches on the
token's first byte, does a little straight-line work, and jumps to the next state. The
transducer already has a program of that kind: its instruction words, with `GOTO` and one
block per instruction, lowered by `native_ir` and Cranelift. So tier 2 is written in those
words, with new instructions for what the tree walker lacks: reading the token stream,
switching on a byte, and writing nodes. It is not a new IR and not a new code generator.

## Prior work consulted

- **Doc 12 (what is wanted):** one format, the transducer's backends. **Agrees.**
- **0051 (the tree as a grammar):** the states and arms are data, read by the compiler only.
  **Kept:** the Bitlex compiler lowers them to instruction words. Nothing executes the tables.
- **0052 (checks in tier 2):** a check is a block program the token's arm runs. **Kept:** the
  `CHECK` instruction runs it 64 bytes at a time.
- **Doc 17 §5b:** a second body in `native_ir` beside `BlockNative`. **Superseded:** the
  body is the Program's words, lowered like any others.

## Decision

1. **New instructions,** in the 0x50 range, 32 bits each as before:

   | word | does |
   |---|---|
   | `TOKEN` | takes the next token of the stream (its first byte); at its end, notes that |
   | `SPAN n`, `SPAN m=1` | the token's end: `n` bytes on (a one-byte token, a keyword), or its last byte, the stream's next |
   | `SWITCH t` | jumps by the token's byte at table `t`'s offset (the end of the stream has its own target); no target refuses |
   | `WORD s` | the token's text is string `s`, or the file is refused |
   | `CHECK c` | the token passes check `c` (decision 0052), or the file is refused |
   | `OPEN k` | opens a node of kind `k` at the token's start |
   | `CLOSE m` | closes the innermost open node at the end of the token before (`m` 0), or of this one (`m` 1) |
   | `LEAF k` | a leaf node of kind `k`, the token |
   | `NODE_FIELD f`, `NODE_SUPER s` | the last node's field, and a supertype of it |
   | `PUSH p` | pushes the state to return to |
   | `RETURN m` | pops it and jumps there: to take a token (`m` 0) or to switch on this one (`m` 1) |
   | `ACCEPT` | the end of the file: closes the root at the file's end |

   `STOP` refuses: the file goes to tree-sitter, and only when tree-sitter is the only one
   that can parse it. `GOTO` jumps.
2. **The switch tables** are a table of the Program (`switches`), as `strings` and `longs`
   are: printed by `rir`, checked by `validate`, in the digest.
3. **The kind is the token's first byte,** as simdjson's `switch (*value)`. The compiler
   finds each token row's possible first bytes from its start mask's operations. Where two
   rows can begin with the same byte, the narrower row takes it, and tier 1 gets a guard: the
   other row starting with that byte sends the file to tree-sitter. A keyword is a switch per
   byte, and a `WORD` only where the switches have not read all of it or its length is not its
   row's: JSON's nine escapes are one switch on the second byte and one arm.
4. **The stream** is two of tier 1's extracts, as simdjson's structural indexes: every token's
   first byte, and the last byte of each token whose end its row does not give (a one-byte
   token ends after its byte; a keyword-only row's word gives its length). For JSON, the
   second holds string contents' and words' last bytes. `SPAN` refuses a last byte outside
   its token, and `ACCEPT` one left over. The extract pass reads only the file's bytes, so the
   stream's masks are not cleaned past the end.
5. **One loop.** The verifier refuses any cycle of the program that has no `TOKEN` in it, so
   every loop is the loop over the stream. A `CHECK` runs over the token's 64-byte words inside
   the instruction; nothing runs over bytes.
6. **The backends are the transducer's:** an interpreter of the words, then the new
   instructions in `native_ir` and in Cranelift's lowering, then Metal.

## Consequences

- The Bitlex compiler's output is a whole transducer program: block rows and words.
- The token rows, keywords, `skip`, states and arms left the block program: they are the
  compiler's, and the words carry them. Tier 1's outputs are the fallbacks and the stream.
- Tier 1 rose from 37 operations to 40 and its loop from 174 instructions to 182: it now ends
  in the stream, as stage 1 ends in its structural indexes.
