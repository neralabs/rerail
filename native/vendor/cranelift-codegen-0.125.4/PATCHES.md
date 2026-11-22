# Cranelift 0.125.4, with `clmul` and `preserve_most`

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
    `i64x2`, lane 0) is read in place, with no move out and back (0346);
  - the x64 lowering, `pclmulqdq` when `has_pclmulqdq` is set;
  - aarch64's constants (0381): a 128-bit constant repeating a 64-bit value that no `movi`
    makes, and that doesn't repeat a 32-bit one, is loaded from the constant pool (one `ldr q`),
    not built in a general register and duplicated (up to five instructions: a bitmask's
    weights); `clmul` by all ones takes them from a `movi`;
  - aarch64's `cmtst` (0381): `VecALUOp::Cmtst` (CMEQ's register encoding with U = 0), its
    printing and an encoding test; `icmp ne (band x m), 0` on an integer vector lowers to it, as
    NEON's `vtst` (one instruction, where it was `and`, `cmeq #0` and a `not` or a complement);
  - `links = "cranelift_codegen"` and `cargo:clmul=1` in `build.rs`, so a dependent's build
    script sees `DEP_CRANELIFT_CODEGEN_CLMUL` (rerail's `compiler/build.rs` sets the cfg
    `cranelift_clmul` from it).
  - a calling convention, `preserve_most` (`CallConv::PreserveMost`), as LLVM's, for a cold
    helper. The callee saves every general register it writes but the returns' and the
    scratch ones (aarch64: x0, x1, x9, x16, x17; x86-64: rax, rdx, r11), so a caller's values
    stay in registers across the call. Vectors stay as the platform's default. Arguments and
    returns are System V's (at most two returns). `build.rs` emits `cargo:preserve_most=1`,
    and rerail's `compiler/build.rs` sets the cfg `cranelift_preserve_most` from it;
  - a load or store whose base is `get_pinned_reg` reads the pinned register in place (aarch64
    `amode`, x64 `to_amode`), as `sp`, where it was first moved to a register of its own.
  - for UKL's vector code on the CPU (decision 0567 step 3, `rerail-ukl-vec`), what its lowering cannot shape:
    - the elaboration (`egraph/elaborate.rs`): a pure value is placed no deeper than its uses' common dominator, so
      sibling blocks share it. A value made in a loop stays where its arguments are, shared by its uses in and
      after the loop. Each block's walk starts from its parent's loop stack: a sibling outside the loop had popped
      it, and LICM hoisted nothing;
    - the rewrites (`opts.rs`, `opts/arithmetic.isle`, `prelude_opt.isle`): `((a + b) + k) + d` is not rebalanced
      to `(a + b) + (k + d)` when `k` is a constant, whose sum would be made in the loop. An add of a constant is
      not rematerialized in a loop deeper than where it is;
    - AArch64's addresses (`isa/aarch64/lower.isle`, `machinst/lower.rs`, `prelude_lower.isle`): a sum with a
      constant offset is one register and an immediate, shared by the loads at offsets. A sum that several loads
      and stores use (`value_is_multiply_used`, by the CLIF's direct uses) is their base register at no offset;
    - pairs (`machinst/vcode.rs`, `machinst/mod.rs`, `isa/aarch64/inst/mod.rs`): two 128-bit loads or stores 16
      bytes apart from one base, adjacent once registers are allocated, are emitted as one `ldp` or `stp`
      (`MachInst::pair`, at emission; loads that cannot trap only);
    - reductions: a 32-bit lanes' sum of two pairwise sums of a vector with itself is one `addv` on AArch64, and
      x86-64 lowers 64-bit lanes' pairwise sums (`iadd_pairwise` of `i64x2`).
- `cranelift-codegen-meta-0.125.4`: the instruction, and the two ISA flags.
- `cranelift-assembler-x64-meta-0.125.4`: `pclmulqdq`, and its CPU feature.
- `cranelift-native-0.125.4`: detects `pmull` and `pclmulqdq` on the host.

Rerail builds against upstream Cranelift as well: without `DEP_CRANELIFT_CODEGEN_CLMUL` the
cfg is off and a toggle is the rounds; without `DEP_CRANELIFT_CODEGEN_PRESERVE_MOST` the cold
helpers take the platform's convention. The other workspaces (`native/wasm`, `tools/bitlex`)
keep upstream.
