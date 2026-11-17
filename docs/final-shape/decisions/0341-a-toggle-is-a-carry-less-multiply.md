# 0341: A toggle is a carry-less multiply

Status: decided (2026-11-17); landed on the CPU (the JIT, and the AOT's Rust). Follows 0338.

**Corrected by 0343.** The premise below is wrong: simdjson 5.0.5's arm64 `prefix_xor` is the
six shift-XOR rounds. Its `vmull_p64` line is commented out ("apparently slow"). Whether tier 1
keeps `clmul` is open.

## Intro

**The problem.** A toggle `t = toggle(x)` holds between flips: bit `i` of `t` is the XOR of
`x`'s bits up to `i`, with the parity carried in from the block before. JSON's string is one
(0338). simdjson makes it with one instruction, a carry-less multiply by all ones:

```cpp
// simdjson's arm64 prefix_xor
simdjson_inline uint64_t prefix_xor(uint64_t bitmask) {
  return vmull_p64(-1ULL, bitmask);
}
```

Cranelift has no carry-less multiply (none up to 0.138), so tier 1 made it as six shift-XOR
rounds, twelve operations where simdjson's stage 1 has one:

```rust
// tier 1, before: the rounds, then the parity
let mut x = quote;
for k in [1, 2, 4, 8, 16, 32] { x ^= x << k }
x ^ parity
```

## The decision

**Cranelift gets the instruction.** A fork, neralabs/wasmtime, branch `nera/clmul-v38.0.4`
(private for now), on our Cranelift 0.125.4 (Wasmtime 38.0.4), adds `clmul`: the low half of
the carry-less product of two integers.

- aarch64 lowers it to `pmull v.1q, v.1d, v.1d` (`vmull_p64`), with the ISA flag `has_pmull`;
- x64 to `pclmulqdq`, with `has_pclmulqdq`;
- `cranelift-native` detects both on the host; the interpreter runs it;
- filetests: a run test (interpreter, aarch64, x64), and both lowerings' output.

The four changed crates are vendored under `native/vendor` (the convention of
`tree-sitter-0.27.1`), with `[patch.crates-io]` entries: CI cannot fetch a private repository.
`native/vendor/cranelift-codegen-0.125.4/PATCHES.md` lists them.

**Rerail builds against either Cranelift.** The vendored `cranelift-codegen` has
`links = "cranelift_codegen"`, and its `build.rs` prints `cargo:clmul=1`. The compiler's
`build.rs` sets the cfg `cranelift_clmul` when it sees `DEP_CRANELIFT_CODEGEN_CLMUL`.
Upstream's Cranelift sets no such variable, and a toggle is the rounds.

**The toggle is simdjson's.** `native_cpu_block::prefix_xor` emits `clmul(x, !0)` where the cfg
is set and the host has the multiply (`clmul()`), and the rounds otherwise:

```rust
// tier 1's generated Rust (the AOT), now
let t = (unsafe { core::arch::aarch64::vmull_p64(quote, 0xffffffffffffffff) } as u64) ^ parity;
```

Every module's ISA is the host's with its detected flags. Tier 2's own ISA (`Cut::unsigned`)
was `aarch64-apple-darwin` with no CPU flags; it is now `cranelift_native::builder()`, as the
JIT's own.

`clif_rust` prints `clmul` as `vmull_p64`, as simdjson writes it.

## What is left against simdjson's stage 1

- The outputs go to memory each block (an 88-byte record), and the token loop's state crosses
  each block through a stack array (0177's boundary).
- The token loop: numbers and short strings.

## Tests

- rr: `tmp/verify.sh` (the parser's named tests, and the AOT's tapes are the JIT's).
- `rr_parser`'s `jsons_cpu_records_are_the_interpreters`, `jsons_tier2_split_is_tier1s`,
  `json_loop_against_step_14`.
- The fork's filetests: `runtests/clmul.clif`, `isa/aarch64/clmul.clif`, `isa/x64/clmul.clif`,
  and the aarch64 emit tests.
- Against upstream: `cargo check -p rerail-compiler` with the patch entries removed.
