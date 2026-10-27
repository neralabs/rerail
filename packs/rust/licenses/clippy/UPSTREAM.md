# rust-clippy (Rust 1.99.0)

License: MIT OR Apache-2.0 (Copyright (c) The Rust Project Contributors); [`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE) and [`COPYRIGHT`](COPYRIGHT) are the upstream license files, copied verbatim at the commit below.

## Source

| Repository | Tag | Commit | Licence |
|---|---|---|---|
| https://github.com/rust-lang/rust-clippy | `rust-1.99.0` | `49e2f89a2605a9a340e04955fdbefa9803e6027a` | MIT OR Apache-2.0 |

## Taken from it

| Pack paths | Upstream paths | How |
|---|---|---|
| `upstream/clippy/fixtures/**` | `tests/**` | verbatim: each file keeps its upstream path below `upstream/clippy/fixtures/` |

The files taken from it are UI tests of the clippy lints the Rust pack ports (`packs/rust/conformance.json`), the auxiliary crates those tests build, and the `clippy.toml` files they read. They remain under clippy's license. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of clippy's lints, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. Every other file under `upstream/clippy/` is Rerail's, under the Apache License 2.0 (this repository's `LICENSE`; `upstream/clippy/UPSTREAM.md` lists the cases under `fixtures/rerail/`): `generate.py`, `../common.py` and the expected findings they write by running clippy (`fixtures.expected.json`, `fixtures.upstream-check.json`), and the upstream manifest `gates.yaml`.

Paths are relative to the pack directory, `packs/rust/`. The test files are in no package and not in git: the tool-pack's S3 archive (`upstream/clippy/test-data.lock`) holds them, with `provenance.json` (every file, its upstream path and how it was made, a snapshot of the per-file record this file kept until commit `cbf682862a47`) and `LICENSES.md` (this licence text). The files Rerail wrote among them are listed in `upstream/clippy/UPSTREAM.md` and are Rerail's, as are the expected findings recorded by running the tool.
