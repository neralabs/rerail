#!/usr/bin/env python3
"""Refuses a commit that adds a copy of a block or a tape into another data structure (decision 0353).

Data lives in one place, in one layout, and every reader (the CPU executor, the JIT, the Metal kernels, SQLite's
virtual tables) reads it there. A view is a read of the tape, not a relation. A missing accessor is written once as
a read (decision 0310), never worked around by copying the data into a shape a reader prefers.

Reads the lines a commit adds (the index against HEAD, and in a merge against every parent; with --all, the working
tree against HEAD) in the engine's data paths, and refuses a line that copies values somewhere else for a reader:
`copy_nonoverlapping`, `copy_from_slice`, `extend_from_slice`, an upload, a view expanded or pairs put into a chunk,
rows copied, or a row read into a `Vec`. Such a line passes only when it or the line above says which allowed kind
it is (decision 0353, "The rule"):

    // copy: load              bytes from disk into the exact bytes the readers read
    // copy: growth            a block that outgrew its space, moved once (named and counted)
    // copy: derived           a fact written once where it is computed (a head row, an axis's answer, a symbol)
    // copy: not block data    a vector of keys, plan state, text being built: nothing a reader reads instead of a block
    // copy (0353 step N)      an existing copy, listed in 0353, that step N removes (the list only shrinks)

Usage: check-no-block-copies.py [--all]
"""
import re
import subprocess
import sys

DIRS = (
    "native/src/code_rule_dsl/logic/native/",
    "native/src/code_rule_dsl/facts/",
    "native/src/code_rule_dsl/transducer/packed/",
    "native/src/code_rule_dsl/device_metal.rs",
    "native/crates/blocks/src/",
    "native/crates/facts/src/",
    "native/crates/device_metal/src/",
    "accel/src/gpu/",
)
COPY = re.compile(
    r"copy_nonoverlapping\s*\(|copy_from_slice\s*\(|extend_from_slice\s*\(|\bupload\s*\(|DevRel::upload|"
    r"expand_view\s*\(|put_pairs\s*\(|copy_rows\s*\(|\brows?\s*\([^;]*\)\s*\.to_vec\s*\(\)|\.row\s*\([^;]*\)\s*\.read\s*\(\)\s*\.to_vec"
)
MARK = re.compile(r"copy: (load|growth|derived|not block data)|copy \(0353 step \d+\)")


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def is_test(path):
    name = path.rsplit("/", 1)[-1]
    return "/tests/" in path or name == "tests.rs" or name.endswith("_tests.rs") or "/examples/" in path


def main():
    every = "--all" in sys.argv
    # A merge adds only the lines no parent has: what the merged branch brings was checked where it was made.
    try:
        parents = ["HEAD", *git("rev-parse", "-q", "--verify", "MERGE_HEAD").split()]
    except subprocess.CalledProcessError:
        parents = ["HEAD"]
    added = None
    for parent in parents:
        diff = git("diff", *([parent] if every else ["--cached", parent]), "-U0", "--no-color", "--", *DIRS)
        lines_added = {}
        path = None
        for line in diff.splitlines():
            if line.startswith("+++ "):
                path = line[6:] if line.startswith("+++ b/") else None
            elif line.startswith("@@") and path:
                m = re.search(r"\+(\d+)(?:,(\d+))?", line)
                start, count = int(m.group(1)), int(m.group(2) or "1")
                lines_added.setdefault(path, set()).update(range(start, start + count))
        added = lines_added if added is None else {
            p: added[p] & lines_added[p] for p in added.keys() & lines_added.keys() if added[p] & lines_added[p]
        }
    if not added:
        return 0

    def text(p):
        try:
            return open(p).read() if every else git("show", f":{p}")
        except (OSError, subprocess.CalledProcessError):
            return ""

    problems = []
    for p, numbers in sorted(added.items()):
        if not (p.endswith(".rs") or p.endswith(".metal")) or is_test(p):
            continue
        lines = text(p).split("\n")
        for n in sorted(numbers):
            if n - 1 >= len(lines):
                continue
            line = lines[n - 1]
            code = line.split("//")[0]
            if not COPY.search(code):
                continue
            above = lines[n - 2] if n >= 2 else ""
            if MARK.search(line) or MARK.search(above):
                continue
            problems.append(f"{p}:{n}: {line.strip()[:110]}")
    if problems:
        print("Refused: the commit adds a copy of a block or a tape (decision 0353). Read the data where it lies,", file=sys.stderr)
        print("through a read written once (0310). If it is not such a copy, say which kind on the line or the one", file=sys.stderr)
        print("above: `copy: load`, `copy: growth`, `copy: derived`, `copy: not block data`, or `copy (0353 step N)`.", file=sys.stderr)
        for line in problems:
            print("  " + line, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
