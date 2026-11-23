#!/usr/bin/env python3
"""Refuses a commit whose parallel work reaches rayon's global pool (decision 0442 step 5: a job pool per machine).

Each machine has its own pool (`rerail_context::Cores`). Rayon's functions reach the pool of the thread they run on,
and the process's global pool from any other thread: a test's, a service's, a Node thread, a caller's. So every
parallel loop (`par_iter`, `par_chunks_mut`, `into_par_iter`, ...) and every rayon call that starts work (`spawn`,
`join`, `scope`, `broadcast`, `current_num_threads`) is inside one of the machine's entry points, which put it on the
caller's machine's pool in the caller's run:

  on_cores(|| ...)            a loop, its items split over the machine's pool
  cores_spawn(...)            a job
  cores_spawn_fifo(...)       a job, first in first out
  cores_scope(|s| ...)        jobs that borrow the caller's data

The global pool is never built (`build_global`), and a pool of its own (`ThreadPoolBuilder`) is made only where
`POOLS` says why; in those files, work inside the pool's `install` is on it. Test modules and test files are left out: a test may make what it likes.

Usage: check_machine_pool.py
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BASES = ["native/src", "native/crates", "accel/src", "forest/src"]

# Where a pool other than the machine's is made, and why.
POOLS = {
    "native/crates/context/src/lib.rs": "the machine's pool itself (`Cores`)",
    "accel/src/exec.rs": "the device lanes' host pool and the submissions' slots: the machine's (`Pools`)",
    "native/crates/logic_core/src/code_rule_dsl/logic/costs.rs": "the cost model's training, a few threads at a low priority, for one fit",
    "forest/src/mldb/plugins/jml/randomforest_procedure.rs": "the forest's training bounds the bags trained at once, inside the cost model's training pool",
}

SITE = re.compile(
    r"\.(par_iter|par_iter_mut|par_chunks|par_chunks_mut|into_par_iter|par_bridge|par_sort\w*|par_extend|par_drain)\("
    r"|\brayon::(spawn|spawn_fifo|join|join_context|scope|scope_fifo|in_place_scope|in_place_scope_fifo|broadcast|spawn_broadcast|current_num_threads)\b"
)
ENTRY = re.compile(r"\b(on_cores|cores_spawn|cores_spawn_fifo|cores_scope)\(")


def is_test(path):
    name = os.path.basename(path)
    return "/tests/" in path or "/examples/" in path or "/benches/" in path or name in ("tests.rs", "test.rs") or name.endswith(("_tests.rs", "_bench.rs")) or name.startswith("test_")


def code_mask(text):
    """1 where a char is code (not in a string or a comment)."""
    m = bytearray(len(text))
    i, n = 0, len(text)
    while i < n:
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        r = re.match(r'r(#*)"', text[i:]) if text[i] == "r" and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")) else None
        if r:
            j = text.find('"' + r.group(1), i + len(r.group(0)))
            i = n if j < 0 else j + 1 + len(r.group(1))
            continue
        if text[i] == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            i = j + 1
            continue
        if text[i] == "'":
            c = re.match(r"'(\\.|[^\\'])'", text[i:])
            if c:
                i += len(c.group(0))
                continue
        m[i] = 1
        i += 1
    return m


def spans(text, mask, opener, close_at_paren=True):
    """The spans that start at each match of `opener` and end at its matching bracket."""
    out = []
    for m in opener.finditer(text):
        if not mask[m.start()]:
            continue
        open_c = text[m.end() - 1]
        close_c = {"(": ")", "{": "}"}[open_c]
        depth, j = 1, m.end()
        while j < len(text) and depth:
            if mask[j]:
                if text[j] == open_c:
                    depth += 1
                elif text[j] == close_c:
                    depth -= 1
            j += 1
        out.append((m.start(), j))
    return out


TEST_MOD = re.compile(r"#\[cfg\((?:test|all\(test[^\]]*|any\(test[^\]]*)\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{")


def main():
    bad = []
    for base in BASES:
        for root, _, files in os.walk(os.path.join(ROOT, base)):
            if "/target" in root:
                continue
            for f in files:
                if not f.endswith(".rs"):
                    continue
                path = os.path.join(root, f)
                rel = os.path.relpath(path, ROOT)
                if is_test(rel):
                    continue
                text = open(path, errors="ignore").read()
                mask = code_mask(text)
                tests = spans(text, mask, TEST_MOD)
                inside = spans(text, mask, ENTRY) + (spans(text, mask, re.compile(r"\.install\(")) if rel in POOLS else [])
                line = lambda p: text.count("\n", 0, p) + 1
                for m in SITE.finditer(text):
                    p = m.start()
                    if not mask[p] or any(a <= p < b for a, b in tests) or rel == "native/crates/context/src/lib.rs":
                        continue
                    if not any(a <= p < b for a, b in inside):
                        bad.append(f"{rel}:{line(p)}: `{m.group(0)}` outside the machine's pool (wrap it in `on_cores(|| ...)`, or use `cores_spawn`/`cores_scope`)")
                for m in re.finditer(r"\bbuild_global\b", text):
                    if mask[m.start()] and not any(a <= m.start() < b for a, b in tests):
                        bad.append(f"{rel}:{line(m.start())}: rayon's global pool is never built: a machine's pool is (`rerail_context::Cores`)")
                for m in re.finditer(r"\bThreadPoolBuilder\b", text):
                    if mask[m.start()] and not any(a <= m.start() < b for a, b in tests) and rel not in POOLS:
                        bad.append(f"{rel}:{line(m.start())}: a pool of its own: the machine's is the one (list it in POOLS, with why, if it must be)")
    if bad:
        print("check_machine_pool: parallel work outside the machine's pool (decision 0442):")
        for b in bad:
            print("  " + b)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
