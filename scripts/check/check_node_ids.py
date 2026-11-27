#!/usr/bin/env python3
"""Refuses a shift by 32 outside the one definition of the ids' encodings (plan "Chunk node numbers").

A node leaves a chunk's numbering as a located id (`key << 32 | low`), a document's key, or a place (`-2 - (path << 32 |
line)`). Their splits, joins and the test of a location value (a node's located id, or a document's key) are defined
once, in `rerail_blocks::located`; a chunk's own ids are decoded through its directory. A `>> 32`, `<< 32` or
`1 << 32` written by hand in a crate that handles ids is the slip the plan's numbering would turn into a wrong
document: it passes only marked as bits that are no id, on its line or the line above:

    // bits: <what they are>        a hash's mix, a pair of row numbers in a word, an offset's halves

Comments and `#[cfg(test)]` items (whose ids are written by hand on purpose) are not read.

Reads the lines a commit adds (the index against HEAD); with --all, every line of the crates.

Usage: check_node_ids.py [--all]
"""
import re
import subprocess
import sys

CRATES = (
    "native/crates/facts/src/",
    "native/crates/facts_base/src/",
    "native/crates/logic/src/",
    "native/crates/corpus_memo/src/",
    "native/crates/corpus_prep/src/",
    "native/crates/run/src/",
    "native/crates/sessions_base/src/",
    "native/crates/plan_search/src/",
    "native/crates/setfix/src/",
    "native/crates/planner/src/",
    "native/crates/plan_base/src/",
    "native/crates/sessions/src/",
    "native/crates/lang_python/src/",
)
DEFINITION = "native/crates/blocks/src/located.rs"
SHIFT = re.compile(r">>\s*32\b|<<\s*32\b|\b1(i64|u64)?\s*<<\s*32\b")
MARK = "// bits:"


def code_lines(text):
    """Each line's number and its code (comments cut), outside `#[cfg(test)]` items."""
    lines = text.split("\n")
    skip_depth = None
    pending_test = False
    depth = 0
    out = []
    for i, line in enumerate(lines, 1):
        code = re.sub(r'"(\\.|[^"\\])*"', '""', line)
        code = code.split("//", 1)[0]
        stripped = code.strip()
        if skip_depth is None and stripped.startswith("#[cfg(test)]"):
            pending_test = True
            continue
        opens, closes = code.count("{"), code.count("}")
        if pending_test and skip_depth is None:
            if stripped.startswith("#["):
                continue
            pending_test = False
            if opens > closes:
                skip_depth = depth
                depth += opens - closes
                continue
            # A one-line item (`mod tests;`, a `use`): skipped alone.
            depth += opens - closes
            continue
        depth += opens - closes
        if skip_depth is not None:
            if depth <= skip_depth:
                skip_depth = None
            continue
        out.append((i, code, line))
    return out


def files():
    out = subprocess.run(["git", "ls-files", *CRATES], capture_output=True, text=True, check=True).stdout.split()
    return [f for f in out if f.endswith(".rs") and not re.search(r"(^|/)(tests?|[a-z_]*_tests)\.rs$|/tests/", f)]


def added(path):
    """The line numbers the index adds to `path` against HEAD (all of them for a new file)."""
    diff = subprocess.run(["git", "diff", "--cached", "-U0", "HEAD", "--", path], capture_output=True, text=True).stdout
    nums = set()
    for m in re.finditer(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@", diff, re.M):
        start, n = int(m.group(1)), int(m.group(2) or 1)
        nums.update(range(start, start + n))
    return nums


def main():
    every = "--all" in sys.argv
    bad = []
    for path in files():
        if path == DEFINITION:
            continue
        try:
            text = open(path).read()
        except FileNotFoundError:
            continue
        want = None if every else added(path)
        if want is not None and not want:
            continue
        lines = text.split("\n")
        for i, code, line in code_lines(text):
            if want is not None and i not in want:
                continue
            if not SHIFT.search(code):
                continue
            if MARK in line or (i >= 2 and MARK in lines[i - 2]):
                continue
            bad.append(f"{path}:{i}: {line.strip()}")
    if bad:
        print("A shift by 32 outside rerail_blocks::located (plan \"Chunk node numbers\"): use its encodings, or mark")
        print("bits that are no id with `// bits: <what they are>`:")
        for b in bad:
            print("  " + b)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
