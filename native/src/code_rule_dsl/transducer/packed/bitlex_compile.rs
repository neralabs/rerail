//! The Bitlex compiler: a Bitlex file (`docs/parser-generators/03-bitlex.md`) to a block program
//! (`block`), printed as `rir` rows so it goes through the transducer's own assembler and
//! verifier. It holds no language: everything comes from the file.
//!
//! The parser is the reference evaluator's (`tools/bitlex`), so Bitlex is parsed in one place, and
//! the evaluator is the compiler's test oracle (`bitlex_compile_tests`).
//!
//! Built one construct at a time (`docs/parser-generators/13-tape-on-the-gpu.md` §7, step 2).
//!
//! - **Classes.** Each class is a union of products of a set of high nibbles and a set of low
//!   nibbles. Products are shared between classes and packed eight to a nibble table. A one-byte
//!   class is an `eq`.
//! - **Masks** over `~ & | ^`, `empty`, `prev`, `next`, `starts`, `lasts`, `after`, `span`,
//!   `aligned`, `toggle`, `after_odd_run` and `fill`.
//! - **Counts** as bit-planes, and `C.frame(a)` (below).
//! - **Numbers** (`number`) as bit-planes too: `NUMBER_BITS` of them, a byte's value mod 2^32
//!   (below).
//! - **Tokens**, `keywords` and `skip`: each token kind's start and last-byte masks, and the
//!   coverage and overlap checks.
//! - **The tree section** (`docs/parser-generators/18-tree-as-grammar.md`): the grammar's state
//!   machine (`grammar`), with grammar ids from `Symbols`, lowered to tier 2's words
//!   (`bitlex_tier2`, decision 0087); tier 1 outputs the stream they read, two extracts.
//! - **Errors and escapes.** Each mask joins the fallback output (`FALLBACK`); a file with a bit
//!   of it goes to tree-sitter.
//!
//! Anything else is refused by name.
//!
//! # Counts and frames
//!
//! A count `count(up, down) max M` is kept as `w` bit-planes of its value after each byte, with
//! `2^w >= M + 2`. Bit `k` flips at an up byte whose bits below `k` were all 1, and at a down
//! byte whose bits below `k` were all 0, so plane `k` is a `toggle` of a mask of the planes
//! below it one byte back: no new operation. The value is kept mod `2^w`; a count that leaves
//! `0..=M` passes through `M + 1` or `-1`, both above `M` as `w`-bit numbers, and one compare
//! escapes the file. (The evaluator holds a count at 0 instead: a file that goes below 0 escapes
//! here and is lexed there; such a file is malformed for JSON.)
//!
//! `C.frame(a)` is one `fill` per level `L` in `1..=M` (`10-bit-programs.md` §2): from the
//! openers at level `L` in `a` to the next opener at level `L` not in `a`, kept where the count
//! before the byte is `L`.
//!
//! # Numbers
//!
//! A number is `NUMBER_BITS` planes, plane `k` the bytes whose value has bit `k`. A file is
//! shorter than 2^32 bytes, so a place in it, or a distance, is exact.
//!
//! - `position`: plane `k` a pattern of the word below bit 6 (`const`), the block's place
//!   from bit 6 on (`position k`, a new leaf with no carry).
//! - `fill(s, e, v)`: plane `k` is `fill(s & v_k, e | s)`, a fill that restarts at every byte
//!   of `s` and runs where `v` had bit `k` there; `back_fill` likewise.
//! - `a - b`: a ripple of borrows over the planes, three operations a plane.
//! - `at_least(a, b)`: no borrow out of `a - b`; `above(a, b)` is `~at_least(b, a)`.
//!
//! No new operation but `position`: every plane is a mask, every step a word operation.
//!
//! # Delays: `next` without lookahead
//!
//! Every value is computed some bytes late: its bit `p` is byte `p - d`'s, for its delay `d`.
//! Bytes and classes have delay 0. An operation's operands are brought to the later delay by
//! `prev`, and `next(a, k)` is `a` itself with `k` more delay. The outputs are brought to the
//! largest delay `D`, the program's `delay`; each executor runs `D` bytes past the stream and
//! shifts the outputs back (`block::undelay`).
//!
//! So that no value has bits outside the stream (which `prev` and `next` would carry in):
//!
//! - before the stream every value is zero: `prev` shifts in zeros, `~a` is `within & ~a` with
//!   `within` (the stream's bytes) at the same delay, and `next` clears the bytes it brings in
//!   before the start;
//! - past the end a value may be *dirty*: a byte test that the zero padding passes (`eq 0x00`),
//!   a constant, or a carried operation whose state runs on. `next` cleans its operand with
//!   `within` when it is dirty.

// The compiler, the compiler crate's (`rerail_compiler::bitlex_masks`): masks, tokens and the
// tree section, a Bitlex file to its block program and tier 2's words.
pub(in crate::code_rule_dsl) use rerail_compiler::bitlex_masks::{compile, Compiled, Cost, FALLBACK, Named, STREAM_LAST, STREAM_MARK, STREAM_START, Symbols};
