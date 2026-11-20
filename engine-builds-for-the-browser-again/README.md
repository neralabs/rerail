# The engine builds for the browser again, with two build fixes

For [pull request #912](https://github.com/neralabs/rerail/pull/912), head b681e3a1e. [report.html](report.html) is the same report as one page: download it and open it.

## 1. The decision

**May the browser build leave out `rerail-clif`'s JIT of the reads, and may CI check that build with wasi-sdk?**

**We recommend:** approve it: the engine builds for the browser again and CI checks it, a second checkout builds its own sources, and `rerail-blocks` builds alone on Windows.

| Size | Risk and review | Waits on | Not verified |
|---|---|---|---|
| 134 lines in 8 files: 65 ship, 62 run in every test run, 7 are tests and text | Standard. Touches the CI workflow: a person approves. | Nothing. No CI run on GitHub yet: not opened. | 2 |

## 2. Intent, and the gap

| Change | Intended | Found | Now |
|---|---|---|---|
| A. The browser build | The engine builds for wasm32 _(docs)_ | It fails in `region` _(one check per commit)_ | Closes the gap. |
| B. A shared target folder | Each checkout builds its own sources _(our reading)_ | The second builds the first's _(the report's four steps)_ | Closes the gap. |
| C. One crate on Windows | The crate builds on Windows _(code comment)_ | Alone, it fails with E0432 _(one build per crate)_ | Closes the gap. |

Found on 19 November 2026 on Linux x64 machines and Windows 11 virtual machines, building the engine from source.

<details><summary>Sources at main, and causes</summary>

| Change | Where main states the intent | The cause, and what was measured |
|---|---|---|
| A. The browser build | `native/wasm/README.md` at bf24c778f, its first lines: "The engine library builds for `wasm32-wasip1`, so a page can run Rerail on files a reader types: the rule explorer beside the rule-syntax site"; its Build section gives `cargo check -p rerail-engine --lib --target wasm32-wasip1`. | `rerail-clif`'s default feature `ir` takes `cranelift-jit` on every target for `rerail_clif::jit`, which only tests use. On wasm every other crate takes the stand-in `native/wasm/cranelift-jit`, so `ir` alone brought the real JIT and its `region` into the browser build. No workflow builds this target. |
| B. A shared target folder | `native/crates/simdjson_native/build.rs` at bf24c778f, lines 13 to 15: `std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shim.cpp")`, and the same for the vendored simdjson and `kernel.inc`. | `env!` fixes the folder when the build script is compiled. Cargo keeps one compiled build script per target folder, so every checkout that shares the folder runs the first checkout's copy, with that checkout's paths. |
| C. One crate on Windows | `native/crates/blocks/Cargo.toml` at bf24c778f, line 13, above its `windows-sys` line, which asks for `Win32_Foundation` and `Win32_System_Memory` only; `src/windows.rs` imports `CreateFileMappingW`. | `windows-sys` declares `CreateFileMappingW` only with `Win32_Security`, since it takes security attributes. The engine's other crates ask for that feature, and cargo unifies features in one build, so only a build without them fails. |

</details>

## 3. Where to look

<img src="map.png" width="544" alt="The changed files as bars sized by changed lines, grouped by change and coloured by risk. Code to read: A, Browser build, and its check, low risk, 75 lines; B, Build scripts read their folder, low risk, 47 lines; C, One crate alone on Windows, low risk, 5 lines. Read first: clif/Cargo.toml, simdjson_native/build.rs, blocks/Cargo.toml. To skim: 0 lines of tests and 7 lines of how to build on linux.">

1. **Read first**, 5 min: `clif/Cargo.toml`: the `ir` feature and the target table. Does any native build lose `cranelift-jit`, or any wasm build still take it?
2. **Read first**, 2 min: `simdjson_native/build.rs`: `main`, where the paths are read. Does every path come from the folder cargo gives this run?
3. **Read first**, 1 min: `blocks/Cargo.toml`: the `windows-sys` features. Does the crate ask for every feature its imports need?
4. 5 min: `lib.rs`, `env.sh`, `ci.yml` and `native/build.rs`: the `jit` gate, the wasi-sdk path, the new step, one rerun line.

**Then skim**, 2 min: The README's new paragraph: how to build on Linux, and that CI checks the build.

## 4. Risk against verification

<img src="grid.png" width="544" alt="The changes by risk, high on top, against six checks. A, Browser build, low risk: independent review holds with a limit. B, Build scripts, low risk: independent review holds with a limit, Windows 11: no new failure holds with a limit. C, Windows crate, low risk: independent review holds with a limit. CI at this head holds with a limit for every change.">

Three small changes, low risk: start at the manifest, where the JIT leaves the browser build.

<details><summary>Each half mark's limit</summary>

| Change | Check | It holds with this limit |
|---|---|---|
| A. Browser build | independent review | No review by another reader yet. The automated reviewer reads it once the pull request opens. |
| B. Build scripts | independent review | No review by another reader yet. The automated reviewer reads it once the pull request opens. |
| B. Build scripts | Windows 11: no new failure | The shim's crate is empty on Windows with MSVC: nothing of it runs there. The engine's build script ran in every Windows build here. |
| C. Windows crate | independent review | No review by another reader yet. The automated reviewer reads it once the pull request opens. |
| A to C | CI at this head | No CI run at this head on GitHub yet. The new step ran as written on a Linux x64 machine. |

</details>

<details><summary><b>5. The changes</b></summary>

**A. Browser build, and its check**, low risk. The browser check: 10 errors at `main`, none here.

- **Problem:** The browser build the README gives, and the rule explorer's build, stop at 10 errors in `region`, which has no wasm32 code.
- **Who meets it:** Anyone building the rule explorer or a browser build from source at `main`, every time. No published version has a browser build.
- **What it does:** `rerail-clif` takes `cranelift-jit` off wasm only and leaves `jit` out there. `env.sh` takes a wasi-sdk named by `WASI_SDK_PATH`. CI's fmt and clippy job checks the browser build, and the explorer's one-language build.
- **Risk:** Low: off wasm the manifest resolves as before, and the Homebrew path of `env.sh` is unchanged.
- **Check:** Does any native build lose `cranelift-jit`, or any wasm build still take it? Is the tarball's digest checked before it is used?
- **Picture:** Errors in the browser check: 10 at main, 0 with this change.
- **Method:** The CI step as written, in a new checkout with an empty target folder, once at `main` and once here, on Linux x64.

**B. Build scripts read their folder**, low risk. Builds in a second checkout that compile its own shim: 0 of 2 at `main`, 2 of 2 here.

- **Problem:** With one target folder for two checkouts, the second compiles the first checkout's `shim.cpp`, and fails once the first is removed.
- **Who meets it:** Anyone building two checkouts with one cargo target folder, at each build after the first checkout's. No published version has the shim.
- **What it does:** The simdjson build script reads `CARGO_MANIFEST_DIR` when it runs, not as `env!` fixed it at its compile. The engine's build script names `RERAIL_FARM_CONFIG`, which it reads, to cargo.
- **Risk:** Low: in a checkout of its own the paths are the same as before.
- **Check:** Does every path the script reads come from the folder cargo gives this run?
- **Picture:** Builds in the second checkout that compile its own sources, of the report's two: none at main, both with this change.
- **Method:** The four steps of #901 as written, once at `main` and once here, on Linux x64.

**C. One crate alone on Windows**, low risk. Crates that fail alone on Windows 11 for want of the feature: 4 at `main`, none here.

- **Problem:** `rerail-blocks` imports `CreateFileMappingW`, which `windows-sys` declares only with `Win32_Security`, so it fails to build alone on Windows, and so does every crate built on it alone.
- **Who meets it:** Anyone building one engine crate alone on Windows, such as `cargo test -p rerail-clif`. No published version has the crate.
- **What it does:** `rerail-blocks` asks `windows-sys` for `Win32_Security` itself.
- **Risk:** Low: the engine's other crates ask for the feature, so the engine's build had it already.
- **Check:** Does `rerail-blocks` now ask for every feature its imports need?
- **Picture:** Crates under native/crates that fail cargo check alone on Windows 11 for want of the feature: 4 at main, none with this change.
- **Method:** `cargo check -p <crate> --all-targets` for each crate under `native/crates`, alone, 4 jobs, on Windows 11 and Linux x64.

</details>

<details><summary><b>6. The decisions</b></summary>

**Should the browser build leave out the reads' JIT, or take the stand-in?**

**We chose** to leave it out: it only serves tests, which do not run on wasm. **Would change our mind:** A maintainer who wants every crate to follow one pattern for the JIT on wasm.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Leave `jit` out on wasm **(chosen)** | The browser build; nothing on wasm that could only fail there; `Cargo.lock` unchanged. | Two lines of manifest and one `cfg`. | `rerail-clif` differs from the other crates, which take the stand-in. |
| The stand-in in `rerail-clif`, as the other crates | The browser build, in the same form as the other crates. | A new dependency and a change to `Cargo.lock`. | `jit::Reads::new` would compile on wasm and panic at its first call. |

**Evidence.** `rerail_clif::jit` is used by `rerail-clif`'s and `rerail-tape-format`'s tests and one engine test, never by the engine; the browser check passes here with it left out.

**Which C toolchain should CI's browser check use?**

**We chose** wasi-sdk 34, pinned by its digest: the same toolchain on every run. **Would change our mind:** A maintainer who wants CI to run the README's Homebrew steps as written.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| wasi-sdk 34, digest checked **(chosen)** | The same clang, wasi-libc and libc++ on every run; `env.sh` gains a Linux path. | A 192 MB download per run. | Nothing that runs today. |
| Homebrew on the runner | The README's steps as written. | An LLVM install per run. | Unpinned formulas can change under the check. |

**Evidence.** The step as written passed in a new checkout with an empty target folder on a Linux x64 machine, 174 s at 4 jobs, the download included.

</details>

<details><summary><b>7. How it fits</b></summary>

Nothing has to merge first, and no open pull request edits these files. #812 needs this for its browser branch to pass a check. Any order works. This one first is better: #812's and #909's next CI runs then check their code for this target.

This pull request stands on main bf24c778f, which it merges. It merges clean with #812, #883, #904 and #909, and the browser check passes merged with #812. Once it lands, the next CI runs of #812 and #909 check their code for the browser target.

**Trial merges and what merged today.** This head merges `main` bf24c778f. b00488a1d merged clean with #812, #883, #904 and #909, and this head with #812. Merged with #812's head, the browser check passes, exit 0 in 172 s. #909 edits `native/src/tree_watch/`, which the check compiles. The twelve pull requests merged today, #865 to #905, edit none of these files.

</details>

<details><summary><b>8. What is not covered</b>: 2 not verified, 2 limits that stay</summary>

1. **The browser engine** builds but still stops when it runs: the content store starts a thread for a new store's schema, and the browser has none. Its own report follows.
2. **macOS** never ran, nor `env.sh`'s Homebrew path.

**Limits that stay**

- `cargo clippy -- -D warnings` for this target stops on warnings `main` has: an unused variable in `rerail-blocks` and four in the engine. No check lints this target.
- `rerail-logic`'s example `versions` calls `libc::mmap` with no Unix gate, so `cargo check -p rerail-logic --all-targets` fails on Windows 11, at `main` as here. A dependency line cannot fix it: Windows has no `mmap`.

</details>

## 9. What we ask of you

We ask no ruling.

<details><summary><b>10. Evidence</b></summary>

The description on GitHub is the record: "How you tested it" holds the runs on `main` and here.

| Where | What ran | Result |
|---|---|---|
| Linux x64 | The new CI step as written, in a new checkout with an empty target folder, at `main` and here; then merged with #812's head | At `main` bf24c778f the engine check exits 101, 10 errors in `region`. Here exit 0 in 174 s; merged with #812, exit 0 in 172 s. |
| Linux x64 | The CI step over a break only the explorer's build compiles: here, here without its new line, and as b00488a1d wrote it | Exit 101 here, E0308 at the break; exit 0 without the line and at b00488a1d. |
| Linux x64 | #901's four steps as written, at `main` and here | At `main`, step 3 exits 0 without the second checkout's line and step 4 exits 101. Here, step 3 stops on that line and step 4 exits 0. |
| Windows 11 | `cargo test -p rerail-clif` and `-p rerail-tape-format`, at `main` and here | Exit 101 at `main`, E0432. Here 9 and 12 passed. |
| Windows 11 and Linux x64 | `cargo check -p <crate> --all-targets` for each of the 45 crates under `native/crates`, alone, at `main`; the ones that failed again here | Linux: 45 pass. Windows: 5 fail at `main`; the 4 on E0432 pass here; `rerail-logic`'s example fails at both. |
| Linux x64 | `check-rustfmt`, `npm run check:native`, CI's Lint and type-check job, `rerail-clif`, `rerail-tape-format`, the executors guard test | All exit 0: 363 files formatted, both clippy runs clean at `-D warnings`, `rerail-clif` 9 and `rerail-tape-format` 12 passed, the guard test passed. |
| Linux x64 | The engine's library tests here; the ones that failed, again at `main` | 2775 passed, 22 failed, 146 ignored; `main` fails the same 22 on the same machine, 20 of them for pack test data it lacks. |
| Linux x64 | Rerail's reviewer rules at `main` 6ec384ad9, published and built from `main`, over this head | No new finding. Over df944a0d2, two: the edited build-script functions at complexity 8 and 7, now 6 and 5. |

**Sources.** The branch at b681e3a1e and its diff against `main` bf24c778f, read on 19 November 2026; runs at it, at b00488a1d and at `main` on Linux x64 machines and Windows 11 virtual machines.

</details>
