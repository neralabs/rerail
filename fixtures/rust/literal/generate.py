#!/usr/bin/env python3
"""Writes literal.expected.json: rustc 1.99.0's value for every Rust literal in fixtures/ (offline, run by a
maintainer; never in CI).

fixtures/valid.rs holds `case!(name, type: literal)` lines (`_` keeps the literal's own type). It is compiled as a
module of a generated harness whose `case!` binds the literal to the type and prints its source text
(`stringify!`), Rust type and value; the harness must build without diagnostics. fixtures/edge.rs holds literals
at the edge of the language; each case is compiled alone, at its own line, recording rustc's diagnostics (error
code or lint name, level, line, column) and, when it still builds, the value.

Values: integers as decimal strings; floats as Rust's `{:?}` plus the IEEE bits in hex; chars as the character
and its code point; `&str` as the text plus its UTF-8 in hex; byte strings and C strings (without the trailing
NUL) as hex; booleans as JSON booleans. Lines and columns are 1-based, columns in code points.

Pins (refuses anything else): rustc 1.99.0 (b940084d7 2026-09-28), edition 2024.
Build products go to <checkout>/.nera/tools/rust/literal/ (gitignored).
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
        if (parent / ".nera" / "corpora" / "rust").is_dir():
            return parent
    raise SystemExit("no .nera/corpora/rust above " + str(REPO))


BUILD = checkout_root() / ".nera" / "tools" / "rust" / "literal"

HARNESS = r"""#![allow(unused)]
use std::ffi::CStr;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

trait Emit {
    fn emit(&self) -> (&'static str, String);
}

macro_rules! ints {
    ($($t:ty),*) => {
        $(impl Emit for $t {
            fn emit(&self) -> (&'static str, String) { ("int", self.to_string()) }
        })*
    };
}
ints!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

impl Emit for f32 {
    fn emit(&self) -> (&'static str, String) { ("float", format!("{:?} {:08x}", self, self.to_bits())) }
}
impl Emit for f64 {
    fn emit(&self) -> (&'static str, String) { ("float", format!("{:?} {:016x}", self, self.to_bits())) }
}
impl Emit for char {
    fn emit(&self) -> (&'static str, String) { ("char", (*self as u32).to_string()) }
}
impl Emit for bool {
    fn emit(&self) -> (&'static str, String) { ("bool", self.to_string()) }
}
impl Emit for &str {
    fn emit(&self) -> (&'static str, String) { ("str", hex(self.as_bytes())) }
}
impl<const N: usize> Emit for &[u8; N] {
    fn emit(&self) -> (&'static str, String) { ("bytes", hex(&self[..])) }
}
impl Emit for &CStr {
    fn emit(&self) -> (&'static str, String) { ("cstr", hex(self.to_bytes())) }
}

macro_rules! case {
    ($name:ident, $t:ty: $lit:expr) => {{
        let value: $t = $lit;
        let (kind, text) = $crate::Emit::emit(&value);
        println!("{}\t{}\t{}\t{}\t{}\t{}", stringify!($name), line!(), std::any::type_name_of_val(&value), kind,
                 $crate::hex(stringify!($lit).as_bytes()), text);
    }};
}

#[path = "MODULE"]
mod cases;

fn main() {
    cases::run();
}
"""


def check_pins() -> None:
    found = subprocess.run(["rustc", "+" + TOOLCHAIN, "-V"], capture_output=True, text=True, check=True).stdout.strip()
    if found != RUSTC_VERSION:
        raise SystemExit(f"want {RUSTC_VERSION}, found {found}")


def build(module: pathlib.Path, name: str) -> tuple[int, list[dict], pathlib.Path]:
    harness = BUILD / (name + "_harness.rs")
    harness.write_text(HARNESS.replace("MODULE", str(module)))
    binary = BUILD / name
    proc = subprocess.run(["rustc", "+" + TOOLCHAIN, "--edition", EDITION, "--error-format", "json", "-o",
                           str(binary), str(harness)], capture_output=True, text=True, check=False)
    diagnostics = []
    for line in proc.stderr.splitlines():
        message = json.loads(line)
        if message.get("$message_type") != "diagnostic" or not message["spans"]:
            continue
        span = next(s for s in message["spans"] if s["is_primary"])
        while span["file_name"] != str(module) and span.get("expansion"):
            span = span["expansion"]["span"]
        code = message["code"]["code"] if message["code"] else ""
        diagnostics.append({"code": code, "level": message["level"], "line": span["line_start"],
                            "column": span["column_start"], "message": message["message"]})
    # The harness has no `Emit` for types rustc rejects anyway (f16); that follow-on error is not the literal's.
    diagnostics = [d for d in diagnostics if ": Emit`" not in d["message"]]
    diagnostics.sort(key=lambda d: (d["line"], d["column"], d["code"], d["message"]))
    return proc.returncode, diagnostics, binary


def run(binary: pathlib.Path) -> list[dict]:
    out = subprocess.run([str(binary)], capture_output=True, text=True, check=True).stdout
    cases = []
    for line in out.splitlines():
        name, at, rust_type, kind, source, text = line.split("\t")
        case = {"name": name, "line": int(at), "source": bytes.fromhex(source).decode(), "type": rust_type,
                "kind": kind}
        if kind == "int":
            case["value"] = text
        elif kind == "float":
            debug, bits = text.split(" ")
            case["value"], case["bits"] = debug, bits
        elif kind == "char":
            case["value"], case["code_point"] = chr(int(text)), int(text)
        elif kind == "bool":
            case["value"] = text == "true"
        elif kind == "str":
            case["value"], case["utf8"] = bytes.fromhex(text).decode(), text
        else:
            case["bytes"] = text
        cases.append(case)
    return cases


def edge_cases(path: pathlib.Path) -> list[tuple[int, str]]:
    """(first line, text) of every `case!(...);` statement, one or more whole lines each."""
    lines = path.read_text().split("\n")
    found = []
    k = 0
    while k < len(lines):
        if lines[k].strip().startswith("case!("):
            start = k
            while not lines[k].rstrip().endswith(");"):
                k += 1
            found.append((start + 1, "\n".join(lines[start : k + 1])))
        k += 1
    return found


def main() -> None:
    check_pins()
    BUILD.mkdir(parents=True, exist_ok=True)
    results = []

    valid = HERE / "fixtures" / "valid.rs"
    code, diagnostics, binary = build(valid, "valid")
    if code != 0 or diagnostics:
        raise SystemExit("fixtures/valid.rs must build cleanly: " + json.dumps(diagnostics[:10], indent=1))
    for case in run(binary):
        results.append({"file": "fixtures/valid.rs", **case})

    edge = HERE / "fixtures" / "edge.rs"
    for line, text in edge_cases(edge):
        module = BUILD / "edge_case.rs"
        module.write_text("#![allow(unused)]\npub fn run() {\n" + "\n" * (line - 3) + text + "\n}\n")
        code, diagnostics, binary = build(module, "edge")
        name = text.strip()[len("case!("):].split(",", 1)[0].strip()
        case = {"file": "fixtures/edge.rs", "name": name, "line": line, "accepted": code == 0,
                "diagnostics": diagnostics}
        if code == 0:
            ran = run(binary)
            case.update({k: v for k, v in ran[0].items() if k not in ("name", "line")})
        else:
            case["source"] = text.strip()[len("case!("):-2].split(":", 1)[1].strip()
        results.append(case)

    record = {
        "tool": "rustc",
        "version": RUSTC_VERSION,
        "source": {"kind": "fixtures", "origin": "Rerail-authored (MIT), rerail-fixtures/rust/literal/fixtures",
                   "edition": EDITION},
        "cases": results,
    }
    (HERE / "literal.expected.json").write_text(json.dumps(record, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
