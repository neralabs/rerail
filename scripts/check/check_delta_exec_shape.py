#!/usr/bin/env python3
"""The delta executor's shape checks (decision 0686, the rebuild plan's §5).

Each milestone of the rebuild adds the mechanical checks it can make here: names the final shape has no place for,
and forms the agreed design rules out. A milestone's commit runs this beside check_no_block_copies.py, and its
checklist names the result. Run from the repository root:

    python3 scripts/check/check_delta_exec_shape.py

Exit 0 when every check holds; otherwise each failure is printed, with its file and line, and the exit is 1.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
CRATE = ROOT / "native/crates/delta_exec"
CHANGE = ROOT / "native/crates/blocks/src/change.rs"


def strip_rust(src: str) -> str:
    """Rust with its comments and string literals blanked, line breaks kept."""
    out, i, n = [], 0, len(src)
    while i < n:
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("\n" * src.count("\n", i, j))
            i = j
        elif src[i] == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            out.append('""')
            i = j + 1
        else:
            out.append(src[i])
            i += 1
    return "".join(out)


def strip_rerail(src: str) -> str:
    """A .rerail file with its comments and strings blanked, line breaks kept."""
    lines = []
    for line in src.split("\n"):
        line = re.sub(r'"(\\.|[^"\\])*"', '""', line)
        lines.append(line.split("//", 1)[0])
    return "\n".join(lines)


def hits(path: pathlib.Path, text: str, pattern: str, why: str, failures: list) -> None:
    for no, line in enumerate(text.split("\n"), 1):
        if re.search(pattern, line):
            failures.append(f"{path.relative_to(ROOT)}:{no}: {why}: {line.strip()}")


def m0(failures: list) -> None:
    """M0, the clean slate: what the first build had and the final shape has no place for."""
    forbidden = [
        (r"\bBoot\b", "a driver of fixed levels (the loop is B§8's, M8)"),
        (r"\bExec\b", "the first build's executor"),
        (r"\bRounds\b", "a second table type for a fixpoint's rounds"),
        (r"\bsettle\b", "a roll-up that rewrites rows (traces roll up geometrically, M1)"),
        (r"\bDb\b", "a store of its own beside the traces"),
        (r"with_prefix|\bk1_", "a level's program named by a prefix (the export is K-form, O6)"),
        (r"\bk_fn\b|\beval\b", "arithmetic in K0 (O2)"),
        (r"\bstratum\b", "strata stated per head (strata are files, O4)"),
        (r"change::body|\bPart::|\bBody::", "the k-way body operator (steps are separate operators, O1)"),
        (r"write_pair|write_diff|write_keys", "the first build's writes"),
    ]
    for path in sorted((CRATE / "src").rglob("*.rs")):
        code = strip_rust(path.read_text())
        for pattern, why in forbidden:
            hits(path, code, pattern, why, failures)
    for path in sorted(CRATE.rglob("*.rerail")):
        code = strip_rerail(path.read_text())
        hits(path, code, r"[\w)]\s*[-+*/%]\s*[\w(]", "arithmetic (K0 has none, O2)", failures)
        hits(path, code, r"\bstratum\s*\(", "a stated stratum (strata are files, O4)", failures)
    hits(CHANGE, strip_rust(CHANGE.read_text()), r"\bfn body\b", "change::body (O1)", failures)


def main() -> int:
    failures: list = []
    for check in (m0,):
        check(failures)
    for f in failures:
        print(f)
    if failures:
        print(f"{len(failures)} shape check(s) failed (decision 0686, the rebuild plan)")
        return 1
    print("delta_exec shape: every check holds (M0)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
