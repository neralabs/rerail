#!/usr/bin/env python3
"""Writes fixtures.expected.json: the T2 lint-level matrix of clippy-driver 0.1.99 (rustc 1.99.0) over the
Rerail-authored fixtures in fixtures/*.rs (offline, run by a maintainer; never in CI).

Each fixture is compiled twice as a 2024-edition library:
- with `--force-warn` for every rule in RULES: every site of those rules, whatever the attributes say; and
- plainly: what the attributes leave, at the level they leave it.
Every force-warn site becomes a case `{file, rule, line, column, emitted, level, sites, emitted_sites}`: `emitted`
when the plain run reports the same rule at the same position, `level` the plain run's level (`warning`/`error`)
or `allow` when suppressed (by `allow` or a fulfilled `expect`). Sites of one macro body expanded several times
share a position: `sites` counts the force-warn diagnostics there, `emitted_sites` the plain ones. Diagnostics
only the plain run produces about the attributes themselves (rules in ATTRIBUTE_RULES and every error, e.g.
E0453 under `forbid`, E0658 for attributes on expressions) are cases too, `emitted: true`. `files` records each
run's exit status; when the plain run fails, later passes may not have run. Lines and columns are 1-based,
columns in code points (rustc's JSON `column_start`); rules keep rustc's spelling, so clippy lints carry
`clippy::`. A diagnostic inside a macro expansion is placed where rustc's primary span is (the macro definition
for tokens of the macro body, the argument for tokens passed in).

Pins (refuses anything else): clippy 0.1.99 (b940084d7e 2026-09-28) = rust-clippy rust-1.99.0
49e2f89a2605a9a340e04955fdbefa9803e6027a, rustc 1.99.0 (b940084d7 2026-09-28).
Usage: python3 generate_fixtures.py
"""

from __future__ import annotations

import collections
import json
import pathlib
import subprocess
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
BIN = pathlib.Path.home() / ".rustup" / "toolchains" / "1.99.0-aarch64-apple-darwin" / "bin"
CLIPPY_VERSION = "clippy 0.1.99 (b940084d7e 2026-09-28)"
RUSTC_VERSION = "rustc 1.99.0 (b940084d7 2026-09-28)"
CLIPPY_COMMIT = "49e2f89a2605a9a340e04955fdbefa9803e6027a"
RULES = ["clippy::double_parens", "clippy::needless_else", "clippy::unreadable_literal",
         "clippy::redundant_static_lifetimes", "unused_variables", "unused_labels"]
ATTRIBUTE_RULES = {"unfulfilled_lint_expectations", "unknown_lints", "renamed_and_removed_lints",
                   "forbidden_lint_groups", "unused_attributes", "clippy::duplicated_attributes"}


def check_pins() -> None:
    clippy = subprocess.run([str(BIN / "clippy-driver"), "--version"], capture_output=True, text=True,
                            check=True).stdout.strip()
    rustc = subprocess.run([str(BIN / "rustc"), "-V"], capture_output=True, text=True, check=True).stdout.strip()
    if clippy != CLIPPY_VERSION or rustc != RUSTC_VERSION:
        raise SystemExit(f"want {CLIPPY_VERSION} / {RUSTC_VERSION}, found {clippy} / {rustc}")


def compile_(name: str, out: pathlib.Path, force: bool) -> tuple[int, list[dict]]:
    flags: list[str] = []
    if force:
        for rule in RULES:
            flags += ["--force-warn", rule]
    proc = subprocess.run(
        [str(BIN / "clippy-driver"), "--edition", "2024", "--crate-type", "lib", "--crate-name", "fixture",
         "--error-format", "json", "--emit", "metadata", "--out-dir", str(out), *flags, "fixtures/" + name],
        capture_output=True, text=True, cwd=HERE, check=False)
    found = []
    for line in proc.stderr.splitlines():
        message = json.loads(line)
        if message.get("$message_type") != "diagnostic" or not message["spans"]:
            continue
        span = next(s for s in message["spans"] if s["is_primary"])
        while span["file_name"] != "fixtures/" + name and span.get("expansion"):
            span = span["expansion"]["span"]
        code = message["code"]["code"] if message["code"] else ""
        found.append({"rule": code, "level": message["level"], "file": span["file_name"],
                      "line": span["line_start"], "column": span["column_start"]})
    return proc.returncode, found


def main() -> None:
    check_pins()
    cases = []
    files = {}
    with tempfile.TemporaryDirectory() as tmp:
        out = pathlib.Path(tmp)
        for path in sorted((HERE / "fixtures").glob("*.rs")):
            name = path.name
            forced_exit, forced = compile_(name, out, True)
            plain_exit, plain = compile_(name, out, False)
            files["fixtures/" + name] = {"exit_force_warn": forced_exit, "exit": plain_exit}
            plain_at: dict[tuple, list[str]] = {}
            for d in plain:
                plain_at.setdefault((d["rule"], d["file"], d["line"], d["column"]), []).append(d["level"])
            sites = collections.Counter((d["rule"], d["file"], d["line"], d["column"]) for d in forced
                                        if d["rule"] in RULES)
            for key in sorted(sites):
                rule, file, line, column = key
                levels = plain_at.get(key, [])
                cases.append({"file": file, "rule": rule, "line": line, "column": column, "emitted": bool(levels),
                              "level": "/".join(sorted(set(levels))) if levels else "allow",
                              "sites": sites[key], "emitted_sites": len(levels)})
            for key in sorted(plain_at):
                rule, file, line, column = key
                if key in sites:
                    continue
                levels = plain_at[key]
                if rule in RULES or rule in ATTRIBUTE_RULES or "error" in levels:
                    cases.append({"file": file, "rule": rule, "line": line, "column": column, "emitted": True,
                                  "level": "/".join(sorted(set(levels))), "sites": 0, "emitted_sites": len(levels)})
    cases.sort(key=lambda c: (c["file"], c["line"], c["column"], c["rule"]))
    result = {
        "tool": "clippy-driver",
        "version": CLIPPY_VERSION,
        "rustc": RUSTC_VERSION,
        "source": {"kind": "fixtures", "origin": "Rerail-authored (Apache-2.0), rerail-fixtures/rust/suppress/fixtures",
                   "clippy": "https://github.com/rust-lang/rust-clippy@" + CLIPPY_COMMIT, "rules": RULES,
                   "attribute_rules": sorted(ATTRIBUTE_RULES), "edition": "2024", "crate_type": "lib"},
        "files": files,
        "cases": cases,
    }
    (HERE / "fixtures.expected.json").write_text(json.dumps(result, indent=1) + "\n")


if __name__ == "__main__":
    main()
