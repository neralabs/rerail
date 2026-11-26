# 0645. Kernels are prepared for a named target, and shipped per target

Status: decided (2026-11-23). The user asked for the design ("for production builds, we need to compile kernels for
each of the M families GPUs so we're not limiting ourselves to M1 hardware features ... same thing for CUDA, ROCm,
etc"), and answered the design below with "go ahead", then its two open questions:

- **Feature tiers, not exact chips** (question 3): the design's recommendation, taken with "go ahead".
- **The oldest hardware** (question 1): "support the V100 and everything up, including the toolkits needed".
- **A machine without a GPU compiler** (question 2): "require the user to install what is needed for GPU support,
  we will not ship with it. This needs to be a clear message so it's actionable".

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
a compiler on the user's machine. Shipping covers the fixed kernels; the compiler is the user's to install (§6).

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
| CUDA | `sm_<N>`. Ship SASS for sm_70 (the V100) and sm_75, 80, 86, 89, 90, 100, 120, 121, and PTX for compute_70 and compute_80, which the driver compiles for GPUs newer than the table. sm_70 is built with a 12.x toolkit (CUDA 13 is believed to have dropped it; the build checks NVRTC's own list), the rest with the newest. An architecture-specific variant (`sm_90a`, `sm_100a`) as exact SASS only, once a kernel uses one. | A fatbin per library |
| HIP | the `gfx` target, exact: RDNA2 (`gfx1030`-`gfx1036`, with `UKL_HIP_AMD_DOT`), RDNA3 (`gfx1100`-`gfx1103`, `gfx1150`, `gfx1151`), RDNA4 (`gfx1200`, `gfx1201`), CDNA (`gfx90a`, `gfx942`, `gfx950`). The wave is the name's: 64 on gfx9's, 32 on RDNA's. Not the generic targets (`gfx10-3-generic`): the HIP SDK's hiprtc has no device library for them (step 3). A generic object shipped would still be taken. | Code objects per target |
| OpenCL, the CPU | local | nothing |

A HIP device whose wave is not its name's takes a local target with its own wave: its text is then no shipped one's.

### 4. Shipped kernels are keyed by target; the store by the exact machine

- A shipped kernel's key: its target's name and its prepared text (`<dir>/gpu/<target>/<key>.<form>`); the options
  are the backend's fixed ones, the device's compile's and the build's alike. The store's key stays exact, the
  local compiler's and driver's versions in it.
- Lookup: this process; the shipped kernels; the store; a local compile. The shipped kernels are searched under each
  target the device runs (`Target::runs`), best first: its own; Metal's lower tier (`apple7` under `apple10`);
  CUDA's SASS of the same major architecture and a lower minor one, then PTX up to it; an AMD target's generic
  family. The device looks up its own text under each, so another target's code is taken only where it was compiled
  from the very text the device prepared: no feature of the device's text is lost (an M5's matrix kernels never take
  an M1's AIR; its other kernels do).
- A GPU backend runs only with its compiler installed (§6). Without it the process runs on the CPU, and says what
  to install.

### 5. Builds cross-compile; devices validate

- `kernels ship --targets` runs where the compilers are, without the GPUs: a Mac with the Metal Toolchain
  (`xcrun metal`), Linux with NVRTC and ROCm's hiprtc, Windows with NVRTC (its CUDA text differs: `long` is 32
  bits). The engine runs on its CPU device there: a kernel's text is the target's, not the device's (measured,
  step 4). The release ships `native/rerail-kernels/`, beside the engines.
- A target ships only once its tests pass on a device of it. The farm has `apple7` (the M1, the M1 Max, the M3
  Max), `apple10` (the M5 Max), sm_70 (the V100), sm_121 (the GB10s), gfx1035 (the Radeon 680M, by HIP and by
  OpenCL) and the Arc 140V (OpenCL). Without a device: the M2 and M4, sm_75 to sm_120, RDNA3 and 4, CDNA. Those
  ship PTX or a generic object only, or wait for a device.

### 6. The user installs the GPU compiler; Rerail says which, and where

Rerail ships no GPU compiler: NVRTC (the CUDA Toolkit) and hiprtc (ROCm, the HIP SDK) are the user's to install.
Metal's is macOS's, OpenCL's the driver's. Where a machine has a GPU it cannot use, the process says so once, on
stderr, with what to install, where to get it, and the variable that names an install elsewhere
(`rerail: GPU support: ...`):

- **A GPU driver without its compiler:** NVIDIA's driver without NVRTC ("Install the CUDA Toolkit (12.x or later;
  12.x for a Tesla V100 or another sm_70 GPU): <NVIDIA's archive>. Installed elsewhere, set CUDA_HOME to it", Windows'
  `CUDA_PATH`); AMD's HIP runtime without hiprtc (ROCm and `ROCM_PATH` on Linux; the HIP SDK and `HIP_PATH` on
  Windows).
- **A compiler that does not compile for the GPU:** NVRTC lists its architectures (`nvrtcGetSupportedArchs`). A GPU
  older than the list is told the older toolkit that compiles for it (12.x for sm_70), a newer one a newer toolkit.
- A machine with no GPU driver says nothing, and neither does one whose GPU another backend opened (Windows'
  OpenCL, where HIP's compiler is missing).
- The toolkit the user names comes first: `CUDA_HOME` (`CUDA_PATH`) is searched before the system's libraries, so a
  V100's machine with a CUDA 13 toolkit installed takes the 12.x one the message asks for.

## Order of work

1. The target, the names' table and the features; every preparation through it; the test of the rule. Done
   2026-11-23 ("Step 1, built").
2. Shipped kernels keyed by target, and the lookup of the best target a device runs. Done 2026-11-23.
3. `kernels ship --targets`: Metal's AIR per target, CUDA's SASS and PTX, HIP's code objects. Done 2026-11-23.
4. The release workflow ships `rerail-kernels/`. Done 2026-11-23, not yet run in CI.
5. The message where the GPU's compiler is missing or does not compile for it (§6). Done 2026-11-23 ("Step 5,
   built").

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

### Step 5, built (2026-11-23)

- `compute_kernel_cuda::needs`: what the user must install, set where the driver loads and its compiler does not
  (CUDA's NVRTC, HIP's hiprtc), or where NVRTC's architectures do not hold the GPU's (`needs_for_arch`).
  `rerail_device::shared` says it where the process is left without a GPU.
- `CUDA_HOME` and `CUDA_PATH` are searched first; `/usr/local/cuda` only where `CUDA_HOME` is not set.
- **Passing:** `a_gpu_nvrtc_does_not_compile_for_is_told_the_toolkit` (the V100 under a toolkit without sm_70, a GPU
  newer than the toolkit), on Linux; the GB10 opens as before, its sm_121 in its NVRTC's list. The missing-compiler
  messages are not run on a device: the farm's machines have their compilers in the system's search.

### Steps 2-4, built (2026-11-23)

- **The targets** (`rerail_ukl::target`): `SHIPPED`, the release's; `AMD_GENERIC`; `compute_<N>` (PTX);
  `Target::form` (`metallib`, `cubin`, `ptx`, `hsaco`) and `Target::runs`.
- **The lookup** (`rerail_gpu::kernels::shipped`, `shipped_path`): in `program_with`, before the store, for the
  device's text under each target it runs. `ComputeFunctionLibrary::from_compiled` loads the code (Metal's
  `newLibraryWithData:`, CUDA's and HIP's `cuModuleLoadData`). `accel`'s Metal kernels look up their AIR the same
  way (`shipped_air`: the device's tier, then a lower tier's of the same text), keyed per tier
  (`Key::for_target`).
- **The compiles without a GPU** (`rerail_device::device::compile_offline`):
  - Metal: `xcrun metal` with `-include metal_stdlib` (the device's compile includes it itself); `apple7` as Metal
    3.2 for macOS 15 (UKL's atomics take 3.2's coherent memory and thread scopes; 3.1 refused them), `apple10` as
    Metal 4 for macOS 26.
  - CUDA: NVRTC loaded alone (`Rtc`), the newest that lists the architecture, of `RERAIL_SHIP_CUDA_HOMES`'
    toolkits and the system's. CUDA 13's NVRTC lists no sm_70: the V100's kernels need 12.x (NVIDIA's
    `nvidia-cuda-nvrtc-cu12` wheel is enough).
  - AMD: hiprtc alone. The HIP SDK's (7.2, Windows) compiles the exact targets, not the generic ones: "cannot find
    ROCm device library for gfx10-3-generic". So the release ships exact targets.
- **The build** (`rerail-engine kernels ship <dir> --targets <t,...>|metal|cuda|amd|all`, `rerail_gpu::kernels::
  ship_for`): each program the process compiles is prepared and compiled for each target too. A program that does
  not compile for a target fails the command. Without a Metal device `accel`'s Metal part is skipped, and said so.
  `accel`'s part ships only this build's libraries of its manifest (an older build's are counted and left).
- **Found beside the engine:** `store::shipped_dir` looks beside the program, then beside the library the engine
  was loaded from (`os::own_image`: `dladdr`, Windows' `GetModuleHandleExW`), the Node addon in the package's
  `native/`. `accel` takes the same directory.
- **The release** (`.github/workflows/package.yml`, job `kernels`): macOS (Xcode 26's Metal Toolchain), Linux
  (NVRTC 13 and 12 from NVIDIA's wheels; ROCm's `hip-runtime-amd`, `comgr` and `rocm-device-libs`) and Windows
  (NVRTC), each on the CPU device. The package job puts them in `native/rerail-kernels/gpu/`, checks that each
  backend's targets are there, and `package.json` packs the directory. AMD's on Windows are compiled where they run.
- **Measured:**
  - Loaded and run, each target the device runs, from a kernel compiled without the device
    (`a_kernel_shipped_for_a_target_the_device_runs_is_loaded`): on the M5 Max `apple10` and `apple7`; on a GB10
    `sm_121`, `sm_120` and `compute_80`; `compute_70`'s text is not the GB10's (sm_70 has no `__reduce_add_sync`),
    and is not taken; on the Radeon 680M `gfx1035` by HIP.
  - End to end on a GB10: `kernels ship --targets sm_121,sm_80,compute_70 --only join` compiled the joins' 6
    programs for each; a run with an empty store then loaded all 6 and compiled none (`join_run`: 10.6 s compiled,
    0.12 s loaded).
  - Shipped from the CPU device, as the release does: all 20 programs a GB10's CUDA device asked for loaded, none
    compiled.
  - On the M5 Max: `--targets all` shipped the device layer's programs for both tiers, and `accel`'s relational
    library as AIR for both (a split library ships no AIR: decision 0571).
  - Size: about 12 MB a CUDA target (11.7 MB sm_121, 11.2 MB sm_70, 12.8 MB compute_80; 2 MB a target
    compressed), so about 130 MB for CUDA's 11. Metal's two tiers: 2.0 MB each, 2.4 MB compressed together.
    AMD's 18 targets, shipped from Linux: 60 MB (2.9 to 3.5 MB a target, 20 programs each), 19.6 MB compressed.

### Checked against the release's inputs (2026-11-23)

- **The V100:** on a Tesla V100 (sm_70), a kernel compiled for `sm_70` and `compute_70` without the GPU is loaded
  and run, with CUDA 12.9 (`a_kernel_shipped_for_a_target_the_device_runs_is_loaded`; the matrix tests pass
  there). With only NVRTC 13.5 in reach (NVIDIA's wheel), the device is refused and the user is told: "This
  machine's NVIDIA GPU is sm_70, and its CUDA Toolkit's NVRTC (13.5) compiles for sm_75 to sm_121, not for it ...
  Install an older CUDA Toolkit that still compiles for it (12.x for sm_70, the Tesla V100)".
- **CUDA's kernels compiled on Windows** from NVIDIA's wheels (13.5 and 12.9, unpacked without Python), which
  found two faults, fixed: the 12.x wheel holds an alternate NVRTC (`nvrtc64_120_0.alt.dll`) without its builtins
  library, and the search took it; and NVRTC loads its builtins library by name, which Windows finds only on
  `PATH`, where a wheel is not. `every_shipped_cuda_and_amd_target_compiles_without_a_gpu` compiles each CUDA
  and AMD target of the table, on Windows and on Linux.
- **The Linux job's inputs** (in an Ubuntu 24.04 container): the wheels install (`nvidia/cu13/lib/libnvrtc.so.13`,
  `nvidia/cuda_nvrtc/lib/libnvrtc.so.12`); ROCm 7.2's `hip-runtime-amd`, `comgr` and `rocm-device-libs` install
  from AMD's apt repository, with `libhiprtc.so` in `/opt/rocm/lib`; its hiprtc compiles every exact target of
  the table, and refuses the generic ones as the HIP SDK does. The macOS-15 image holds Xcode 26.0.1 to 26.3
  (16.4 its default); the job selects the newest 26.
- **AMD's kernels from Linux** had never compiled: the AMD GPU on the farm is Windows', where `uint64_t` is
  `unsigned long long`. Two faults, fixed: HIP's 64-bit intrinsics have a signed and an unsigned overload, which
  Linux's `uint64_t` matches equally (`__ffsll`, ambiguous: no kernel compiled); and `ukl_clock_ns` took PTX's
  `globaltimer` asm on HIP (the preprocessor's kernels, on every AMD target and OS). HIP reads AMD's 100 MHz
  counter (`the_device_clock_counts_nanoseconds`: 201.27 ms on the Radeon over the host's 201.78).
- **An AMD GPU hiprtc does not compile for** is asked at the device's open (one small compile, its answer kept in
  the store), and the user is told to install ROCm, or the HIP SDK on Windows, with hiprtc's reason.
- **accel's lower tier on the M5:** a library's `apple7` AIR is taken where its text is the M5's, and refused for a
  source taking the matrix header, whose `apple10` text holds the M5's unit
  (`a_lower_tiers_air_is_taken_only_for_the_same_text`).

## Open

- **The release job has not run.** Its inputs are checked one by one (above); whether the Metal Toolchain is on the
  macOS runner, or downloaded by the job, is seen at its first run.
- **The package's size:** about 130 MB of CUDA's kernels a platform, and AMD's besides: decision 0651 proposes a
  package per backend family and platform.
- **A target ships only once its tests pass on a device of it (§5):** the release ships the whole table; the farm
  has no device of most of it.
