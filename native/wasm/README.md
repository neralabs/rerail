# The engine in a browser (wasm32-wasip1)

The engine library builds for `wasm32-wasip1`, so a page can run Rerail on files a reader types: the rule
explorer beside the rule-syntax site (docs/rule-syntax). The target is WASI preview 1 because the engine
reads and writes files, keeps SQLite on disk and reads the clock; a page provides those with an in-memory
WASI filesystem in a Web Worker. No Node and no emscripten: clang, lld and wasi-libc build the C and C++.

## Build

    brew install llvm lld wasi-libc wasi-runtimes
    rustup target add wasm32-wasip1 --toolchain 1.95.0
    . native/wasm/env.sh
    cargo check -p rerail-engine --lib --target wasm32-wasip1

`env.sh` joins Homebrew's libc and libc++ sysroots under `target/wasi-sysroot` and points cc-rs at clang.
It also links that libc (newer than the copy Rust bundles) and libc++.

## One build per language, without the Node addon

A page needs one language's engine, not the addon's whole API:

    cargo build --release -p rerail-engine --bin rerail-engine --target wasm32-wasip1 \
      --no-default-features --features headless,lang-python

- `headless` compiles `#[napi]` items as plain Rust (napi-derive `noop`): nothing registers the addon's API, so
  the binary links only what `main` calls.
- `lang-<id>` builds a registry language (`languages/<id>.json`); all are on by default. A language left out keeps
  its tables but has no grammar: no file is routed to it (`GrammarDialect::built`), nothing embeds it, and a
  family with none of its languages built dispatches to no adapter, so neither its grammar nor its adapter is
  linked. `logic` refuses a corpus of a language that is not built.

Sizes (release, `wasm32-wasip1`, unstripped):

| build                                   | module  | gzip -9 |
|-----------------------------------------|---------|---------|
| default features (addon API registered) | 44.5 MB |         |
| `headless`, every language              | 29.4 MB |         |
| `headless,lang-python`                  | 14.0 MB | 4.7 MB  |

The Python build is about 2.7 MB of data (the Python grammar's 0.44 MB among it) and 9.9 MB of code.

## Run

    cargo build -p rerail-engine --bin rerail-engine --target wasm32-wasip1
    wasmtime run -W exceptions=y -W unknown-imports-trap=y --dir .::/w --env RERAIL_TAPE_CACHE=off \
      target/wasm32-wasip1/debug/rerail-engine.wasm logic /w/rule.rr /w/src /w/out

The outputs land in `out` as for a native run (`<relation>.csv`, `store.sqlite`). `exceptions` is for libc++'s
unwinding; `unknown-imports-trap` stubs the few `napi_*` imports still linked (napi types' drops), which a
CLI run never calls.
`RERAIL_TAPE_CACHE` names the tape store, since WASI has no temporary directory. A rule as written in the
library (`#include "python/…"`) runs too: mount `docs/rule-syntax/lib` and name it in `RERAIL_RR_LIB`
(`--dir docs/rule-syntax/lib::/lib --env RERAIL_RR_LIB=/lib`); `logic` flattens its includes (`tools::flatten`).

## What differs from a native build

- **No JIT.** A page has no executable memory. `cranelift-jit` is replaced on wasm by `wasm/cranelift-jit`,
  whose `JITBuilder::new` fails, so the transducer interprets (as it does natively until a background JIT
  compile lands).
- **No Metal, no jemalloc.** The system allocator runs (as on Windows); heap counters read 0.
- **One thread.** rayon runs its jobs on the calling thread. The corpus runner builds every chunk before it evaluates
  them (`corpus::spawn_chunks`), so the whole input is in memory at once. CPU time is the thread's monotonic time
  (`prof::single_thread_cpu`): a worker runs nothing else.
- **32-bit ids.** Node ids of sub-trees use bit 31 (`literal::ID_FLAG`) and leave 7 bits of tree counter;
  annotation ranges use bit 30 with 16-bit offsets (`scope::ANNOTATION_RANGE`). Enough for an explorer's
  files, not for a corpus.
- **No file locks.** The content store's lease is always free (`atoms::try_lock`): one process.
- **No Node addon** with `headless` (above): 44.5 MB → 29.4 MB release, code 23.4 → 10.6 MB.
- **Left out:** `facts::ygraph`, an exploration tool that forks and reads macOS counters.
- tree-sitter's `Language` is `Send` and `Sync` here too (`native/vendor/tree-sitter-0.27.1/PATCHES.md`).
