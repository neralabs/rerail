#!/usr/bin/env python3
"""Refuses a commit that lets the warm check (`RERAIL_WARM`, decision 0197) change what a run does.

The warm check only watches a run: it never changes how the run computes, or what it writes. This checks the tree
(in under a second) for two things:

- Who reads the mode. Only the places in READERS may call `warm()` or `warm_mode()`, name `Warm::`, or read the
  `RERAIL_WARM` variable, each as many times as it says. A comment naming the mode is not a read.
- What a warm run may still write. `profile::WARM_WRITES_ALLOWED` must be ALLOWED below, entry for entry: locks
  and timestamps, approved by the user. Changing either needs the user's approval; the two copies make a change
  to one alone fail here.

Usage: check-warm-mode.py [--all]   (the flag is accepted for the commit hook, which passes it; the tree is read)
"""
import re
import subprocess
import sys

PROFILE = "native/src/code_rule_dsl/logic/profile.rs"

# (file, calls) -> why it may read the mode. A new reader, or another read in a listed file, needs the user's approval.
READERS = {
    PROFILE: (None, "the mode's own implementation: the faults, the commit hook, the writer's scope"),
    "native/src/code_rule_dsl/logic/plan/corpus.rs": (1, "the maintained part's delta check: reads the deltas held in memory, and faults"),
    "native/src/code_rule_dsl/logic/plan/corpus/reuse.rs": (1, "`NOT_OUTPUTS`: the mode is left out of the cutoff's key"),
}

# The writes a warm run may make (`profile::WARM_WRITES_ALLOWED`): approved by the user (decision 0197).
ALLOWED = [
    "the run's hold on its plans",
    "the run's hold released",
    "a source's pin",
    "the combined program's pins released",
    "the run's manifest",
    "the journal's aged cursor",
]

READ = re.compile(r"(?<![A-Za-z0-9_.])(?:profile::)?warm(?:_mode)?\(\)|\bWarm::|\"RERAIL_WARM\"")
# Functions of the same name that are not the mode: GPU pipelines warmed before a timed run.
NOT_THE_MODE = re.compile(r"\bfn warm\(\)|(?:matcher_metal|metal_label_passes|label_passes)::warm\(\)")


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def code(line):
    """The line without its comment (a `//` outside a string, near enough for these names)."""
    out, quoted = [], False
    for i, c in enumerate(line):
        if c == '"' and (i == 0 or line[i - 1] != "\\"):
            quoted = not quoted
        if not quoted and line.startswith("//", i):
            break
        out.append(c)
    return "".join(out)


def main():
    # Paths are the repository's: run from its top, wherever called from.
    import os
    os.chdir(git("rev-parse", "--show-toplevel").strip())
    faults = []
    counts = {}
    for path in git("ls-files", "*.rs").split():
        if path.startswith("native/target") or "/target/" in path:
            continue
        try:
            text = open(path, encoding="utf-8").read()
        except (OSError, UnicodeDecodeError):
            continue
        if "warm" not in text and "Warm" not in text and "RERAIL_WARM" not in text:
            continue
        for n, line in enumerate(text.splitlines(), 1):
            body = NOT_THE_MODE.sub("", code(line))
            for _ in READ.finditer(body):
                counts.setdefault(path, []).append(n)
    for path, lines in sorted(counts.items()):
        if path not in READERS:
            faults.append(f"{path}:{lines[0]}: reads the warm mode ({len(lines)}x); only {', '.join(READERS)} may")
            continue
        allowed, _ = READERS[path]
        if allowed is not None and len(lines) > allowed:
            faults.append(f"{path}: reads the warm mode {len(lines)} times (lines {', '.join(map(str, lines))}); {allowed} approved")

    profile = open(PROFILE, encoding="utf-8").read()
    block = re.search(r"pub const WARM_WRITES_ALLOWED: &\[\(&str, &str\)\] = &\[(.*?)\n\];", profile, re.S)
    if not block:
        faults.append(f"{PROFILE}: no `WARM_WRITES_ALLOWED` list")
    else:
        listed = re.findall(r'^\s*\(\s*"((?:[^"\\]|\\.)*)"\s*,', block.group(1), re.M)
        if listed != ALLOWED:
            faults.append(
                f"{PROFILE}: `WARM_WRITES_ALLOWED` is {listed}, approved is {ALLOWED}: the writes a warm run may make "
                "change only with the user's approval (decision 0197), in both places"
            )

    if faults:
        print("check-warm-mode: the warm check (decision 0197) must not change what a run does:", file=sys.stderr)
        for f in faults:
            print(f"  {f}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
