"""Shared by packs/rust/upstream/clippy/generate.py and fixtures/rust/suppress/generate_corpus.py: the
pinned toolchain, the Rust pack's corpora and clippy lints (packs/rust/conformance.json), the
`cargo clippy` corpus run, and the recording format `rerail/conformance/languages/pack.ts` reads (`recordingSchema`).

Pins (each generator refuses anything else):
- Rust toolchain 1.99.0: `rustc 1.99.0 (b940084d7 2026-09-28)`, `clippy 0.1.99 (b940084d7e 2026-09-28)`, invoked as
  `rustc +1.99.0` / `cargo +1.99.0 clippy` / `<sysroot>/bin/clippy-driver`.
- Clippy source: https://github.com/rust-lang/rust-clippy at tag `rust-1.99.0`,
  commit 49e2f89a2605a9a340e04955fdbefa9803e6027a.
- Corpora (from conformance.json): ripgrep 3fce3b5bb0236da2df6d99672afb8a719642eca7,
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
CORPUS_COMMITS = {
    "ripgrep": "3fce3b5bb0236da2df6d99672afb8a719642eca7",
    "tokio": "b2636752450484955e7ad334bac678424d51bc4a",
}
# Corpora that commit no Cargo.lock: `cargo +1.99.0 generate-lockfile --publish-time <time>` resolves their
# dependencies to what crates.io held then (unstable in cargo 1.99, so RUSTC_BOOTSTRAP=1 and -Zunstable-options),
# the lock the recordings were made with.
CORPUS_LOCK_PUBLISH_TIMES = {
    "tokio": "2026-10-04T00:00:00Z",
}
TOOL = "clippy"


def pack() -> dict:
    return json.loads(PACK.read_text())


def tool_version() -> str:
    """The clippy version the pack pins (`tools[].version`), which every recording carries."""
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
    """The pack's declaration of corpus `name`, checked against the commit this file pins."""
    for entry in pack()["corpora"]:
        if entry["name"] == name:
            if CORPUS_COMMITS.get(name) != entry["commit"]:
                raise SystemExit(f"{PACK} pins {name} at {entry['commit']}, common.py at {CORPUS_COMMITS.get(name)}")
            return entry
    raise SystemExit(f"{name} is not a corpus of {PACK}")


def corpus_files(root: pathlib.Path, name: str) -> list[str]:
    """Corpus-relative `.rs` files under the pack's paths (`.git` and `target` directories excluded), checked out at
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
    no tokio_unstable or other `--cfg`. Returns the pack lints' findings (deduplicated across the units that compile
    a file twice, primary span inside the corpus), the corpus files no compiled unit read (from the units' dep-info),
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
        key = (rule, path, primary["line_start"], primary["column_start"])
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


# Rust keywords that may precede `!` without naming a macro (`if !(..)`, `return !x`, `in ![..]`).
KEYWORDS = {"as", "async", "await", "box", "break", "const", "continue", "crate", "do", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
            "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
            "unsafe", "use", "where", "while", "yield"}
IDENT = re.compile(r"(?:r#)?[A-Za-z_][A-Za-z0-9_]*")
OPEN = {"(": ")", "[": "]", "{": "}"}


def macro_token_trees(text: str) -> tuple[list[tuple[int, int]], list[tuple[int, int]]]:
    """Code-point ranges `[start, end)` of the outermost bang-macro invocation token trees in `text` (`name!(..)`,
    `path::name! { .. }`; the delimiters included), and of `macro_rules! name { .. }` definition bodies. A lexer:
    comments, string, raw string, byte, C-string and char literals and lifetimes are skipped, so delimiters inside
    them never count."""
    invocations: list[tuple[int, int]] = []
    definitions: list[tuple[int, int]] = []
    stack: list[tuple[str, int, str | None]] = []  # (closing delimiter, start, kind of the outermost macro)
    i, n = 0, len(text)
    previous_ident: str | None = None  # an identifier immediately followed by `!`
    pending: str | None = None  # "invocation" / "definition" once `ident!` (or `macro_rules! name`) was seen
    while i < n:
        c = text[i]
        if c in " \t\r\n":
            i += 1
            continue
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            depth, i = 1, i + 2
            while i < n and depth:
                if text.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif text.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
            continue
        raw = re.match(r'(?:b|c)?r(#*)"', text[i:i + 300])
        if raw:
            end = text.find('"' + raw.group(1), i + raw.end())
            i = n if end < 0 else end + 1 + len(raw.group(1))
            previous_ident = pending = None
            continue
        if c == '"' or (c in "bc" and text.startswith('"', i + 1)):
            i += 1 if c == '"' else 2
            while i < n and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
            i += 1
            previous_ident = None
            continue
        if c == "'" or (c == "b" and text.startswith("'", i + 1)):
            start = i + (1 if c == "'" else 2)
            if start < n and text[start] == "\\":
                end = text.find("'", start + 2)
                i = n if end < 0 else end + 1
            elif start + 1 < n and text[start + 1] == "'":
                i = start + 2
            else:  # a lifetime or label
                match = IDENT.match(text, start)
                i = match.end() if match else start
            previous_ident = None
            continue
        match = IDENT.match(text, i)
        if match:
            word = match.group(0)
            i = match.end()
            if pending == "definition-name":
                pending = "definition"
                continue
            previous_ident = word if text.startswith("!", i) and not text.startswith("!=", i) else None
            pending = None
            continue
        if c == "!" and previous_ident is not None:
            word, previous_ident = previous_ident, None
            i += 1
            if word == "macro_rules":
                pending = "definition-name"
            elif word not in KEYWORDS:
                pending = "invocation"
            continue
        if c in OPEN:
            kind = pending if pending in ("invocation", "definition") else None
            outer = next((entry[2] for entry in stack if entry[2] is not None), None)
            stack.append((OPEN[c], i, kind if outer is None else None))
            pending = previous_ident = None
            i += 1
            continue
        if c in ")]}":
            if stack:
                closing, start, kind = stack.pop()
                if kind == "invocation":
                    invocations.append((start, i + 1))
                elif kind == "definition":
                    definitions.append((start, i + 1))
            i += 1
            pending = previous_ident = None
            continue
        pending = previous_ident = None
        i += 1
    return invocations, definitions


def classify_macro_positions(root: pathlib.Path, findings: list[dict]) -> dict:
    """How many findings' primary span starts inside a bang-macro invocation's token tree (e.g. tokio's
    `cfg_rt! { .. }`), inside a `macro_rules!` definition body, or carries rustc's expansion backtrace."""
    counts = {"findings": len(findings), "insideInvocationTokenTree": 0, "insideMacroRulesDefinition": 0,
              "expansion": 0, "byRule": {}}
    cache: dict[str, tuple[list[int], list[tuple[int, int]], list[tuple[int, int]]]] = {}
    for finding in findings:
        path = finding["path"]
        if path not in cache:
            text = (root / path).read_text(encoding="utf-8")
            starts = [0] + [index + 1 for index, char in enumerate(text) if char == "\n"]
            cache[path] = (starts, *macro_token_trees(text))
        starts, invocations, definitions = cache[path]
        offset = starts[finding["line"] - 1] + finding["column"] - 1
        rule = counts["byRule"].setdefault(finding["rule"], {"findings": 0, "insideInvocationTokenTree": 0,
                                                             "insideMacroRulesDefinition": 0, "expansion": 0})
        rule["findings"] += 1
        for key, ranges in (("insideInvocationTokenTree", invocations), ("insideMacroRulesDefinition", definitions)):
            if any(start <= offset < end for start, end in ranges):
                counts[key] += 1
                rule[key] += 1
        if finding.get("expansion"):
            counts["expansion"] += 1
            rule["expansion"] += 1
    return counts


def corpus_source(name: str, rules: list[str], skipped: list[str]) -> dict:
    declared = corpus(name)
    return {"kind": "corpus", "name": name, "repository": declared["repository"], "commit": declared["commit"],
            "rules": sorted(rules), "skipped": sorted(skipped)}


def write(path: pathlib.Path, source: dict, findings: list[dict]) -> None:
    findings = sorted(({key: finding[key] for key in ("rule", "path", "line", "column")} for finding in findings),
                      key=lambda f: (f["rule"], f["path"], f["line"], f["column"]))
    path.write_text(json.dumps({"tool": TOOL, "version": tool_version(), "source": source, "findings": findings},
                               indent=1, ensure_ascii=False) + "\n")
    pack_fixtures.recording_written(path, source)
