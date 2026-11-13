# 0148: Parsers are routed by what the host runs; the native code is one crate

Status: accepted (user, 2026-11-13: "yes, we do have x86 hosts / please ensure that the
platform-specific parts are in a separate crate and that we have a parser router that allows the
different implementations to be registered per format and the most optimized one selected")

## Intro

JSON is read by simdjson's stage 1 and a stage 2 of ours, with the source's SHA-256 computed inside
stage 1 (0134 and its amendment). That code was written for arm64 alone: on an x86 host the engine fell
back to simdjson's DOM parser, then wrote the columns from its index, then hashed the source again, at
twice the cost.

The engine has x86 hosts. Now:
- **One crate holds all the native code**, and its kernel is compiled for every CPU family simdjson
  supports.
- **The engine has a parser router.** Each format registers its implementations, and the router takes
  the best one the host runs.

```
rerail-simdjson-native   simdjson (C++), the shim, the kernel: everything platform-specific
        ^
rerail-simdjson          the Tape, the index writer (the oracle): portable Rust
        ^
engine                   the router: each format's implementations, ranked; the best the host runs
```

## Prior work consulted

- **ast-index-spike indexes:** the spike parsed with tree-sitter only, one parser a language. It has no
  per-host choice of parser to compare with.
- **Lessons ledger #10** (measure the code that runs). **Agrees:** the oracle test runs every kernel the
  test's CPU runs, not only the one the router takes.
- **Lessons ledger #28** (only end-to-end CPU time counts). **Agrees:** each kernel is measured through
  the product's path (`json_parse_costs`, `tape_cost`).
- **0134** (JSON is read with simdjson). **Extended:** its stage 2 and fused hash now run on x86 too, and
  its fallback (the DOM parser and the index writer) becomes the router's last JSON entry.
- **The crate split's rule** (core defines the interfaces, and language crates register their
  implementations). **Agrees:** the router is core's (`tape::install_parsers`), and JSON's
  implementations are registered by the JSON code (`json_parser::simdjson::registrations`).

## Decision

1. **The native code is one crate**, `rerail-simdjson-native` (`native/crates/simdjson_native`). It holds
   the vendored simdjson, the shim (`shim.cpp`) and the kernel (`kernel.inc`), behind a C ABI. Nothing
   else in the repository is platform-specific for JSON. `rerail-simdjson` keeps the portable parts: the
   `Tape`, the `grow` callback, and the index writer that checks the kernels.
2. **The kernel is written once** and included in each of simdjson's implementations that the target
   compiles: arm64 on aarch64; icelake, haswell and westmere on x86-64. Each inclusion sits in a target
   region with that family's instructions, so `simd`, `stage1` and `numberparsing` resolve to that
   implementation's code. The only per-family difference is the 32-byte string check
   (`backslash_quote`), which follows the vector width: 16, 32 or 64 bytes.
3. **SHA-256 is a template parameter of stage 1,** in one of three forms:
   - ARMv8's SHA2 instructions;
   - x86's SHA-NI, which AMD Zen and Intel from Ice Lake have, but Intel's Skylake and Cascade Lake
     servers do not;
   - portable code: FIPS 180-4's rounds, unrolled.

   Each implementation is compiled twice, with its SHA instructions and with portable code, giving
   eight kernels (`Kernel::ALL`). A kernel `runs` where simdjson's run-time check accepts its
   implementation and, if it uses the SHA instructions, CPUID or HWCAP reports them.
4. **The router** (`tape::install_parsers`, `ParserImpl`):
   - each implementation is registered with its dialect, name, rank and whether the host runs it;
   - the router takes, for each dialect, the lowest rank among those that run (the first registered,
     among equals); `tree_parser` returns it and `parser_routes` lists every implementation and which one
     was taken.

   For JSON the ranks follow `Kernel::ALL`:
   - kernels with wider vectors first, and the SHA instructions before portable code;
   - then simdjson's DOM parser with the index writer, which runs on any CPU simdjson supports and hashes
     the source afterwards.
5. **There is no fallback chain between implementations.** A file the chosen one refuses (JSONC, invalid
   JSON) goes to tree-sitter-json, as before. Every simdjson route refuses the same files, so trying the
   next one would only parse a refused file twice.
6. **The tests:**
   - `every_kernels_tape_is_the_index_writers` runs every kernel the CPU runs against the index writer:
     the same tapes byte for byte, the digest equal to the `sha2` crate's, and the same refusals;
   - `the_router_takes_the_best_json_parser_this_cpu_runs` checks the router's choice, and that every
     JSON implementation the CPU runs writes the product's tape;
   - `this_cpu_runs_a_kernel` checks that a kernel with the SHA instructions never runs where its
     portable twin does not.

## Results

The repository's 2,200 `.json` files (73.6 MB, 7.2 M nodes), in instructions. On arm64 (Apple M5 Max),
natively:

| | instructions | a node |
|---|---:|---:|
| the tape, arm64 with SHA2 (the router's choice) | 0.79 G | 110 |
| the tape, arm64 with portable SHA-256 | 4.47 G | 621 |
| stage 1 with SHA2 in it | 0.34 G | 47 |
| stage 1 with portable SHA-256 in it | 2.46 G | 342 |
| the `sha2` crate, its software backend, alone | 2.16 G | 299 |

The fused portable hash costs 2.26 G over stage 1, 5% more than `sha2`'s software backend (2.16 G), and
the source is read once.

On x86-64 under Rosetta, which offers SSE4.2 but not AVX2, AVX-512 or SHA-NI, simdjson's runtime check
picks westmere:
- the westmere kernel's tapes and digests equal the oracle's on every check of the test;
- its instruction counts are Rosetta's translation and only compare with each other: the tape 7.2 G,
  stage 1 0.59 G, the `sha2` crate's software hash 5.5 G.

**Not run here:** the haswell and icelake kernels and the SHA-NI hash are compiled for every x86-64
build, but this machine cannot run them. They are checked by the same oracle test on any x86 host that
runs them, since the test runs every kernel the host runs.

## Open

- **O-x86-kernels-measured:** run the oracle and `tape_cost` on an x86 host with AVX2 and SHA-NI, for
  each kernel there.
- **O-portable-sha:** the portable hash is 5% over `sha2`'s software backend. On Intel servers without
  SHA-NI, two-block interleaving or AVX2 message scheduling may help.
