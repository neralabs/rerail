#!/usr/bin/env python3
"""Refuses a commit that adds a cache beside the engine's facts (decision 0104).

Reads the lines a commit adds (the index against HEAD; with --all, the working tree against HEAD) and refuses:

- a `static` behind a lock or a cell that holds more than a number, a flag or an instant (what
  `logic::facts_only::maps` finds), in the engine's run path, unless `facts_only.rs` lists it;
- a `CREATE TABLE` in the code that keeps a run's state, unless `facts_only.rs` lists the table;
- a new entry in `facts_only.rs`'s lists of state or of caches not yet facts, unless the commit command says why
  with `RERAIL_STATE_LISTED=<reason>`: listing a map as state is how a cache gets in unseen.

A repeated computation is fixed by flow (computed once where first needed and passed on, or derived once as a
fact), never by a memo map behind the callers. `facts_only`'s tests check the whole tree; this checks a commit in
under a second, before it is made.

Usage: check-no-new-caches.py [--all] [--command '<the git commit command>']
"""
import re
import subprocess
import sys

FACTS_ONLY = "native/src/code_rule_dsl/logic/facts_only.rs"
MAP_DIRS = ("native/src/code_rule_dsl/logic/", "native/src/content_store/", "native/src/code_rule_dsl/text.rs")
TABLE_DIRS = MAP_DIRS[:2] + ("native/src/code_rule_dsl/stats", "native/src/code_rule_dsl/slice")
HEAD = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?static\s+([A-Z_][A-Z_0-9]*)\s*:")
HELD = re.compile(r"(Mutex|RwLock|RefCell|OnceLock|LazyLock|Cell)\s*<")
PLAIN = re.compile(
    r"(Mutex|RwLock|RefCell|OnceLock|LazyLock|Cell)\s*<\s*(\(\s*\)|bool|u8|u16|u32|u64|usize|i32|i64|f64|"
    r"std::time::Instant|Instant|std::time::Duration|Option<\s*(usize|u64|i64|bool|f64|std::time::Instant)\s*>|"
    r"\(\s*(Option<usize>|usize|u64|f64)\s*,\s*(usize|u64|f64)\s*\))\s*>"
)
TABLE = re.compile(r"CREATE\s+(?:TEMP\s+|TEMPORARY\s+)?TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?(?:[a-z_]+\.)?([A-Za-z_][A-Za-z_0-9]*)")
LIST_ENTRY = re.compile(r'^\s*\(\s*"[^"]+"\s*,\s*"([A-Z_][A-Z_0-9]*)"')


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def is_test(path):
    name = path.rsplit("/", 1)[-1]
    return "/tests/" in path or name == "tests.rs" or name.endswith("_tests.rs")


def main():
    every = "--all" in sys.argv
    command = sys.argv[sys.argv.index("--command") + 1] if "--command" in sys.argv else ""
    diff = git("diff", "HEAD" if every else "--cached", "-U0", "--no-color", "--", "native/src")
    added = {}  # path -> [new line numbers]
    path = None
    for line in diff.splitlines():
        if line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else None
        elif line.startswith("@@") and path:
            m = re.search(r"\+(\d+)(?:,(\d+))?", line)
            start, count = int(m.group(1)), int(m.group(2) or "1")
            added.setdefault(path, []).extend(range(start, start + count))
    if not added:
        return 0

    def text(p):
        try:
            return open(p).read() if every else git("show", f":{p}")
        except (OSError, subprocess.CalledProcessError):
            return ""

    listed = text(FACTS_ONLY)
    problems = []
    for p, numbers in sorted(added.items()):
        if not p.endswith(".rs") or is_test(p):
            continue
        lines = text(p).split("\n")
        for n in numbers:
            if n - 1 >= len(lines):
                continue
            line = lines[n - 1]
            if p.startswith(MAP_DIRS):
                c = HEAD.match(line)
                if c:
                    decl = " ".join(lines[n - 1 : n + 3]).split(" = ")[0]
                    if HELD.search(decl) and not PLAIN.search(decl) and f'"{c.group(1)}"' not in listed:
                        problems.append(f"{p}:{n}: a new static map `{c.group(1)}`. Fix the flow (compute it once and pass it on, or make it a fact); never a memo map.")
            if p.startswith(TABLE_DIRS):
                for t in TABLE.finditer(line):
                    if f'"{t.group(1)}"' not in listed:
                        problems.append(f"{p}:{n}: a new table `{t.group(1)}`. A cache is rows of a relation, not a table of its own.")
            if p == FACTS_ONLY and "RERAIL_STATE_LISTED=" not in command:
                e = LIST_ENTRY.match(line)
                if e:
                    problems.append(f"{p}:{n}: `{e.group(1)}` newly listed. Say why in the command (RERAIL_STATE_LISTED=<reason>), or fix the flow instead.")
    if problems:
        print("Refused: the commit adds a cache beside the facts (decision 0104).", file=sys.stderr)
        for line in problems:
            print("  " + line, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
