#!/usr/bin/env python3
"""Refuses a commit that adds a thread-local the run-state list does not name (decision 0192).

The engine's run context lives in thread-locals, and a job on another thread sees it only if it is carried
(`native/src/code_rule_dsl/logic/run_state.rs`). Every thread-local outside the tests is listed there with its class:
carried, the thread's own, one call's, the outermost run's, or not carried yet. An unlisted one is state no one has
decided about: a nested run on another thread would silently not see it.

Reads the files a commit changes (the index; with --all, the working tree) under `native/src` and
`native/crates/*/src`, finds their `thread_local!` statics outside the tests (a block under `#[cfg(test)]` or
`#[cfg(any(test, ...))]`, or a test file), and refuses each one `run_state.rs`'s `STATE` (as it will be committed)
does not list. `run_state`'s own tests check the whole tree; this checks a commit in well under a second.

Usage: check-thread-state.py [--all]
"""
import re
import subprocess
import sys

RUN_STATE = "native/src/code_rule_dsl/logic/run_state.rs"
ENTRY = re.compile(r'^\s*\(\s*"([^"]+)"\s*,\s*"([A-Z_][A-Z_0-9]*)"\s*,\s*(\w+)\s*,')
STATIC = re.compile(r'(?:^|[\s{])static\s+([A-Z_][A-Z0-9_]*)\s*:')


def git(*args, check=True):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=check).stdout


def is_test(path):
    name = path.rsplit("/", 1)[-1]
    return "/tests/" in path or name == "tests.rs" or name.endswith("_tests.rs")


def watched(path):
    if not path.endswith(".rs") or is_test(path):
        return False
    return path.startswith("native/src/") or re.match(r"native/crates/[^/]+/src/", path) is not None


def thread_locals(text):
    out = []
    for m in re.finditer(r"thread_local!\s*\{", text):
        attr = text[: m.start()].rstrip().rsplit("\n", 1)[-1].strip()
        i, depth = m.end(), 1
        while depth and i < len(text):
            depth += {"{": 1, "}": -1}.get(text[i], 0)
            i += 1
        if attr.startswith("#[cfg") and "test" in attr:
            continue
        out += STATIC.findall(text[m.end() : i - 1])
    return out


def main():
    every = "--all" in sys.argv
    changed = git("diff", "HEAD" if every else "--cached", "--name-only", "--diff-filter=AMR").split()
    files = [p for p in changed if watched(p)]
    if not files:
        return 0
    read = (lambda p: open(p, errors="replace").read()) if every else (lambda p: git("show", f":{p}", check=False))
    listed = set()
    for line in read(RUN_STATE).splitlines():
        m = ENTRY.match(line)
        if m:
            listed.add((m.group(1), m.group(2)))
    missing = []
    for p in files:
        rel = p[len("native/") :]
        for name in thread_locals(read(p)):
            if (rel, name) not in listed:
                missing.append(f"{p}: {name}")
    if missing:
        print("check-thread-state: thread-locals not listed in " + RUN_STATE + " (STATE):", file=sys.stderr)
        for m in missing:
            print("  " + m, file=sys.stderr)
        print(
            "List each with its class: is it the run's context, which a job on another thread must carry "
            "(Carried, with a carrier), or correct on any thread (Thread, Call, Outermost)? See run_state's docs.",
            file=sys.stderr,
        )
        return 2  # a PreToolUse hook blocks on 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
