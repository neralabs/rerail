"""Records clippy 0.1.99 (Rust 1.99.0) for the Rust pack's ported lints (packs/rust/conformance.json),
each one checked only by an `EarlyLintPass` (AST, no types). Run by `npm run fixtures:derive` (a maintainer, or the
bump workflow for an upstream release; never pull request CI), with the pinned toolchain (upstream/common.py pins it
and refuses any other; Python 3.9 standard library only):

    python3 generate.py fixtures <rust-clippy checkout at tag rust-1.99.0> <scratch directory>

`fixtures` writes Rerail's own cases (`PROBES`, listed in UPSTREAM.md) to fixtures/rerail/ and
copies each lint's own UI tests (rust-clippy `tests/ui/**`, `tests/ui-toml/**`, commit
49e2f89a2605a9a340e04955fdbefa9803e6027a) to fixtures/ under their upstream paths, with the `auxiliary/` crates they
`//@aux-build` and the `clippy.toml` each test reads (licenses/clippy/UPSTREAM.md lists them), then runs the pinned
clippy-driver on each test as rust-clippy's tests/compile-test.rs (ui_test 0.30.7) does: edition 2024 unless
`//@edition`, `//@compile-flags` appended, `clippy_test_deps` built with `cargo +1.99.0 build --locked
--target=<host>` and its direct dependencies passed with `--extern` to every test and aux build, aux crates built
first and passed with `--extern`, ui_test's crate type (proc-macro, lib or bin from the file's contents; tests/ui
only), `--emit=metadata -Aunused -Ainternal_features -Zui-testing
-Zdeduplicate-diagnostics=no --error-format=json`, `CLIPPY_CONF_DIR` = `tests` for tests/ui and the test's directory
for tests/ui-toml. `RUSTC_BOOTSTRAP=1` is set because the tests and aux crates use `#![feature]` and the `-Z` flags,
which the stable driver otherwise refuses (upstream runs them on nightly); `-Dwarnings` is not passed (it changes
only the level). That run is compared, per test, with the upstream `.stderr` (each diagnostic's primary `-->`
line:column) and the `//~` annotations (lint and line); every difference is written to fixtures.upstream-check.json.
The recording, fixtures.expected.json, is a second run with `-W clippy::<lint>` for every ported lint added, so each
lint is recorded over every test (in-source `allow`/`expect` still apply): each finding is the lint, test path, and
the primary span's 1-based line and column (code points), one per distinct primary span (start and end): two
diagnostics that start at one point, like precedence_bits' outer and nested `^`, are two findings. `source.units`
names each upstream test's origin: the lints TESTS lists it for (Rerail's own cases have none)."""

from __future__ import annotations

import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
from collections import Counter

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import common  # noqa: E402

# Each ported lint's upstream UI tests: every test file whose `//~` annotations name the lint or that is named after
# it, at rust-clippy 49e2f89a2605a9a340e04955fdbefa9803e6027a.
TESTS = {
    "allow_attributes": ["tests/ui/allow_attributes.rs"],
    "allow_attributes_without_reason": ["tests/ui/allow_attributes_without_reason.rs"],
    "builtin_type_shadow": ["tests/ui/builtin_type_shadow.rs"],
    "decimal_literal_representation": [
        "tests/ui/decimal_literal_representation.rs",
        "tests/ui-toml/decimal_literal_representation/decimal_literal_representation.rs",
    ],
    "double_parens": ["tests/ui/double_parens.rs"],
    "else_if_without_else": ["tests/ui/else_if_without_else.rs"],
    "inconsistent_digit_grouping": [
        "tests/ui/inconsistent_digit_grouping.rs",
        "tests/ui/literals.rs",
        "tests/ui-toml/lint_decimal_readability/test.rs",
    ],
    "int_plus_one": ["tests/ui/int_plus_one.rs"],
    "large_digit_groups": ["tests/ui/large_digit_groups.rs"],
    "many_single_char_names": ["tests/ui/many_single_char_names.rs"],
    "mixed_case_hex_literals": ["tests/ui/literals.rs"],
    "needless_arbitrary_self_type": [
        "tests/ui/needless_arbitrary_self_type.rs",
        "tests/ui/needless_arbitrary_self_type_unfixable.rs",
    ],
    "needless_else": ["tests/ui/needless_else.rs"],
    "needless_raw_string_hashes": [
        "tests/ui/needless_raw_string_hashes.rs",
        "tests/ui-toml/needless_raw_string_hashes_one_allowed/needless_raw_string_hashes.rs",
    ],
    "needless_raw_strings": ["tests/ui/needless_raw_string.rs"],
    "octal_escapes": ["tests/ui/octal_escapes.rs"],
    "option_env_unwrap": ["tests/ui/option_env_unwrap.rs"],
    "partial_pub_fields": ["tests/ui/partial_pub_fields.rs"],
    "precedence": ["tests/ui/precedence.rs"],
    "precedence_bits": ["tests/ui/precedence_bits.rs"],
    "redundant_at_rest_pattern": ["tests/ui/redundant_at_rest_pattern.rs"],
    "redundant_field_names": ["tests/ui/redundant_field_names.rs"],
    "redundant_static_lifetimes": [
        "tests/ui/redundant_static_lifetimes.rs",
        "tests/ui/redundant_static_lifetimes_multiple.rs",
    ],
    "single_char_lifetime_names": ["tests/ui/single_char_lifetime_names.rs"],
    "single_component_path_imports": [
        "tests/ui/single_component_path_imports.rs",
        "tests/ui/single_component_path_imports_macro.rs",
        "tests/ui/single_component_path_imports_nested_first.rs",
        "tests/ui/single_component_path_imports_self_after.rs",
        "tests/ui/single_component_path_imports_self_before.rs",
    ],
    "unneeded_wildcard_pattern": ["tests/ui/unneeded_wildcard_pattern.rs"],
    "unnested_or_patterns": ["tests/ui/unnested_or_patterns.rs", "tests/ui/unnested_or_patterns2.rs"],
    "unreadable_literal": [
        "tests/ui/unreadable_literal.rs",
        "tests/ui/inconsistent_digit_grouping.rs",
        "tests/ui-toml/lint_decimal_readability/test.rs",
    ],
    "unusual_byte_groupings": ["tests/ui/literals.rs", "tests/ui/large_digit_groups.rs"],
    "zero_prefixed_literal": ["tests/ui/literals.rs"],
}
# Ours: inputs the ports once misread, each a golden case (fixtures/rerail/<lint>.rs). Each is recorded with every
# ported lint on, like Clippy's own tests, and its `//~` annotations are checked against the faithful run.
PROBES = {
    "allow_attributes": """\
#![warn(clippy::allow_attributes)]

#[cfg_attr(test, allow(dead_code))]
fn off_in_a_build_without_tests() {}

#[cfg_attr(all(), allow(dead_code), allow(unused_variables))]
//~^ allow_attributes
//~| allow_attributes
fn two_expanded_allows() {}

#[cfg_attr(clippy, allow(dead_code))]
//~^ allow_attributes
fn clippy_is_on() {}
""",
    "allow_attributes_without_reason": """\
#![warn(clippy::allow_attributes_without_reason)]

#[cfg_attr(test, allow(dead_code))]
fn off_in_a_build_without_tests() {}

#[cfg_attr(all(), allow(dead_code), allow(unused_variables, reason = "documented"))]
//~^ allow_attributes_without_reason
fn first_allow_has_no_reason() {}

#[cfg_attr(all(), allow(dead_code, reason = "documented"), allow(unused_variables))]
//~^ allow_attributes_without_reason
fn second_allow_has_no_reason() {}

#[cfg_attr(all(), allow(dead_code, reason = "a"), allow(unused_variables, reason = "b"))]
fn both_have_reasons() {}
""",
    "double_parens": """\
#![warn(clippy::double_parens)]

fn f(_: i32, _: i32) -> i32 {
    0
}

fn g(_: i32) -> i32 {
    0
}

fn spaced() -> i32 {
    ( (0))
    //~^ double_parens
}

fn newline() -> i32 {
    (
        (0)
    )
    //~^^^ double_parens
}

fn comments() -> i32 {
    // a comment between the parentheses: Clippy's source check rejects it
    (/* c */ (0))
}

fn triple() -> i32 {
    ((( 2 )))
    //~^ double_parens
    //~| double_parens
}

fn unit() {
    let _ = ( ());
    //~^ double_parens
    let _ = (());
    //~^ double_parens
}

fn tuple() {
    let _ = ( (1, 2));
    //~^ double_parens
}

fn call_arguments() {
    let _ = f((1), 2);
    let _ = f(1, (2));
    let _ = g( (1));
    //~^ double_parens
    let _ = g((1)) + 2;
    //~^ double_parens
    let _ = 2 + g((1));
    //~^ double_parens
    let _ = g(((2)));
    //~^ double_parens
    //~| double_parens
    let _ = g(( (2) ));
    //~^ double_parens
    //~| double_parens
}
""",
    "precedence": """\
#![warn(clippy::precedence)]
#![allow(clippy::eq_op, clippy::identity_op, clippy::no_effect, clippy::erasing_op, clippy::if_same_then_else)]

fn takes(_: i32) {}

fn shift_with_multiplicative() {
    1 << 2 * 3;
    //~^ precedence
    8 >> 4 / 2;
    //~^ precedence
    1 << 5 % 3;
    //~^ precedence
    1 * 2 << 3;
    //~^ precedence
    16 / 2 >> 3;
    //~^ precedence
    4 % 3 << 1;
    //~^ precedence
}

fn bitwise_with_arithmetic() {
    1 & 2 * 3;
    //~^ precedence
    1 | 2 / 3;
    //~^ precedence
    1 ^ 2 % 3;
    //~^ precedence
    1 * 2 & 3;
    //~^ precedence
    1 - 2 | 3;
    //~^ precedence
    1 / 2 ^ 3;
    //~^ precedence
}

fn arithmetic_on_both_sides() {
    1 + 2 & 3 * 4;
    //~^ precedence
    1 << 2 & 3 + 4;
    //~^ precedence
}

fn nested() {
    let _ = (1 << 2 + 3) == 0;
    //~^ precedence
    let _ = 1 + (2 & 3 - 1);
    //~^ precedence
    let _ = 1 | 2 & 3 * 4;
    //~^ precedence
    takes(1 + (2 << 3 * 4));
    //~^ precedence
    let _ = [1 << 2 % 3, 4];
    //~^ precedence
    let _ = if true { 1 } else { 2 >> 1 * 1 };
    //~^ precedence
    let _ = 1 << 2 - 3 << 4;
    //~^ precedence
}

fn parenthesised() {
    (1 << 2) + 3;
    1 << (2 + 3);
    (1 & 2) * 3;
    1 | (2 / 3);
    1 << 2 << 3;
    1 + 2 * 3;
}
""",
    "precedence_bits": """\
#![warn(clippy::precedence_bits)]
#![allow(clippy::eq_op, clippy::identity_op, clippy::no_effect, clippy::erasing_op, clippy::precedence)]

fn takes(_: i32) {}

fn outermost() {
    0x0F00 & 0x00F0 << 4;
    //~^ precedence_bits
    0x0F00 | 0xF000 >> 4;
    //~^ precedence_bits
    0x0F00 ^ 1 << 4;
    //~^ precedence_bits
    1 << 4 & 0x0F00;
    //~^ precedence_bits
    1 >> 4 | 0x0F00;
    //~^ precedence_bits
    1 << 4 ^ 0x0F00;
    //~^ precedence_bits
    1 << 2 & 3 >> 4;
    //~^ precedence_bits
}

fn nested() {
    let _ = (0x0F00 & 0x00F0 << 4) == 0;
    //~^ precedence_bits
    let _ = 1 + (2 & 3 << 4);
    //~^ precedence_bits
    takes(1 + (2 | 3 >> 4));
    //~^ precedence_bits
    let _ = [1 ^ 2 << 3, 4];
    //~^ precedence_bits
    let _ = if true { 1 } else { 2 & 3 >> 1 };
    //~^ precedence_bits
    let _ = 1 | 2 & 3 << 4;
    //~^ precedence_bits
}

fn same_start() {
    // Clippy reports each of these once per matching operation, so two diagnostics start at the same column
    let _ = 1 ^ 2 ^ 3 >> 7 ^ 4 >> 16;
    //~^ precedence_bits
    //~| precedence_bits
    let _ = 1 >> 1 ^ 2 ^ 3 ^ 4 >> 2;
    //~^ precedence_bits
    //~| precedence_bits
}

fn arithmetic_inside_a_shift() {
    // the outer `&` has a shift operand and no arithmetic one; the shift's own `+` is the `precedence` lint's
    1 & 2 << 3 + 4;
    //~^ precedence_bits
}

fn arithmetic_makes_it_precedence() {
    // an arithmetic operand makes it the `precedence` lint's, not `precedence_bits`'
    1 << 2 & 3 + 4;
    1 & 2 + 3;
    1 << 2 + 3;
    1 * 2 & 3 << 4;
}

fn parenthesised() {
    (1 & 2) << 3;
    1 & (2 << 3);
    1 << 2 << 3;
    1 & 2 & 3;
}
""",
    "redundant_static_lifetimes": """\
#![warn(clippy::redundant_static_lifetimes)]

fn take(_: &str) {}

const FN_POINTER: fn(&'static str) = take;
const FN_POINTER_RETURN: fn() -> &'static str = || "";
const BOXED: Option<Box<dyn Fn(&'static str)>> = None;
const REFERENCE_TO_DYN: &'static dyn Send = &0;
const REFERENCE_TO_REFERENCE: &'static &'static str = &"";
//~^ redundant_static_lifetimes
const ARRAY_OF_REFERENCES: [&'static str; 1] = [""];
//~^ redundant_static_lifetimes
const TUPLE_OF_REFERENCES: (&'static str, u8) = ("", 0);
//~^ redundant_static_lifetimes
const GENERIC_ARGUMENT: Option<&'static str> = None;
const LOCAL: usize = {
    let _: &'static str = "";
    0
};
""",
    "unnested_or_patterns": """\
#![warn(clippy::unnested_or_patterns)]

struct T5(u8, u8, u8, u8, u8);
struct T6(u8, u8, u8, u8, u8, u8);

fn tuples(a: (u8, u8, u8, u8, u8), b: (u8, u8, u8, u8, u8, u8)) {
    if let (1, 2, 3, 4, 5) | (1, 2, 3, 4, 6) = a {}
    //~^ unnested_or_patterns
    if let (1, 2, 3, 4, 5, 6) | (1, 2, 3, 4, 5, 7) = b {}
    //~^ unnested_or_patterns
    // two elements differ: not nestable
    if let (1, 2, 3, 4, 5) | (1, 2, 3, 6, 6) = a {}
}

fn slices(a: &[u8], b: &[u8]) {
    if let [1, 2, 3, 4, 5] | [1, 2, 3, 4, 6] = a {}
    //~^ unnested_or_patterns
    if let [1, 2, 3, 4, 5, 6] | [1, 2, 3, 4, 5, 7] = b {}
    //~^ unnested_or_patterns
    if let [1, 2, 3, 4, 5] | [1, 2, 3, 6, 6] = a {}
}

fn tuple_structs(a: T5, b: T6) {
    if let T5(1, 2, 3, 4, 5) | T5(1, 2, 3, 4, 6) = a {}
    //~^ unnested_or_patterns
    if let T6(1, 2, 3, 4, 5, 6) | T6(1, 2, 3, 4, 5, 7) = b {}
    //~^ unnested_or_patterns
    if let T5(1, 2, 3, 4, 5) | T5(1, 2, 3, 6, 6) = a {}
}
""",
}
BASE_ARGUMENTS = ["--error-format=json", "-Aunused", "-Ainternal_features", "-Zui-testing",
                  "-Zdeduplicate-diagnostics=no"]
DIRECTIVE = re.compile(r"^//@\s*([\w-]+)\s*(?::\s*(.*?))?\s*$")
ANNOTATION = re.compile(r"//~(\^+|\||v+)?\s*([A-Za-z_]\w*(?:::\w+)*)?")
LEVELS = {"ERROR", "WARN", "HELP", "NOTE", "SUGGESTION"}


def directives(text: str) -> list[tuple[str, str]]:
    return [(match.group(1), match.group(2) or "") for match in map(DIRECTIVE.match, text.splitlines()) if match]


def crate_type(text: str) -> list[str]:
    """ui_test 0.30.7's `default_per_file_config`: proc-macro, test, bin (no flag) or lib from the contents."""
    if re.search(r"#\[proc_macro(_derive|_attribute)?[\](]", text):
        return ["--crate-type=proc-macro"]
    if "#[test]" in text:
        return ["--test"]
    if re.search(r"fn main\(\)|#\[start\]", text):
        return []
    return ["--crate-type=lib"]


def edition(text: str) -> str:
    return next((value for key, value in directives(text) if key == "edition"), "2024")


def aux_files(test: str, text: str) -> list[str]:
    return [(pathlib.PurePosixPath(test).parent / "auxiliary" / value).as_posix()
            for key, value in directives(text) if key == "aux-build"]


def environment(conf_dir: str | None) -> dict:
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("CLIPPY_", "CARGO_")) and key not in ("RUSTFLAGS", "RUSTC_WRAPPER")}
    env.update(RUSTC_BOOTSTRAP="1", RUSTC_ICE="0")
    if conf_dir is not None:
        env["CLIPPY_CONF_DIR"] = conf_dir
    return env


def diagnostics(stderr: str) -> list[dict]:
    """Top-level errors and warnings rustc printed as JSON, minus its `aborting` / `N warnings emitted` summaries."""
    found = []
    for line in stderr.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("$message_type", "diagnostic") != "diagnostic" or message["level"] not in ("error", "warning"):
            continue
        primary = [span for span in message["spans"] if span["is_primary"]]
        if not primary:
            continue
        code = (message.get("code") or {}).get("code") or "?"
        found.append({"code": code[len("clippy::"):] if code.startswith("clippy::") else code,
                      "file": primary[0]["file_name"], "line": primary[0]["line_start"],
                      "column": primary[0]["column_start"], "end": (primary[0]["line_end"], primary[0]["column_end"]),
                      "level": message["level"], "message": message["message"]})
    return found


def dependencies(checkout: pathlib.Path, scratch: pathlib.Path) -> list[str]:
    """ui_test 0.30.7's DependencyBuilder: builds rust-clippy's clippy_test_deps for the host target (from outside
    the checkout, so its `.cargo/config.toml` nightly flags stay out) and returns `--extern` for each direct
    dependency's target artifacts and `-L` for every artifact directory."""
    manifest = checkout / "clippy_test_deps" / "Cargo.toml"
    host = next(line.split()[1] for line in subprocess.run(["rustc", f"+{common.TOOLCHAIN}", "-vV"],
                                                           capture_output=True, text=True, check=True)
                .stdout.splitlines() if line.startswith("host: "))
    target_dir = scratch / "clippy_test_deps"
    target_dir.mkdir(parents=True, exist_ok=True)
    env = {key: value for key, value in os.environ.items() if key not in ("RUSTFLAGS", "RUSTC_WRAPPER")}
    env["CARGO_TARGET_DIR"] = str(target_dir)
    build = subprocess.run(["cargo", f"+{common.TOOLCHAIN}", "build", "--manifest-path", str(manifest), "--locked",
                            f"--target={host}", "--message-format=json"], cwd=scratch, env=env,
                           capture_output=True, text=True)
    if build.returncode != 0:
        raise SystemExit(f"building {manifest} failed:\n{build.stderr}")
    metadata = json.loads(subprocess.run(["cargo", f"+{common.TOOLCHAIN}", "metadata", "--manifest-path",
                                          str(manifest), "--locked", "--format-version=1", "--no-deps"],
                                         cwd=scratch, env=env, capture_output=True, text=True, check=True).stdout)
    root = metadata["packages"][0]
    direct = {(dependency.get("rename") or dependency["name"]).replace("-", "_")
              for dependency in root["dependencies"] if dependency["kind"] is None}
    externs: dict[str, list[str]] = {}
    paths: set[str] = set()
    for line in build.stdout.splitlines():
        message = json.loads(line) if line.startswith("{") else {}
        if message.get("reason") == "build-script-executed":
            paths |= set(message["linked_paths"])
        if message.get("reason") != "compiler-artifact" or not {"lib", "rlib", "proc-macro"} & set(
                message["target"]["crate_types"]):
            continue
        files = [pathlib.Path(file) for file in message["filenames"]]
        if "proc-macro" not in message["target"]["crate_types"] and files[0].parent.parent.parent.name != host:
            continue  # a host build of a dependency (for a build script or proc macro)
        paths |= {str(file.parent) for file in files}
        name = message["target"]["name"].replace("-", "_")
        if name in direct:
            if name in externs:
                raise SystemExit(f"{name} was built twice for {host}")
            externs[name] = [str(file) for file in files]
    if sorted(externs) != sorted(direct):
        raise SystemExit(f"clippy_test_deps built {sorted(externs)}, it depends on {sorted(direct)}")
    arguments = [argument for name in sorted(externs) for file in externs[name]
                 for argument in ("--extern", f"{name}={file}")]
    return arguments + [argument for path in sorted(paths) for argument in ("-L", path)]


class Builder:
    def __init__(self, driver: pathlib.Path, fixtures: pathlib.Path, scratch: pathlib.Path, deps: list[str]):
        self.driver, self.fixtures, self.scratch, self.deps = driver, fixtures, scratch, deps
        self.built: dict[str, list[str]] = {}

    def run(self, arguments: list[str], conf_dir: str | None) -> subprocess.CompletedProcess:
        return subprocess.run([str(self.driver), *arguments], cwd=self.fixtures, env=environment(conf_dir),
                              capture_output=True, text=True)

    def aux(self, path: str) -> list[str]:
        """Builds aux crate `path` as ui_test does (its own directives, crate type from its contents, `--emit=link`)
        and returns the `--extern` / `-L` arguments for the test."""
        if path in self.built:
            return self.built[path]
        text = (self.fixtures / path).read_text()
        out = self.scratch / "aux"
        out.mkdir(parents=True, exist_ok=True)
        extra: list[str] = []
        for dependency in aux_files(path, text):
            extra += self.aux(dependency)
        arguments = [path, "--edition", edition(text), *BASE_ARGUMENTS, *crate_type(text), "--emit=link",
                     "--out-dir", str(out), *self.deps, *extra]
        run = self.run(arguments, "tests")
        if run.returncode != 0:
            raise SystemExit(f"aux build {path} failed:\n{run.stderr}")
        names = self.run([*arguments, "--print", "file-names"], "tests").stdout.split()
        name = pathlib.PurePosixPath(path).stem.replace("-", "_")
        self.built[path] = [argument for file in names for argument in ("--extern", f"{name}={out / file}", "-L",
                                                                       str(out))]
        return self.built[path]

    def test(self, path: str, extra: list[str]) -> list[dict]:
        text = (self.fixtures / path).read_text()
        flags = [value for key, value in directives(text) if key == "compile-flags"]
        externs = [argument for aux in aux_files(path, text) for argument in self.aux(aux)]
        ui_toml = path.startswith("tests/ui-toml/")
        out = self.scratch / "out" / path
        out.mkdir(parents=True, exist_ok=True)
        arguments = [path, "--edition", edition(text), "--emit=metadata", *BASE_ARGUMENTS,
                     *([] if ui_toml else crate_type(text)), *" ".join(flags).split(), *self.deps, *externs,
                     "--out-dir", str(out), *extra]
        run = self.run(arguments, str(pathlib.PurePosixPath(path).parent) if ui_toml else "tests")
        if run.returncode != 0 and not any(d["level"] == "error" for d in diagnostics(run.stderr)):
            raise SystemExit(f"clippy-driver failed on {path}:\n{run.stderr}")
        return diagnostics(run.stderr)


def annotations(text: str) -> list[tuple[str, int]]:
    """ui_test `//~` annotations as (lint or `?` for an `ERROR: message` one, line)."""
    found: list[tuple[str, int]] = []
    previous = None
    for number, line in enumerate(text.splitlines(), 1):
        for match in ANNOTATION.finditer(line):
            marker, code = match.group(1) or "", match.group(2) or "?"
            if marker == "|":
                target = previous
            elif marker.startswith("^"):
                target = number - len(marker)
            elif marker.startswith("v"):
                target = number + len(marker)
            else:
                target = number
            previous = target
            found.append(("?" if code in LEVELS else code.removeprefix("clippy::"), target))
    return found


def stderr_positions(text: str, path: str) -> list[tuple[int, int]]:
    """Each top-level diagnostic's primary `--> path:line:column` in an upstream `.stderr`."""
    positions = []
    header = False
    for line in text.splitlines():
        if re.match(r"^(error|warning)(\[\w+\])?: ", line):
            header = not re.match(r"^(error|warning): (aborting due to|\d+ warnings? emitted)", line)
            continue
        match = re.match(r"^\s*--> (.+):(\d+):(\d+)$", line)
        if header and match:
            header = False
            if match.group(1) == path:
                positions.append((int(match.group(2)), int(match.group(3))))
    return positions


def compare(path: str, text: str, stderr: str | None, run: list[dict], ported: set[str]) -> list[dict]:
    """The differences between the faithful run and the upstream expectations of test `path`."""
    own = [d for d in run if d["file"] == path]
    differences = []
    expected = Counter(annotations(text))
    got = Counter((d["code"], d["line"]) for d in own)
    matched = expected & got
    left_expected, left_got = expected - matched, got - matched
    for key in sorted(left_expected):
        if key[0] != "?":
            continue
        for candidate in sorted(left_got):
            take = min(left_expected[key], left_got[candidate]) if candidate[1] == key[1] else 0
            left_expected[key] -= take
            left_got[candidate] -= take
    for (code, line), count in sorted((+left_expected).items()):
        differences.append({"kind": "annotated, not reported", "lint": code, "line": line, "count": count,
                            "ported": code in ported})
    for (code, line), count in sorted((+left_got).items()):
        differences.append({"kind": "reported, not annotated", "lint": code, "line": line, "count": count,
                            "ported": code in ported})
    if stderr is None:
        expected_positions = Counter()
    else:
        expected_positions = Counter(stderr_positions(stderr, path))
    got_positions = Counter((d["line"], d["column"]) for d in own)
    codes = {(d["line"], d["column"]): d["code"] for d in own}
    for (line, column), count in sorted((expected_positions - got_positions).items()):
        differences.append({"kind": "in .stderr, not reported", "line": line, "column": column, "count": count})
    for (line, column), count in sorted((got_positions - expected_positions).items()):
        differences.append({"kind": "reported, not in .stderr", "lint": codes[(line, column)], "line": line,
                            "column": column, "count": count, "ported": codes[(line, column)] in ported})
    return differences


def fixtures(checkout: pathlib.Path, scratch: pathlib.Path) -> None:
    driver = common.check_toolchain()
    common.check_head(checkout, common.CLIPPY_COMMIT)
    lints = common.lints()
    if sorted(TESTS) != lints:
        raise SystemExit(f"TESTS covers {sorted(TESTS)}, the pack ports {lints}")
    target = HERE / "fixtures"
    shutil.rmtree(target, ignore_errors=True)
    for stale in ("aux", "out"):
        shutil.rmtree(scratch / stale, ignore_errors=True)
    tests = sorted({test for files in TESTS.values() for test in files})
    probes = {f"rerail/{name}.rs": text for name, text in PROBES.items()}
    copies = set(tests) | {"tests/clippy.toml"}
    pending = list(tests)
    while pending:
        test = pending.pop()
        auxes = aux_files(test, (checkout / test).read_text())
        copies |= set(auxes)
        pending += auxes
        if test.startswith("tests/ui-toml/"):
            copies.add((pathlib.PurePosixPath(test).parent / "clippy.toml").as_posix())
    for copy in sorted(copies):
        (target / copy).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(checkout / copy, target / copy)
    for path, text in probes.items():
        (target / path).parent.mkdir(parents=True, exist_ok=True)
        (target / path).write_text(text)
    tests = sorted(tests + list(probes))
    builder = Builder(driver, target, scratch, dependencies(checkout, scratch))
    findings = {}
    check = {}
    ported = set(lints)
    record_flags = [argument for lint in lints for argument in ("-W", f"clippy::{lint}")]
    for test in tests:
        text = (target / test).read_text()
        stderr = checkout / pathlib.PurePosixPath(test).with_suffix(".stderr")
        faithful = builder.test(test, [])
        differences = compare(test, text, stderr.read_text() if stderr.exists() else None, faithful, ported)
        if test in probes:  # ours have no `.stderr`: only the `//~` annotations are checked
            differences = [d for d in differences if "stderr" not in d["kind"]]
        if differences:
            check[test] = differences
        for d in builder.test(test, record_flags):
            if d["code"] in ported and d["file"] == test:
                key = (d["code"], test, d["line"], d["column"], d["end"])
                findings[key] = {"rule": d["code"], "path": test, "line": d["line"], "column": d["column"]}
    units = {test: {"rules": sorted(lint for lint, files in TESTS.items() if test in files)}
             for test in tests if test not in probes}
    common.write(HERE / "fixtures.expected.json",
                 {"kind": "fixtures", "origin": common.CLIPPY_ORIGIN, "rules": lints, "files": tests,
                  "units": units},
                 list(findings.values()))
    (HERE / "fixtures.upstream-check.json").write_text(json.dumps(
        {"clippy": common.CLIPPY_VERSION, "origin": common.CLIPPY_ORIGIN,
         "differences": check}, indent=1, sort_keys=True) + "\n")
    common.pack_fixtures.written(HERE / "fixtures.upstream-check.json")
    print(f"{len(tests)} tests ({len(probes)} ours), {len(copies)} files copied, {len(findings)} findings; per lint:")
    for lint, count in sorted(Counter(key[0] for key in findings).items()):
        print(f"  {lint}: {count}")
    for test, differences in sorted(check.items()):
        for difference in differences:
            print(f"  {test}: {difference}")


def main() -> None:
    if sys.argv[1] == "fixtures":
        fixtures(pathlib.Path(sys.argv[2]).resolve(), pathlib.Path(sys.argv[3]).resolve())
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
