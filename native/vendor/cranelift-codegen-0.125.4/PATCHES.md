# Cranelift 0.125.4, with `clmul`

crates.io's cranelift-codegen 0.125.4 (Wasmtime 38.0.4), with one instruction added: `clmul`,
a carry-less multiply. It is the source of neralabs/wasmtime, branch `nera/clmul-v38.0.4`
(private for now), whose filetests cover it (`runtests/clmul.clif`, `isa/aarch64/clmul.clif`,
`isa/x64/clmul.clif`). Upstream has no carry-less multiply, up to 0.138.

```text
v2 = clmul v0, v1    ; the low half of the carry-less product of two i8..i64
```

Bitlex uses it for a toggle, as simdjson's `prefix_xor`: `clmul(x, !0)` is the XOR of every
bit of `x` at or below each position (0340). Without it a toggle is six shift-XOR rounds.

Four crates change; each is crates.io's 0.125.4 with the branch's diff for it:

- `cranelift-codegen-0.125.4` (here):
  - the aarch64 lowering, `pmull v.1q, v.1d, v.1d` when the ISA flag `has_pmull` is set, its
    encoding and printing; a multiplicand that is a vector's low lane (`extractlane` of an
    `i64x2`, lane 0) is read in place, with no move out and back (0345);
  - the x64 lowering, `pclmulqdq` when `has_pclmulqdq` is set;
  - `links = "cranelift_codegen"` and `cargo:clmul=1` in `build.rs`, so a dependent's build
    script sees `DEP_CRANELIFT_CODEGEN_CLMUL` (rerail's `compiler/build.rs` sets the cfg
    `cranelift_clmul` from it).
- `cranelift-codegen-meta-0.125.4`: the instruction, and the two ISA flags.
- `cranelift-assembler-x64-meta-0.125.4`: `pclmulqdq`, and its CPU feature.
- `cranelift-native-0.125.4`: detects `pmull` and `pclmulqdq` on the host.

Rerail builds against upstream Cranelift as well: without `DEP_CRANELIFT_CODEGEN_CLMUL` the
cfg is off and a toggle is the rounds. The other workspaces (`native/wasm`, `tools/bitlex`)
keep upstream.
