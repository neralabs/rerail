#!/usr/bin/env python3
"""Writes corpus.expected.json: what `cargo clippy` 0.1.99 (Rust 1.99.0) decides, per package, for every Clippy lint
the rust pack ports, over the Rerail-authored workspaces in corpus.json (offline, run by a maintainer; never in CI).

Each case's files are written to a fresh folder. Every package in it (a `Cargo.toml` with `[package]`) gets the probe,
`src/rerail_probe.rs` with one item per ported lint that the lint reports, included at the end of each of its crate
roots (`src/lib.rs`, `src/main.rs`, a new `src/lib.rs` when it has neither; `include!`, as a module would not report
`single_component_path_imports`). Then
`cargo clippy --workspace --offline -v --message-format=json` runs at the case's root, with an empty CARGO_HOME so no
user config adds flags. Per package folder it records:
- `flags`: the lint flags Cargo passes clippy-driver (`--warn=clippy::pedantic`, …), in Cargo's order;
- `on`: the ported lints the probe reports in every target of the package;
- `mixed`: those it reports in some of its targets only (`src/lib.rs` and `src/main.rs` set them differently).
Every other ported lint is off there. A package Cargo does not lint from the root (excluded, outside the workspace)
has no record. When Cargo refuses the workspace, `error` is its first error line and no package is linted.

Before the cases run, the probe is checked: compiled with `--force-warn` for each ported lint, it must report every
one of them.

Pins (refuses anything else): cargo 1.99.0 (5f94df478 2026-08-27), clippy 0.1.99 (b940084d7e 2026-09-28) = rust-clippy
rust-1.99.0 49e2f89a2605a9a340e04955fdbefa9803e6027a, from the rustup toolchain 1.99.0
(scripts/pack-data/runtimes/clippy-1.99.0/rust-toolchain.toml).
Usage: python3 generate_clippy.py
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess
import tempfile
import tomllib

HERE = pathlib.Path(__file__).resolve().parent
TOOLCHAIN = "1.99.0"
CARGO_VERSION = "cargo 1.99.0 (5f94df478 2026-08-27)"
CLIPPY_VERSION = "clippy 0.1.99 (b940084d7e 2026-09-28)"

# One item per lint the rust pack ports (packs/rust/rules/*.json `suppress.clippy`), each reporting that lint.
PROBES = {
    "clippy::allow_attributes": "#[allow(dead_code)]\nfn allow_attributes() {}",
    "clippy::allow_attributes_without_reason": "",  # The `#[allow(dead_code)]` above reports it too.
    "clippy::builtin_type_shadow": "pub fn builtin_type_shadow<u32>(a: u32) -> u32 { a }",
    "clippy::decimal_literal_representation": "pub fn decimal_literal_representation() -> u32 { 4_294_967_295 }",
    "clippy::double_parens": "pub fn double_parens() -> u8 { ((1)) }",
    "clippy::else_if_without_else":
        "pub fn else_if_without_else(x: u8) { if x == 1 { println!(\"a\"); } else if x == 2 { println!(\"b\"); } }",
    "clippy::inconsistent_digit_grouping": "pub fn inconsistent_digit_grouping() -> u64 { 618_64_9189_73_511 }",
    "clippy::int_plus_one": "pub fn int_plus_one(x: i32, y: i32) -> bool { x >= y + 1 }",
    "clippy::large_digit_groups": "pub fn large_digit_groups() -> u64 { 12345_67890 }",
    "clippy::many_single_char_names":
        "pub fn many_single_char_names() -> i32 { let (a, b, c, d, e) = (1, 2, 3, 4, 5); a + b + c + d + e }",
    "clippy::mixed_case_hex_literals": "pub fn mixed_case_hex_literals() -> u32 { 0xabCD }",
    "clippy::needless_arbitrary_self_type":
        "pub struct NeedlessArbitrarySelfType;\nimpl NeedlessArbitrarySelfType { pub fn f(self: Self) {} }",
    "clippy::needless_else": "pub fn needless_else(x: bool) { if x { println!(\"a\"); } else { } }",
    "clippy::needless_raw_string_hashes": "pub fn needless_raw_string_hashes() -> &'static str { r#\"a\\b\"# }",
    "clippy::needless_raw_strings": "pub fn needless_raw_strings() -> &'static str { r\"hello\" }",
    "clippy::octal_escapes": "pub fn octal_escapes() -> &'static str { \"\\033[0m\" }",
    "clippy::partial_pub_fields":
        "pub struct PartialPubFields { pub a: u8, b: u8 }\nimpl PartialPubFields { pub fn b(&self) -> u8 { self.b } }",
    "clippy::precedence": "pub fn precedence() -> i32 { 1 << 2 + 3 }",
    "clippy::precedence_bits": "pub fn precedence_bits(x: u32) -> u32 { 0x0F00 & x << 4 }",
    "clippy::redundant_at_rest_pattern":
        "pub fn redundant_at_rest_pattern(a: &[u8]) -> usize { if let [rest @ ..] = a { rest.len() } else { 0 } }",
    "clippy::redundant_field_names":
        "pub struct RedundantFieldNames { pub a: u8 }\n"
        "pub fn redundant_field_names(a: u8) -> RedundantFieldNames { RedundantFieldNames { a: a } }",
    "clippy::redundant_static_lifetimes": "pub const REDUNDANT_STATIC_LIFETIMES: &'static str = \"x\";",
    "clippy::single_char_lifetime_names": "pub fn single_char_lifetime_names<'a>(x: &'a u8) -> &'a u8 { x }",
    "clippy::single_component_path_imports":
        "use core;\npub fn single_component_path_imports() -> u8 { core::cmp::max(1, 2) }",
    "clippy::unneeded_wildcard_pattern": "pub fn unneeded_wildcard_pattern(t: (u8, u8, u8)) -> bool { matches!(t, (0, .., _)) }",
    "clippy::unnested_or_patterns": "pub fn unnested_or_patterns(x: Option<u8>) -> bool { matches!(x, Some(1) | Some(2)) }",
    "clippy::unreadable_literal": "pub fn unreadable_literal() -> u64 { 61864918973511 }",
    "clippy::unusual_byte_groupings": "pub fn unusual_byte_groupings() -> u32 { 0x1_234_5678 }",
    "clippy::zero_prefixed_literal": "pub fn zero_prefixed_literal() -> u32 { 0123 }",
}
PROBE_SOURCE = "\n".join(item for item in PROBES.values() if item) + "\n"
PROBE_FILE = "rerail_probe.rs"
LINT_FLAG = re.compile(r"(?:^|\s)'?--(allow|warn|deny|forbid)=([^\s']+)")
CRATE_ROOTS = ("src/lib.rs", "src/main.rs")


def run(args: list[str], cwd: pathlib.Path, home: pathlib.Path) -> subprocess.CompletedProcess[str]:
    env = {key: value for key, value in os.environ.items() if not key.startswith(("CARGO", "RUSTFLAGS", "RUSTC", "CLIPPY"))}
    env.update(RUSTUP_TOOLCHAIN=TOOLCHAIN, CARGO_HOME=str(home), CARGO_TARGET_DIR=str(home / "target"), CARGO_TERM_COLOR="never")
    return subprocess.run(args, cwd=cwd, env=env, capture_output=True, text=True, check=False)


def check_pins(home: pathlib.Path) -> None:
    cargo = run(["cargo", "--version"], HERE, home).stdout.strip()
    clippy = run(["cargo", "clippy", "--version"], HERE, home).stdout.strip()
    if cargo != CARGO_VERSION or clippy != CLIPPY_VERSION:
        raise SystemExit(f"want {CARGO_VERSION} / {CLIPPY_VERSION}, found {cargo} / {clippy}")


def is_package(manifest: pathlib.Path) -> bool:
    try:
        return "package" in tomllib.loads(manifest.read_text())
    except tomllib.TOMLDecodeError:
        return False


def add_probes(root: pathlib.Path) -> None:
    """Gives every package under `root` the probe, included at the end of each of its crate roots."""
    for folder in [manifest.parent for manifest in root.rglob("Cargo.toml") if is_package(manifest)]:
        (folder / "src").mkdir(exist_ok=True)
        (folder / "src" / PROBE_FILE).write_text(PROBE_SOURCE)
        roots = [folder / name for name in CRATE_ROOTS if (folder / name).exists()] or [folder / "src/lib.rs"]
        for crate_root in roots:
            text = crate_root.read_text() if crate_root.exists() else ""
            crate_root.write_text(f'{text}include!("{PROBE_FILE}");\n')


def unit(command: str, root: pathlib.Path) -> tuple[str, str, list[str]]:
    """A clippy-driver command Cargo runs: its crate root's absolute path, its package's folder and its lint flags."""
    source = next(word.strip("'") for word in command.split() if word.strip("'").endswith(".rs"))
    folder = pathlib.PurePosixPath(source).parent.parent.as_posix()
    flags = [f"{level} {lint}" for level, lint in LINT_FLAG.findall(command)]
    return str((root / source).resolve()), "" if folder == "." else folder, flags


def units(stderr: str, root: pathlib.Path) -> list[tuple[str, str, list[str]]]:
    """Each clippy-driver command Cargo reports running (`-v`), as `unit` reads it."""
    commands = [line.split("`", 1)[1].rstrip("`") for line in stderr.splitlines() if "Running `" in line]
    return sorted(unit(command, root) for command in commands if "clippy-driver" in command)


def probe_lint(message: dict) -> str | None:
    """The ported lint a compiler message reports in the probe, if it is one."""
    diagnostic = message.get("message", {}) if message.get("reason") == "compiler-message" else {}
    code = (diagnostic.get("code") or {}).get("code")
    in_probe = any(span["file_name"].endswith(PROBE_FILE) for span in diagnostic.get("spans", []))
    return code if code in PROBES and in_probe else None


def reported(stdout: str) -> dict[str, set[str]]:
    """Per crate root's absolute path, the ported lints the probe reports there."""
    found: dict[str, set[str]] = {}
    for message in map(json.loads, stdout.splitlines()):
        lint = probe_lint(message)
        if lint is not None:
            found.setdefault(str(pathlib.Path(message["target"]["src_path"]).resolve()), set()).add(lint)
    return found


def check_probe(home: pathlib.Path) -> None:
    with tempfile.TemporaryDirectory() as folder:
        root = pathlib.Path(folder)
        (root / "Cargo.toml").write_text('[package]\nname = "probe"\nversion = "0.1.0"\nedition = "2024"\n')
        add_probes(root)
        forced = [f"--force-warn={lint}" for lint in PROBES]
        result = run(["cargo", "clippy", "--offline", "--message-format=json", "--", *forced], root, home)
        missing = set(PROBES) - set().union(*reported(result.stdout).values())
        if missing:
            raise SystemExit(f"the probe does not report {sorted(missing)}:\n{result.stderr}")


def package_records(stdout: str, stderr: str, root: pathlib.Path) -> dict[str, dict]:
    """Per package folder Cargo lints: its flags, the lints reported in every target, and those in some only."""
    lints = reported(stdout)
    packages: dict[str, dict] = {}
    for source, package, flags in units(stderr, root):
        packages.setdefault(package, {"flags": flags, "targets": []})["targets"].append(lints.get(source, set()))
    records = {}
    for package, entry in sorted(packages.items()):
        every, some = set.intersection(*entry["targets"]), set.union(*entry["targets"])
        records[package] = {"flags": entry["flags"], "on": sorted(every), "mixed": sorted(some - every)}
    return records


def record(case: dict, home: pathlib.Path) -> dict:
    with tempfile.TemporaryDirectory() as folder:
        root = pathlib.Path(folder).resolve()
        for name, text in case["files"].items():
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(text)
        add_probes(root)
        result = run(["cargo", "clippy", "--workspace", "--offline", "-v", "--message-format=json"], root, home)
        errors = [line for line in result.stderr.splitlines() if line.startswith("error:") and "could not compile" not in line]
        if errors:
            return {"name": case["name"], "error": errors[0].replace(str(root), "<root>")}
        return {"name": case["name"], "packages": package_records(result.stdout, result.stderr, root)}


def main() -> None:
    with tempfile.TemporaryDirectory() as folder:
        home = pathlib.Path(folder)
        check_pins(home)
        check_probe(home)
        cases = json.loads((HERE / "corpus.json").read_text())["cases"]
        records = [record(case, home) for case in cases]
    text = '{"cases": [\n' + ",\n".join(json.dumps(entry, sort_keys=True) for entry in records) + "\n]}\n"
    (HERE / "corpus.expected.json").write_text(text)


if __name__ == "__main__":
    main()
