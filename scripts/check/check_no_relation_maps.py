#!/usr/bin/env python3
"""Refuses a commit that adds a thread-local or process-wide collection of relation data (decision 0478, R7).

What a relation has (its rows, dead rows, indexes, orders, statistics, count tables) is read where it is, in its
block, by the CPU and the GPU alike; what is derived once for a symbol goes in the one rolling cache (0478 §1-§4).
A `static` holding a map, a set or a vector of such data, in a `thread_local!` or for the process, is the structure
0478 removed: it keeps a relation's pages by a thread's timing (Q314), and holds a second copy beside the block's.

Reads the lines a commit adds (the index against HEAD, and in a merge against every parent; with --all, the working
tree against HEAD) in the engine's data paths, and refuses a `static` whose type holds a collection (`HashMap`,
`HashSet`, `BTreeMap`, `BTreeSet`, `Vec`, `VecDeque`, `HashTable`, `DashMap`, `IndexMap`). It passes only when it or
the line above says which allowed kind it is:

    // state: bookkeeping          what is in flight: waves, waiting strata, a last report (the user, 2026-11-19:
                                   "Bookkeeping / scoreboards / schedules can be on the CPU for now")
    // state: not relation data    settings, names, compiled code: nothing a relation has or a reader reads instead
    // state (0478 step N)         an existing one, listed in 0478's inventory, that step N removes

Usage: check_no_relation_maps.py [--all]
"""
import re
import subprocess
import sys

DIRS = (
    "native/src/code_rule_dsl/logic/native/",
    "native/src/code_rule_dsl/facts/",
    "native/src/code_rule_dsl/transducer/packed/",
    "native/crates/device_metal/src/code_rule_dsl/device_metal.rs",
    "native/src/code_rule_dsl/residency/",
    "native/src/code_rule_dsl/text/",
    "native/src/content_store/",
    "native/crates/metal/src/",
    "native/crates/slabs/src/",
    "native/crates/blocks/src/",
    "native/crates/device_metal/src/",
    "accel/src/gpu/",
    "accel/gpu/src/gpu/",
)
# A directory of the engine's module tree is in every crate that keeps a part of it (native/crates/*/src/...), as well
# as in the engine's own native/src: each crate's sources are in its own src/.
DIRS = DIRS + tuple(
    "native/crates/*/src/" + d[len("native/src/"):] + ("*" if d.endswith("/") else "") for d in DIRS if d.startswith("native/src/")
)
STATIC = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?static\s+(?:mut\s+)?[A-Z_][A-Z0-9_]*\s*:\s*(?P<ty>.*)$")
COLLECTION = re.compile(r"\b(?:Fx)?Hash(?:Map|Set)\b|\bBTree(?:Map|Set)\b|\bVec(?:Deque)?\s*<|\bHashTable\b|\bDashMap\b|\bIndexMap\b")
MARK = re.compile(r"state: (bookkeeping|not relation data)|state \(0478 step \d+\)")


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def is_test(path):
    name = path.rsplit("/", 1)[-1]
    return "/tests/" in path or name == "tests.rs" or name.endswith("_tests.rs") or "/examples/" in path


def refused(lines, n):
    """Whether line `n` (1-based) of `lines` declares an unmarked static collection."""
    line = lines[n - 1]
    m = STATIC.match(line.split("//")[0])
    if not m:
        return False
    # The type may go on past the line (a long generic): the declaration up to its `=` or `;`.
    ty, k = m.group("ty"), n
    while not re.search(r"[=;]", ty) and k < len(lines):
        ty += " " + lines[k].split("//")[0]
        k += 1
    ty = re.split(r"[=;]", ty, maxsplit=1)[0]
    if not COLLECTION.search(ty):
        return False
    above = lines[n - 2] if n >= 2 else ""
    return not (MARK.search(line) or MARK.search(above))


def main():
    every = "--all" in sys.argv
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
        if not p.endswith(".rs") or is_test(p):
            continue
        lines = text(p).split("\n")
        for n in sorted(numbers):
            if n - 1 < len(lines) and refused(lines, n):
                problems.append(f"{p}:{n}: {lines[n - 1].strip()[:110]}")
    if problems:
        print("Refused: the commit adds a thread-local or process-wide collection (decision 0478, R7). What a relation", file=sys.stderr)
        print("has is in its block, and what is derived for a symbol in the one cache. If it holds no relation data,", file=sys.stderr)
        print("say which kind on the line or the one above: `state: bookkeeping`, `state: not relation data`, or", file=sys.stderr)
        print("`state (0478 step N)`.", file=sys.stderr)
        for line in problems:
            print("  " + line, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
