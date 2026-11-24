# 0651. GPU kernels ship as npm packages, one per backend family and platform

Status: proposed (2026-11-23). The user asked: "For the GPU packaging, we will need a separate npm package to be built
for each of the GPU backends, and possibly per family, so that we can ship everything needed there without busting
npm's limits. Please take a look at how we can separate this work."

It follows decision 0645, which prepares kernels for named targets (`rerail_ukl::Target`) and ships them under
`rerail-kernels/gpu/<target>/`. That release step puts every target into the one `@neralabs/rerail` package. This
record splits them out.

## 1. Sizes, measured

The release's sets, each shipped from the CPU device (`RERAIL_GPU=cpu`, decision 0645 §5):

| Set | Targets | On disk | Compressed (tar.gz) |
|---|---|---|---|
| Metal | apple7, apple10 | 4.1 MB (2.0 MB each) | 2.4 MB |
| CUDA, one OS | 11 (sm_70 … sm_121, compute_70, compute_80) | about 130 MB (about 12 MB a target) | about 22 MB |
| AMD, one OS | 18 (gfx1030 … gfx1201, gfx90a, gfx942, gfx950) | 60 MB (2.9 to 3.5 MB a target) | 19.6 MB |

The CUDA and AMD sets differ by OS: Windows' `long` is 32 bits, so the text, and therefore the key, differs
(`long_is_32`). Linux on arm64 and Linux on x86_64 share their text.

npm documents no tarball limit. The limit npm is known to enforce is 100 MB on a package's metadata document (the
packument), which grows with each version published. Every package below is kept under 50 MB unpacked, so a
package stays far from any limit a registry or proxy applies (a mirror's 100 MiB file cap, for one).

## 2. The packages

One package per backend family and platform. A family's targets are the ones one generation of GPUs runs
(`Target::runs`), so a machine needs exactly one kernel package:

| Package | Platforms (npm's `os` and `cpu` fields: linux, darwin, and Windows' own value) | Targets |
|---|---|---|
| `@neralabs/rerail-kernels-metal` | darwin, arm64 | apple7, apple10 |
| `@neralabs/rerail-kernels-cuda-sm7-<os>` | linux (x64, arm64), windows x64 | sm_70, sm_75, compute_70 |
| `@neralabs/rerail-kernels-cuda-sm8-<os>` | linux (x64, arm64), windows x64 | sm_80, sm_86, sm_89, compute_80 |
| `@neralabs/rerail-kernels-cuda-sm9-12-linux` | linux (x64, arm64) | sm_90, sm_100, sm_120, sm_121 |
| `@neralabs/rerail-kernels-cuda-sm12-windows` | windows x64 | sm_120 |
| `@neralabs/rerail-kernels-amd-rdna2-<os>` | linux x64, windows x64 | gfx1030 … gfx1036 |
| `@neralabs/rerail-kernels-amd-rdna3-<os>` | linux x64, windows x64 | gfx1100 … gfx1103, gfx1150, gfx1151 |
| `@neralabs/rerail-kernels-amd-rdna4-<os>` | linux x64, windows x64 | gfx1200, gfx1201 |
| `@neralabs/rerail-kernels-amd-cdna-linux` | linux x64 | gfx90a, gfx942, gfx950 |

The AMD families, measured on Linux: RDNA2 24.6 MB (7.9 MB compressed), RDNA3 18.9 MB (6.4 MB), RDNA4 7.1 MB
(2.2 MB), CDNA 9.5 MB (3.1 MB). The CUDA families, at about 12 MB a target: 35, 47 and 47 MB.

The data-centre and DGX parts (sm_90, sm_100, sm_121, CDNA) run Linux only, so they have no Windows package.

A GPU newer than the table takes PTX: `runs()` for an `sm_130` is `[sm_130, compute_80, compute_70]`, and
`compute_80` lives in the `sm8` package. Which package a device needs is the first package holding any target of
its `runs()`, in that order. Each package holds only `gpu/<target>/` for its targets: a family is a directory
subset, with nothing else changed.

## 3. One table, in the engine

The table above is data in `rerail_ukl::target`, next to `SHIPPED`: `PACKAGES: &[(&str, &[&str] /* os */, &[&str] /*
targets */)]`. `SHIPPED` becomes the union of its targets, so there is one definition. A test checks that every
shipped target is in exactly one package per OS, and that every target's `runs()` reaches a package.

- `rerail-engine kernels ship <dir> --package <name>` ships that package's targets, writes its `package.json`
  (name, the release's exact version, `os`, `cpu`, `files: ["gpu"]`), and refuses a package over 50 MB.
- The engine's `health` (the Node binding) reports the package this machine's GPU needs, from its device's
  `Target::runs()`, or none (no GPU, OpenCL, the CPU).

## 4. How a machine gets its package

npm's `optionalDependencies` select by `os` and `cpu` only. npm has no field for a GPU, so listing the kernel
packages there would install every CUDA family on every Linux PC, with or without a GPU. So the packages are not
dependencies of `@neralabs/rerail`.

`rerail setup` installs the one package the GPU needs, the way it installs rule packs (`src/setup/rule-pack-store.ts`):

- It installs into the Rerail home (`<home>/kernels`), with `npm install --prefix`, `--ignore-scripts` and the
  same registry-signature check (`src/setup/updater.ts`).
- The version is exactly the engine's, because the shipped key holds the kernel's text. A package of another
  version only misses (decision 0645 §4), so it is never wrong, only slower.
- Setup reruns it on an update, and when the GPU changes: `health` names another package.
- Without the package, Rerail runs as it does now. It compiles each kernel at first use, with the toolkit the user
  installed (decision 0645 §5).

The engine finds the package's directory. `RERAIL_SHIPPED_KERNELS` takes a list of directories (the OS's path
separator), and `store::shipped_dir` becomes `shipped_dirs`. The Node side resolves the installed package's
directory in the Rerail home and sets the variable before it loads the addon (`src/engine/native/native.ts`,
`loadNativeBinding`). `rerail-kernels/` beside the addon stays first, for a build that ships its kernels inside.

## 5. The release

- The `kernels` job's matrix is one entry per package. It runs on the runner whose OS's text the package needs
  (macos-15 for Metal; ubuntu-24.04 for Linux CUDA and AMD; windows-2025 for Windows CUDA; Windows AMD needs
  the HIP SDK's hiprtc on the Windows runner). Each entry runs `kernels ship --package` and uploads its package.
- A package a job also spreads the compiles. hiprtc is slow on the preprocessor's large program: shipping all 18
  AMD targets from one 8-core Linux machine took 20 minutes. A family's job compiles only its own targets.
- The publish job publishes each kernel package before `@neralabs/rerail`, with the same version and provenance
  (`docs/release.md`), so a published `@neralabs/rerail` never names a missing kernel package.
- `@neralabs/rerail` drops `native/rerail-kernels` from its `files` (decision 0645's step 4 put it there).

## 6. What this changes in 0645

- §3 (the release ships the targets) becomes packages. `SHIPPED` is derived from `PACKAGES`.
- §5 still holds: the user installs the GPU compiler. With the kernel package installed, a machine compiles only
  the kernels the package does not hold, such as the rule-generated ones. Whether a device opens without a
  compiler when its package covers every fixed kernel is an open question (§7).

## 7. Open

- **A device with its package and no compiler.** Today a device's open requires the compiler (NVRTC's
  architectures, hiprtc's probe). With the package, the fixed kernels need no compiler. The kernels generated from
  rules still do. Opening without one would run the fixed kernels on the GPU and the generated ones on the CPU.
- **Windows AMD.** The HIP SDK has no GitHub runner image. The Windows AMD packages need the SDK installed in the job
  (a silent install of the HIP SDK), which is not yet tried.
