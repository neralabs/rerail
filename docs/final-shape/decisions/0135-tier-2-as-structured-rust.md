# 0135: Tier 2 as structured Rust, from the native IR

Status: built (2026-11-12). Tier 2 ahead of time: 309.6M instructions, 55.4M cycles,
then 307.0M and 54.3M with a node's kind and field one store.

## Intro

**The problem.** The ahead-of-time build turned the JIT's Cranelift IR into Rust
(`clif_rust`). That Rust was a function of blocks and jumps:
rustc and LLVM compiled it, but it looked nothing like the lean parser,
the hand-written baseline it is measured against.

The lean parser is a function a grammar rule, called as Rust functions are,
each holding the parser's state as `&mut self`.

**The decision.** Tier 2's Rust comes from the native IR, through a backend of its own
(`rust_stream`), beside Cranelift's. Its shape is the lean parser's:

```
// A grammar rule: its words, structured; a call to another rule a Rust call.
#[inline(never)]
fn r209(s: &mut S, mut word: u32, mut flags: u32) -> Result<bool, bool> {
    s.token_all()?;                 // TOKEN
    match s.code & 0xff {           // SWITCH on the record's kind
        3 | 7 => { s.open(39 | 2 << 16); … }
        _ => return Err(false),     // the one failure exit
    }
}
```

- Each rule is a function. A grammar call is a Rust call, its `RETURN` an `Ok(again)`.
- The words are structured by `clif_rust::Shape`, now over any graph:
  `if`, `match` and `loop`, no jumps.
- A `SWITCH` is a `match` on the record's kind, or on a byte.
- A refusal is one exit, `Err`. Node rows are written inline.
- A `TOKEN` whose phase is known takes a token, or none, with no flag.
- A `CHECK` is word-parallel (below), as is the JIT's now.

**A local check, word-parallel.** At a token, the check's mask word, cut to the token's bytes.
Where it has a bit, the check's ops run over the word's 64 places at once:
byte sets are tier 1's nibble-table classes over the word's four vectors, then a bitmask;
`Prev` is a shift across the word before; the test is one AND.
No loop over bits or bytes. The ladder's shape check now refuses a loop over bits in tier 2.

## Measured and dropped

The state's shape was measured four ways, over the corpus:

| Shape | Instructions | Cycles |
|---|---|---|
| All state behind `&mut S` (kept) | 316.2M | 57.1M |
| The whole state as locals, copied at calls | 406.0M | 79.5M |
| Pointers behind a read-only `&P` | 351.6M | 59.6M |
| Hot fields local, the rest in memory | 370.9M | 69.2M |

At about one rule call a token, copies and saved registers at calls cost more than
the loads they save.

## How

- `compiler/src/rust_stream.rs`: `rust_source(&Structure)`; `word_form`, the check's classes,
  shared with Cranelift's `check_word`.
- `rr_aot_gen` writes tier 1 from `clif_rust::rust_source` (tier 1 alone) and tier 2 from
  `rust_stream`.

Tests: every tape over the corpus is the JIT's; `\u` escapes at every place across two words'
edges, good and bad, as the engine's tape.
