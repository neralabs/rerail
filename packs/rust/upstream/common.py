"""Shared by packs/rust/upstream/clippy/generate.py and fixtures/rust/suppress/generate_corpus.py: the
pinned toolchain, the Rust pack's clippy lints (packs/rust/conformance.json), the pinned corpora and the
`cargo clippy` corpus run fixtures/rust/suppress records, and the expected-findings format
`rerail/conformance/languages/pack.ts` reads (`expectedFindingsSchema`).

Pins (each generator refuses anything else):
- Rust toolchain 1.99.0: `rustc 1.99.0 (b940084d7 2026-09-28)`, `clippy 0.1.99 (b940084d7e 2026-09-28)`, invoked as
  `rustc +1.99.0` / `cargo +1.99.0 clippy` / `<sysroot>/bin/clippy-driver`.
- Clippy source: https://github.com/rust-lang/rust-clippy at tag `rust-1.99.0`,
  commit 49e2f89a2605a9a340e04955fdbefa9803e6027a.
- Corpora (`CORPORA`): ripgrep 3fce3b5bb0236da2df6d99672afb8a719642eca7,
  tokio b2636752450484955e7ad334bac678424d51bc4a. tokio commits no Cargo.lock: its lock is cargo's resolution of the
  crates published by 2026-10-04T00:00:00Z (`CORPUS_LOCK_PUBLISH_TIMES`), which `run_corpus` writes before it runs.
Python 3.9 standard library only."""

from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
PACK = HERE.parent / "conformance.json"
sys.path.insert(0, str(HERE.parents[2] / "scripts" / "s3"))
import pack_fixtures  # noqa: E402

TOOLCHAIN = "1.99.0"
RUSTC_VERSION = "rustc 1.99.0 (b940084d7 2026-09-28)"
CLIPPY_VERSION = "clippy 0.1.99 (b940084d7e 2026-09-28)"
CLIPPY_COMMIT = "49e2f89a2605a9a340e04955fdbefa9803e6027a"
CLIPPY_ORIGIN = f"https://github.com/rust-lang/rust-clippy@{CLIPPY_COMMIT}"
# The corpora fixtures/rust/suppress/generate_corpus.py records: each one's repository, commit and the paths scored.
CORPORA = {
    "ripgrep": {"repository": "https://github.com/BurntSushi/ripgrep",
                "commit": "3fce3b5bb0236da2df6d99672afb8a719642eca7", "paths": ["."]},
    "tokio": {"repository": "https://github.com/tokio-rs/tokio",
              "commit": "b2636752450484955e7ad334bac678424d51bc4a", "paths": ["."]},
}
# Corpora that commit no Cargo.lock: `cargo +1.99.0 generate-lockfile --publish-time <time>` resolves their
# dependencies to what crates.io held then (unstable in cargo 1.99, so RUSTC_BOOTSTRAP=1 and -Zunstable-options),
# the lock the expected findings were made with.
CORPUS_LOCK_PUBLISH_TIMES = {
    "tokio": "2026-10-04T00:00:00Z",
}
TOOL = "clippy"


def pack() -> dict:
    return json.loads(PACK.read_text())


def tool_version() -> str:
    """The clippy version the pack pins (`tools[].version`), which every expected-findings file carries."""
    for entry in pack()["tools"]:
        if entry["name"] == TOOL:
            return entry["version"]
    raise SystemExit(f"{PACK} pins no {TOOL}")


def lints() -> list[str]:
    """The clippy lints the pack ports, without the `clippy::` prefix (a lint ported as several rules once)."""
    return sorted({rule["upstream"]["rule"] for rule in pack()["rules"] if rule["upstream"]["tool"] == TOOL})


def check_toolchain() -> pathlib.Path:
    """Refuses any toolchain but the pinned one; returns the pinned clippy-driver."""
    def output(*command: str) -> str:
        return subprocess.run(command, capture_output=True, text=True, check=True).stdout.strip()
    if sys.version_info[:2] < (3, 9):
        raise SystemExit("Python 3.9 or newer is required")
    found = output("rustc", f"+{TOOLCHAIN}", "--version")
    if found != RUSTC_VERSION:
        raise SystemExit(f"rustc +{TOOLCHAIN} is `{found}`, the generators pin `{RUSTC_VERSION}`")
    found = output("cargo", f"+{TOOLCHAIN}", "clippy", "--version")
    if found != CLIPPY_VERSION:
        raise SystemExit(f"cargo +{TOOLCHAIN} clippy is `{found}`, the generators pin `{CLIPPY_VERSION}`")
    driver = pathlib.Path(output("rustc", f"+{TOOLCHAIN}", "--print", "sysroot")) / "bin" / "clippy-driver"
    found = output(str(driver), "--version")
    if found != CLIPPY_VERSION:
        raise SystemExit(f"{driver} is `{found}`, the generators pin `{CLIPPY_VERSION}`")
    if CLIPPY_VERSION.split()[1] != tool_version():
        raise SystemExit(f"{PACK} pins clippy {tool_version()}, the generators {CLIPPY_VERSION}")
    return driver


def check_head(root: pathlib.Path, commit: str) -> None:
    head = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True,
                          check=True).stdout.strip()
    if head != commit:
        raise SystemExit(f"{root} is at {head}, the generators pin {commit}")


def corpus(name: str) -> dict:
    """The pinned corpus `name` (`CORPORA`): repository, commit, paths."""
    if name not in CORPORA:
        raise SystemExit(f"{name} is not a corpus common.py pins")
    return CORPORA[name]


def corpus_files(root: pathlib.Path, name: str) -> list[str]:
    """Corpus-relative `.rs` files under the corpus's paths (`.git` and `target` directories excluded), checked out at
    the pinned commit."""
    declared = corpus(name)
    check_head(root, declared["commit"])
    files = set()
    for path in declared["paths"]:
        for file in (root / path).rglob("*.rs"):
            relative = file.relative_to(root)
            if file.is_file() and not {".git", "target"} & set(relative.parts):
                files.add(relative.as_posix())
    return sorted(files)


def relative_path(file_name: str, root: pathlib.Path) -> str | None:
    """`file_name` (as rustc or a dep-info file names it, relative to the workspace root or absolute) relative to
    the corpus root, or None outside it."""
    path = pathlib.Path(file_name)
    if not path.is_absolute():
        path = root / path
    path = pathlib.Path(os.path.normpath(str(path)))
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return None


def run_corpus(root: pathlib.Path, name: str, target_dir: pathlib.Path, level: str) -> dict:
    """`cargo +1.99.0 clippy --keep-going --locked --workspace --all-targets --all-features --message-format=json
    -- <level> clippy::<lint>...` over corpus `name` (RUSTFLAGS untouched, CARGO_TARGET_DIR outside the corpus), so
    no tokio_unstable or other `--cfg`. Returns the pack lints' findings (one per distinct primary span, start and end,
    so the units that compile a file twice report it once; primary span inside the corpus), the corpus files no
    compiled unit read (from the units' dep-info),
    the units that failed to compile, and the findings outside the corpus that were dropped."""
    declared = corpus(name)
    check_head(root, declared["commit"])
    if root.resolve() in target_dir.resolve().parents or root.resolve() == target_dir.resolve():
        raise SystemExit(f"{target_dir} is inside the corpus {root}")
    publish_time = CORPUS_LOCK_PUBLISH_TIMES.get(name)
    if publish_time is not None:
        subprocess.run(["cargo", f"+{TOOLCHAIN}", "generate-lockfile", "-Zunstable-options", "--publish-time",
                        publish_time], cwd=root, env=dict(os.environ, RUSTC_BOOTSTRAP="1"), check=True)
    wanted = lints()
    arguments = []
    for lint in wanted:
        arguments += [level, f"clippy::{lint}"]
    environment = dict(os.environ, CARGO_TARGET_DIR=str(target_dir), CARGO_TERM_COLOR="never")
    run = subprocess.run(["cargo", f"+{TOOLCHAIN}", "clippy", "--keep-going", "--locked", "--workspace",
                          "--all-targets", "--all-features", "--message-format=json", "--", *arguments],
                         cwd=root, env=environment, capture_output=True, text=True)
    findings = {}
    outside = set()
    failures = set()
    read = set()
    for line in run.stdout.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        manifest = message.get("manifest_path")
        if manifest is None or relative_path(manifest, root) is None:
            continue
        if message["reason"] == "compiler-artifact":
            for artifact in message["filenames"]:
                read |= dep_info_sources(pathlib.Path(artifact), root)
            continue
        if message["reason"] != "compiler-message":
            continue
        diagnostic = message["message"]
        code = (diagnostic.get("code") or {}).get("code") or ""
        if diagnostic["level"] == "error" and not code.startswith("clippy::") \
                and not diagnostic["message"].startswith("aborting due to"):
            target = message["target"]
            failures.add(f"{relative_path(manifest, root)} {'/'.join(target['kind'])} {target['name']}: "
                         f"{diagnostic['message']}")
        if not code.startswith("clippy::") or code[len("clippy::"):] not in wanted:
            continue
        rule = code[len("clippy::"):]
        primary = next(span for span in diagnostic["spans"] if span["is_primary"])
        path = relative_path(primary["file_name"], root)
        key = (rule, path, primary["line_start"], primary["column_start"], primary["line_end"], primary["column_end"])
        if path is None:
            outside.add((rule, primary["file_name"], primary["line_start"], primary["column_start"]))
            continue
        findings[key] = {"rule": rule, "path": path, "line": primary["line_start"],
                         "column": primary["column_start"], "expansion": primary.get("expansion") is not None}
    if run.returncode != 0 and not failures:
        sys.stderr.write(run.stderr)
        raise SystemExit(f"cargo clippy failed on {name} with status {run.returncode} and no compile error")
    files = corpus_files(root, name)
    return {"findings": [findings[key] for key in sorted(findings)],
            "skipped": sorted(set(files) - read), "failures": sorted(failures), "outside": sorted(outside),
            "files": files}


def dep_info_sources(artifact: pathlib.Path, root: pathlib.Path) -> set[str]:
    """The corpus-relative `.rs` files the dep-info beside `artifact` lists: `deps/lib<crate>-<hash>.rmeta` (and
    `.rlib`, `.dylib`, executables) read `deps/<crate>-<hash>.d`; a build script `build/<pkg>-<hash>/build-script-build`
    reads `build/<pkg>-<hash>/build_script_build-<hash>.d`."""
    if artifact.name.startswith("build-script-"):
        dep_info = artifact.parent / f"build_script_{artifact.name[len('build-script-'):].replace('-', '_')}-" \
                                     f"{artifact.parent.name.rsplit('-', 1)[1]}.d"
    else:
        stem = artifact.name.split(".")[0]
        if stem.startswith("lib") and "." in artifact.name:
            stem = stem[len("lib"):]
        dep_info = artifact.parent / f"{stem}.d"
    if not dep_info.exists():
        raise SystemExit(f"no dep-info {dep_info} for {artifact}")
    sources = set()
    for line in dep_info.read_text().splitlines():
        if not line or line.startswith("#") or ":" not in line:
            continue
        head, _, tail = line.partition(": ")
        if head.endswith(".d"):
            for name in re.split(r"(?<!\\) ", tail.strip()):
                path = relative_path(name.replace("\\ ", " "), root) if name.endswith(".rs") else None
                if path is not None:
                    sources.add(path)
    return sources


def corpus_source(name: str, rules: list[str], skipped: list[str]) -> dict:
    declared = corpus(name)
    return {"kind": "corpus", "name": name, "repository": declared["repository"], "commit": declared["commit"],
            "rules": sorted(rules), "skipped": sorted(skipped)}


def write(path: pathlib.Path, source: dict, findings: list[dict]) -> None:
    findings = sorted(({key: finding[key] for key in ("rule", "path", "line", "column")} for finding in findings),
                      key=lambda f: (f["rule"], f["path"], f["line"], f["column"]))
    path.write_text(json.dumps({"tool": TOOL, "version": tool_version(), "source": source, "findings": findings},
                               indent=1, ensure_ascii=False) + "\n")
    pack_fixtures.expected_findings_written(path, source)
