"""Writes corpus-<name>.expected.json: every site where clippy 0.1.99 (Rust 1.99.0) reports one of the Rust pack's
ported lints (fixtures/rust/upstream/conformance.draft.json) in a pinned corpus, and whether the corpus suppresses
it. Run offline, never in CI, with the toolchain upstream/common.py pins (it refuses any other; Python 3.9 standard
library only):

    python3 generate_corpus.py <cargo target root> <name>=<corpus checkout> [...]

Pins: Rust 1.99.0 (`rustc 1.99.0 (b940084d7 2026-09-28)`, `clippy 0.1.99 (b940084d7e 2026-09-28)`), rust-clippy
49e2f89a2605a9a340e04955fdbefa9803e6027a, ripgrep 3fce3b5bb0236da2df6d99672afb8a719642eca7, tokio
b2636752450484955e7ad334bac678424d51bc4a.

Each corpus is checked twice with `cargo +1.99.0 clippy --keep-going --locked --workspace --all-targets
--all-features --message-format=json` (CARGO_TARGET_DIR=<cargo target root>/clippy-<name>, RUSTFLAGS untouched):
once with `-W clippy::<lint>` for every ported lint, as upstream/clippy/corpus-<name>.expected.json records, and once
with `--force-warn clippy::<lint>`, which no in-source `allow` / `expect` (nor crate-level setting) can lower. A site
reported only under `--force-warn` is `suppressed: true`; one reported under both is `suppressed: false`. Sites are
the lint, the corpus-relative path and the primary span's 1-based line and column (code points), deduplicated;
`source.skipped` lists the corpus `.rs` files no compiled unit read in the `-W` run."""

from __future__ import annotations

import json
import pathlib
import sys
from collections import Counter

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "upstream"))
import common  # noqa: E402


def record(target_root: pathlib.Path, name: str, root: pathlib.Path) -> None:
    common.check_toolchain()
    target_dir = target_root / f"clippy-{name}"
    warned = common.run_corpus(root, name, target_dir, "-W")
    forced = common.run_corpus(root, name, target_dir, "--force-warn")
    key = lambda finding: (finding["rule"], finding["path"], finding["line"], finding["column"])  # noqa: E731
    reported = {key(finding) for finding in warned["findings"]}
    everywhere = {key(finding) for finding in forced["findings"]}
    if not reported <= everywhere:
        raise SystemExit(f"{name}: reported under -W but not under --force-warn: {sorted(reported - everywhere)}")
    sites = [{"rule": rule, "path": path, "line": line, "column": column, "suppressed": (rule, path, line, column)
              not in reported} for rule, path, line, column in sorted(everywhere)]
    lints = common.lints()
    (HERE / f"corpus-{name}.expected.json").write_text(json.dumps(
        {"tool": common.TOOL, "version": common.tool_version(),
         "source": common.corpus_source(name, lints, warned["skipped"]), "sites": sites},
        indent=1, ensure_ascii=False) + "\n")
    suppressed = Counter(site["rule"] for site in sites if site["suppressed"])
    print(f"{name}: {len(sites)} sites, {sum(suppressed.values())} suppressed: {dict(sorted(suppressed.items()))}")
    for failure in sorted(set(warned["failures"]) | set(forced["failures"])):
        print(f"  failed to compile: {failure}")


def main() -> None:
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    target_root = pathlib.Path(sys.argv[1]).resolve()
    for argument in sys.argv[2:]:
        name, _, path = argument.partition("=")
        record(target_root, name, pathlib.Path(path).resolve())


if __name__ == "__main__":
    main()
