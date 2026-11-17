# Upstream Cranelift: a draft of our changes

Status: a draft for the user to submit (2026-11-17). We don't file or comment upstream
ourselves. The changes are in `native/vendor/cranelift-codegen-0.125.4/PATCHES.md`. The source
is neralabs/wasmtime, branch `nera/clmul-v38.0.4`, against Wasmtime 38.0.4. Upstream `main` is
at 0.138, so each change has to be ported before it can be submitted.

## Intro

Our vendored Cranelift differs from crates.io's in three ways. Each can go upstream on its own,
and they get smaller and easier to accept down the list:

1. **A `clmul` instruction:** a carry-less multiply, lowered to `pmull` on aarch64 and
   `pclmulqdq` on x86-64. It is a new IR instruction, so upstream will want a reason beyond our
   use. Proposing it as an issue first is the safer route.
2. **aarch64 `cmtst`:** `icmp ne (band x m), 0` on an integer vector lowers to one `cmtst`.
   Today it is `and`, `cmeq #0` and a `not`. This is a peephole on existing IR, with an encoding
   test.
3. **aarch64 128-bit constants:** a constant that repeats a 64-bit value which no `movi` makes is
   loaded from the constant pool with one `ldr q`. Today it is built in a general register and
   duplicated, which takes up to five instructions.

Items 2 and 3 help any SIMD code, wasm included, and need no discussion. Submitting them first
would build some trust before item 1.

## 1. Issue: a carry-less multiply instruction

> **Title:** Cranelift: add `clmul`, a carry-less multiply (aarch64 `pmull`, x86-64 `pclmulqdq`)
>
> Cranelift has no carry-less multiply, so a frontend that needs one writes it as a loop of
> shifts and XORs. Both 64-bit targets have it as a single instruction, and so do LLVM's
> intrinsics (`llvm.aarch64.neon.pmull64`, `llvm.x86.pclmulqdq`).
>
> We use it for the prefix XOR of a bit mask, as simdjson's string scanner does:
> `clmul(x, -1)` sets each bit to the XOR of every bit of `x` at or below it. That marks the
> bytes inside quoted strings in one instruction instead of six shift-XOR rounds. CRC and
> GHASH are other common uses.
>
> Proposed semantics:
>
> ```text
> v2 = clmul v0, v1   ; i8..i64: the low half of the carry-less product of v0 and v1
> ```
>
> Lowering:
> - aarch64: `pmull v.1q, v.1d, v.1d`, where the `has_pmull` flag (FEAT_PMULL) is set. An operand
>   that is lane 0 of an `i64x2` is read in place, with no move to and from a general register.
> - x86-64: `pclmulqdq $0`, where `has_pclmulqdq` is set.
> - Elsewhere, or with the flag unset: not lowered. The frontend checks the flag and emits the
>   rounds, as it does for other optional features.
>
> `cranelift-native` detects both features on the host.
>
> Would a PR along these lines be welcome? Should the high half (`clmulhi`) or a vector form
> come with it?

### What the PR contains, from our branch

- `cranelift-codegen-meta`: the instruction (`shared/instructions.rs`), the two ISA flags
  (`isa/arm64.rs`, `isa/x86.rs`).
- `cranelift-codegen`:
  - aarch64: the ISLE lowering, the encoding and printing of `pmull`, and the lane-0 operand
    rule;
  - x64: the ISLE lowering.
- `cranelift-assembler-x64-meta`: `pclmulqdq` and its CPU feature.
- `cranelift-native`: feature detection.
- Filetests: `runtests/clmul.clif`, `isa/aarch64/clmul.clif`, `isa/x64/clmul.clif`.

### Still to do before submitting

- Port to upstream `main` (0.138). The ISLE files and the x64 assembler's metadata have moved
  since 0.125.
- Add `clmul` to `cranelift-interpreter`, so the runtest also runs interpreted. Our branch
  doesn't vendor the interpreter: check what upstream's runtests require.
- Add a fuzzgen rule for `clmul`, or note in the PR why it has none.
- An s390x and riscv64 story. s390x has `vgfm` and riscv64 has `clmul` in Zbc. The PR could
  lower them too, or leave them unlowered behind their flags.

## 2. PR: aarch64 `cmtst` for `icmp ne (band x m), 0`

> **Title:** aarch64: lower `icmp ne (band x y), 0` on integer vectors to `cmtst`
>
> NEON's `cmtst` (`vtst` in the intrinsics) sets each lane to all ones where `x & y` is not
> zero. Cranelift currently emits `and`, `cmeq #0` and `mvn` for this pattern. The PR adds
> `VecALUOp::Cmtst` (CMEQ's register encoding with U = 0), its printing, an `emit_tests` entry,
> an ISLE rule in `lower.isle`, and a filetest for each lane size.

One instruction instead of three. In our byte classifier this pattern appears once for each
class test on every 16 bytes.

## 3. PR: aarch64 128-bit constants from the constant pool

> **Title:** aarch64: load a 128-bit constant that repeats a 64-bit value from the constant pool
>
> A `vconst` whose two halves are the same 64-bit value, which `movi` can't make and which
> doesn't repeat a 32-bit value, is currently built in a general register (`movz` and up to
> three `movk`) and then duplicated (`dup`). One `ldr q` from the constant pool is shorter, and
> nothing waits on it. The PR changes the `constant_f128` rules' order and adds a filetest. A
> typical case is the bit weights `0x8040201008040201` of a NEON bitmask.

## Measured, for the PRs' text if asked

Taken from our measurements (decision 0343), on Apple M3 and M5 cores:

- `pmull` has the same latency as the six rounds (about 13 cycles for a dependent chain), and two
  to three times their throughput.
- In a JSON structural indexer, the multiply and the rounds are within ±6% of each other overall.
  The multiply wins where it reads the mask straight from the vector register, without moving it
  to a general register first.
