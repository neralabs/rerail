# 0072: Tier 2 on Metal, from a dependency analysis

Status: accepted (user, 2026-11-07: "go ahead"). The design is
`docs/parser-generators/14-tier-2-design.md`. Superseded in part by 0077: the rows'
conditions, backward keys and the 8-byte stack effect give way to a grammar, forward rules and
a depth carry; Metal comes after the CPU.

## Intro

Tier 2 had been a CPU writer that interpreted rows in Rust, 600x slower than simdjson's stage
2. It is deleted. Tier 2 is generated code, compiled from the Bitlex file, like tier 1.

On Metal, the work is split by what each value depends on. Most of it is local and runs in
parallel. The rest is resolved by prefix scans over small per-chunk entries. The design is made
for JSON first, then generalised.

## Prior work consulted

- **`06-block-parallel.md`:** summaries, a scan, then apply. **Agrees.** This record makes its
  rounds concrete for JSON and chains its scans by dependency depth.
- **`13-tape-on-the-gpu.md` §3 and §5:** decoupled look-back between threadgroups, and
  persistent threadgroups. **Changed:** one threadgroup owns a run of files, and every round
  ends with a threadgroup barrier. Nothing waits on another threadgroup.
- **Lessons ledger:** no GPU parsing lessons. #28 (only CPU and GPU time count) holds: every
  choice the Bitlex file can set is measured.
- **ast-index-spike indexes:** no GPU work.

## Decision

1. **An analysis pass orders the work.** Each value is local, forward or backward. Carried
   values are resolved by scans, ordered by depth. A scan is fused into the pass before it
   when its dependence on an earlier scan comes through a few bits at a block's edge, by
   storing a candidate per value of those bits.
2. **JSON's plan:**
   - pass A over the bytes;
   - scans S1 (strings), S2 (the stack, keys), S3 (counts);
   - pass B over the bytes, writing the tape;
   - pass C over the tape.
3. **Entries per 2 KB,** with four candidates always: string state × escape state at the
   block's first byte. About 57 bytes each. Entries go in threadgroup memory, or in device
   memory past it.
4. **The stack effect is 8 bytes:** closes and opens, their counts and kinds, the innermost
   pair bit, and an error.
5. **Keys are found backward:** a string whose next token is `:`.
6. **Matching a close to its open** is set in Bitlex: `match barrier` (the default) or
   `match scan`. Both are measured.
7. **Pass B writes the tape directly.** Pass C finishes `off` and narrows `off` and `tail`.
8. **Large files run in sequential chunks** in one threadgroup. A chunk commits up to its last
   block boundary before an unresolved backward dependency, and carries its state forward.
9. **The CPU** runs one generated loop per file, with no scans.
