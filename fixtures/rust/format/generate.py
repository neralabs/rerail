#!/usr/bin/env python3
"""Writes rust.expected.json: what rustc 1.99.0 does with every format macro call in rust.cases.json
(offline, run by a maintainer; never in CI).

rust.cases.json holds `{"cases": [...]}`; a case is `{"name", "src", "items"?, "observe"?, "calls"?}`:
`src` is one macro invocation, `items` the statements it needs (`let x = 1;`, `use std::fmt::Write; ...`),
`observe` the statement that makes the result visible, `@SRC@` standing for `src` (default `src;`), and
`calls: false` marks calls whose format string the extraction must report as unknown (no literal template).

Each case is compiled alone as `fn main() { <items> <src>; }` (with `--error-format=json`) and recorded:
- `calls: false` copied from the case when set;
- `accepted`: whether rustc built it, and `errors`: every error diagnostic with `message`, `code` ("" when it
  has none) and its primary spans as `{start, end, text}`, bytes relative to the start of `src` (a span inside
  an expansion is followed out to the call site);
- for accepted cases the program is rebuilt as `fn main() { <panic hook printing the message to stdout>;
  catch_unwind(|| { <items> <observe or src;> }) }` and run, recording `stdout` and `stderr`: the output proves
  which argument filled which field (distinct argument values), and a panic message is printed on stdout.

Pins (refuses anything else): rustc 1.99.0 (b940084d7 2026-09-28), edition 2024.
Build products go to <checkout>/.rerail/tools/rust/format/ (gitignored).
Usage: python3 generate.py
"""

from __future__ import annotations

import json
import pathlib
import subprocess

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[2]
TOOLCHAIN = "1.99.0"
RUSTC_VERSION = "rustc 1.99.0 (b940084d7 2026-09-28)"
EDITION = "2024"


def checkout_root() -> pathlib.Path:
    for parent in [REPO, *REPO.parents]:
        if (parent / ".rerail" / "corpora" / "rust").is_dir():
            return parent
    raise SystemExit("no .rerail/corpora/rust above " + str(REPO))


BUILD = checkout_root() / ".rerail" / "tools" / "rust" / "format"

COMPILE_PREFIX = "#![allow(unused)]\nfn main() { "
RUN_PREFIX = """#![allow(unused)]
fn main() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = if let Some(text) = payload.downcast_ref::<String>() {
            text.clone()
        } else if let Some(text) = payload.downcast_ref::<&str>() {
            text.to_string()
        } else {
            String::new()
        };
        println!("{message}");
    }));
    let _ = std::panic::catch_unwind(|| { """


def check_pins() -> None:
    found = subprocess.run(["rustc", "+" + TOOLCHAIN, "-V"], capture_output=True, text=True, check=True).stdout.strip()
    if found != RUSTC_VERSION:
        raise SystemExit(f"want {RUSTC_VERSION}, found {found}")


def rustc(path: pathlib.Path, binary: pathlib.Path) -> tuple[bool, list[dict]]:
    proc = subprocess.run(["rustc", "+" + TOOLCHAIN, "--edition", EDITION, "--error-format", "json", "-o",
                           str(binary), str(path)], capture_output=True, text=True, check=False)
    errors = []
    for line in proc.stderr.splitlines():
        message = json.loads(line)
        if message.get("$message_type") != "diagnostic" or message["level"] != "error" or not message["spans"]:
            continue
        errors.append(message)
    return proc.returncode == 0, errors


def call_site(span: dict, path: pathlib.Path) -> dict | None:
    while span["file_name"] != str(path):
        if not span.get("expansion"):
            return None
        span = span["expansion"]["span"]
    return span


def record_errors(messages: list[dict], path: pathlib.Path, offset: int) -> list[dict]:
    errors = []
    for message in messages:
        spans = []
        for span in message["spans"]:
            if not span["is_primary"]:
                continue
            span = call_site(span, path)
            if span is None:
                continue
            spans.append({"start": span["byte_start"] - offset, "end": span["byte_end"] - offset,
                          "text": path.read_bytes()[span["byte_start"]:span["byte_end"]].decode()})
        code = message["code"]["code"] if message["code"] else ""
        errors.append({"message": message["message"], "code": code, "spans": spans})
    return errors


def statement(case: dict) -> str:
    observe = case.get("observe")
    if observe is None:
        return case["src"] + ";"
    return observe.replace("@SRC@", case["src"]) + ";"


def record(case: dict) -> dict:
    items = case.get("items", "")
    path = BUILD / "case.rs"
    binary = BUILD / "case"
    head = COMPILE_PREFIX + items + (" " if items else "")
    path.write_text(head + case["src"] + "; }\n")
    accepted, messages = rustc(path, binary)
    result = {"name": case["name"], "src": case["src"], "accepted": accepted,
              "errors": record_errors(messages, path, len(head.encode()))}
    if case.get("calls") is False:
        result["calls"] = False
    if accepted:
        path.write_text(RUN_PREFIX + items + " " + statement(case) + " }); }\n")
        ok, messages = rustc(path, binary)
        if not ok:
            raise SystemExit(case["name"] + ": accepted alone but the run harness fails: " + json.dumps(messages))
        ran = subprocess.run([str(binary)], capture_output=True, text=True, check=False)
        result["stdout"], result["stderr"] = ran.stdout, ran.stderr
    return result


def main() -> None:
    check_pins()
    BUILD.mkdir(parents=True, exist_ok=True)
    cases = json.loads((HERE / "rust.cases.json").read_text())["cases"]
    names = [case["name"] for case in cases]
    if len(set(names)) != len(names):
        raise SystemExit("duplicate case names")
    results = [record(case) for case in cases]
    expected = {
        "tool": "rustc",
        "version": RUSTC_VERSION,
        "edition": EDITION,
        "source": {"kind": "fixtures", "origin": "Rerail-authored (Apache-2.0), fixtures/rust/format/rust.cases.json"},
        "cases": results,
    }
    (HERE / "rust.expected.json").write_text(json.dumps(expected, indent=1, ensure_ascii=False) + "\n")
    accepted = sum(1 for result in results if result["accepted"])
    print(f"{len(results)} cases, {accepted} accepted, {len(results) - accepted} rejected")


if __name__ == "__main__":
    main()
