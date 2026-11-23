# 0645. Kernels are prepared for a named target, and shipped per target

Status: decided (2026-11-23). The user asked for the design ("for production builds, we need to compile kernels for
each of the M families GPUs so we're not limiting ourselves to M1 hardware features ... same thing for CUDA, ROCm,
etc"), and answered the design below with "go ahead". Of its three questions, the user did not answer two:

- **Feature tiers, not exact chips** (question 3): the design's recommendation, taken.
- **The oldest hardware supported** (question 1) and **generated kernels on a machine without a compiler**
  (question 2) are open (below). Neither changes step 1; both are settled before steps 3 and 5.

It builds on decision 0395 (the store, and the kernels shipped for the interpreters) and 0440 (UKL on every
backend), and moves 0617's M5 switch (`UKL_METAL_APPLE10`) into a target's feature.

## Intro

A GPU kernel reaches the device in two compiles:

1. **Its text is prepared:** UKL's preprocessor expands the library into the backend's language. That is where
   features are chosen: an M5's matrix unit, CUDA's `mma.sync`, RDNA2's dot products.
2. **The text is compiled** for the GPU: by Metal on the Mac, NVRTC for CUDA, hiprtc for HIP.

Until now step 1 asked the device in hand which features to take. So a compiled kernel was good only for the
machine that compiled it:

- an AIR made by `kernels ship` on an M1 never used the M5's unit, and one made on an M5 would not run on an M1;
- `accel`'s Metal kernels were prepared with no device features at all;
- CUDA's store keyed a cubin by the local NVRTC's and driver's versions, so a shipped cubin would never match a
  user's machine;
- no release ran `kernels ship`.

A **target** is the fix: a name for what the text is prepared for, whose defines follow from the name alone. A
build prepares for each target it ships; a device chooses its target's name; the two texts are the same.

Not every kernel can be shipped. The kernels generated from a pack's rules are written at run time, and always need
a compiler on the user's machine (question 2). Shipping covers the fixed kernels.

## Decision

### 1. A target is a name, and the name gives the defines

`rerail_ukl::Target { name, backend, defines }`:

- `Target::named(name)`: a shippable target, its backend and defines from the name alone. A build takes it.
- `Target::local(backend, defines)`: OpenCL's and the CPU's, prepared from what the device reports. Their compilers
  are on every machine (the driver's; the system's C++), so nothing of theirs ships.
- `Target::preprocess()`: the preprocessing for it.

The device layer stays below the preprocessor: it reports a name (`Device::target_name`: Metal's tier, `sm_<N>`, the
`gfx` architecture) and, without one, its local defines (`Device::local_defines`). `rerail_gpu::Preprocessing::target`
makes the target of them, and every kernel's text is prepared through it: the gpu crate's kernels and `accel`'s
Metal path (`msl`) alike. `accel`'s library key adds the target's name, as two targets' texts of one source differ.

**The rule:** for a shippable target, the device's text is the text a build prepares from the name. A test checks it
on every device (`a_devices_text_is_its_targets`).

### 2. Kernels test features, not devices

A target's defines are features (`UKL_HAS_MATMUL2D`), or the compiler's own architecture (`__CUDA_ARCH__`). A kernel
never tests a device's name.

### 3. A target exists only where the text differs

The second compile is per chip anyway: on the user's machine, or ahead of time where the platform allows it.

| Backend | Targets | What ships (step 3) |
|---|---|---|
| Metal | `apple7` (M1-M4: `simdgroup_matrix`, bf16 from Metal 3.1; `__METAL_VERSION__` 310); `apple10` (M5: `matmul2d`, Metal 4, 400). A tier for the M2, M3 or M4 only when a feature changes the text. | AIR per target. Per-chip archives (`metal-tt`) optional: Metal refuses another OS version's. |
| CUDA | `sm_<N>`. Ship SASS for sm_75, 80, 86, 89, 90, 100, 120, 121, and PTX for compute_75 and compute_80, which the driver compiles for GPUs newer than the table without NVRTC. An architecture-specific variant (`sm_90a`, `sm_100a`) as exact SASS only, once a kernel uses one. | A fatbin per library |
| HIP | the `gfx` target: `gfx10-3-generic` (RDNA2), `gfx11-generic` (RDNA3's WMMA), `gfx12-generic` (RDNA4), exact `gfx90a`, `gfx942`, `gfx950` (CDNA's MFMA). The wave is the name's: 64 on gfx9's, 32 on RDNA's; `UKL_HIP_AMD_DOT` on the exact RDNA2 targets (`gfx103*`). An exact object only where a generic one measures slower. | Code objects per target |
| OpenCL, the CPU | local | nothing |

A HIP device whose wave is not its name's takes a local target with its own wave: its text is then no shipped one's.

### 4. Shipped kernels are keyed by target; the store by the exact machine

- A shipped kernel's key: its target's name, the source's digest and the options. The store's key stays exact, the
  local compiler's and driver's versions in it.
- Lookup: this process; the store; the shipped kernels, of the best target the device runs; a local compile; with no
  local compiler, shipped PTX or a generic code object, which the driver compiles; else the CPU.

### 5. Builds cross-compile; devices validate

- `kernels ship --targets` runs where the compilers are, without the GPUs: a Mac with the Metal Toolchain
  (`xcrun metal`, `metal-tt`), Linux with the CUDA toolkit and ROCm. The release job ships `rerail-kernels/`.
- A target ships only once its tests pass on a device of it. The farm has `apple7` (the M1, the M1 Max, the M3
  Max), `apple10` (the M5 Max), sm_70 (the V100), sm_121 (the GB10s), gfx1035 (the Radeon 680M, by HIP and by
  OpenCL) and the Arc 140V (OpenCL). Without a device: the M2 and M4, sm_75 to sm_120, RDNA3 and 4, CDNA. Those
  ship PTX or a generic object only, or wait for a device.

## Order of work

1. The target, the names' table and the features; every preparation through it; the test of the rule. Done
   2026-11-23 ("Step 1, built").
2. Shipped kernels keyed by target, and the lookup of the best target a device runs.
3. `kernels ship --targets`: Metal's AIR per target, CUDA's fatbin, HIP's code objects.
4. The release workflow ships `rerail-kernels/`.
5. A compiler for the generated kernels where the machine has none (question 2).

### Step 1, built (2026-11-23)

- `rerail_ukl::Target` (`native/crates/ukl/src/target.rs`): `named`, `local`, `shippable`, `preprocess`, and Metal's
  tiers (`METAL`). `metal_source` takes a target; the forest's build-time Metal text is `apple7`'s.
- The device reports its name: Metal's `target_name` (`apple10` on Apple's GPU family 10, else `apple7`;
  `RERAIL_METAL_MATRIX_UNITS=0` takes `apple7`), CUDA's `sm_<N>`, HIP's `gfx` architecture. `compile_defines` is gone:
  its Metal and HIP features are the names' now, and OpenCL's and the CPU's are `local_defines`.
- `accel`'s Metal kernels are prepared for the device's tier (they had no features), and its library key holds the
  tier's name. On the M5 its bit product now takes the M5's C layout, as the gpu crate's kernels do.
- 0617's `UKL_METAL_APPLE10` is `apple10`'s feature `UKL_HAS_MATMUL2D`.
- **Passing:**
  - `targets_are_their_names` (UKL's tests, all 14, on Linux);
  - `a_devices_text_is_its_targets`, with the matrix tests: on the M5 Max (`apple10`; `apple7` with the switch
    off), the M1 (`apple7`), the GB10 (`sm_121`), the Radeon 680M (`gfx1035` by HIP; OpenCL's local target);
  - `accel`'s GPU tests on the M5 Max (36).

## Open

- **The oldest hardware supported** (question 1). CUDA 13's toolkit is believed to compile no longer for sm_70: the
  V100 would need a 12.x toolkit as well, or only the NVRTC on its machine. Turing (sm_75) and RDNA2 have the same
  question.
- **Generated kernels on a machine without a compiler** (question 2). Metal always has one. For CUDA, NVRTC could be
  shipped (its licence to be checked for redistribution); hiprtc needs `comgr`, a large LLVM. Else the toolkit or ROCm
  is required, or those kernels run on the CPU there.
