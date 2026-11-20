#!/usr/bin/env python3
"""Refuses a commit that adds process-global state to the engine (decision 0442).

The engine is re-entrant and holds no global state: two runs in one process, on any threads, and two callers with
different trust (a security boundary), never reach each other's state. What a static may hold:

- a constant (a table, a slice, a string): not listed here;
- `stays`: what cannot differ between runs or callers: code a crate above registers at load (a function pointer, a
  device), a value computed once from the engine's own constants (a regex of a literal, core.rerail's declarations);
- everything else is listed by where it moves, and the list only shrinks:
  - `run`: a run's own data, settings or measurement (`logic::run::Run`);
  - `connection`: a connection's own state (on the connection's object);
  - `shared`: shared by design (one-per-store resources, interners, caches): from the explicit shared object a
    caller makes and passes in (`profile::Shared`);
  - `machine`: the machine's resources (the job pool and its queues, the device, the learned cost and price model,
    the memory budget): from the machine object every shared object is made on (`Machine`);
  - `review`: not yet classified by hand.

Every non-constant `static` in the engine's source (native/src, native/crates/*/src, accel/src; test files and
examples left out) must be in `global-state.txt` beside this script, as `path::NAME<TAB>class<TAB>why`. A static not listed is
refused; a listed one that is gone must be cut from the list (it is printed). Thread-locals have their own check
(`check_thread_state.py`, `run_state::STATE`).

Usage: check_no_global_state.py [--write]   (--write rewrites the list's entries in place, keeping their classes,
adding new ones as `review` and dropping gone ones: for the commit that moves state, never to admit new state)
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LIST = os.path.join(os.path.dirname(os.path.abspath(__file__)), "global-state.txt")
BASES = ["native/src", "native/crates", "accel/src"]
STATIC = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?static\s+(?:mut\s+)?([A-Z_][A-Z_0-9]*)\s*:\s*([^=;]+)", re.M)


def is_test(path):
    name = os.path.basename(path)
    return (
        "/tests/" in path
        or "/examples/" in path
        or "/benches/" in path
        or name in ("tests.rs", "test.rs")
        or name.endswith("_tests.rs")
        or name.startswith("test_")
    )


def constant(ty):
    ty = ty.strip()
    if any(w in ty for w in ("Atomic", "Mutex", "RwLock", "Cell", "OnceLock", "LazyLock", "Lazy<", "Once")):
        return False
    return ty.startswith("&") or ty.startswith("[") or re.fullmatch(r"(u8|u16|u32|u64|usize|i32|i64|f64|bool|char)", ty) is not None


def found():
    out = {}
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
                for m in STATIC.finditer(text):
                    # A thread-local's statics are `check_thread_state.py`'s.
                    before = text[: m.start()]
                    if before.rfind("thread_local!") > before.rfind("}\n"):
                        continue
                    if constant(m.group(2)):
                        continue
                    out[f"{rel}::{m.group(1)}"] = m.group(2).strip()
    return out


def listed():
    out = {}
    if not os.path.exists(LIST):
        return out
    for line in open(LIST):
        if not line.strip() or line.startswith("#"):
            continue
        key, cls, why = (line.rstrip("\n").split("\t") + ["", ""])[:3]
        out[key] = (cls, why)
    return out


CLASSES = ("stays", "run", "connection", "shared", "machine", "review")


def main():
    have, pinned = found(), listed()
    if "--write" in sys.argv:
        header = [l for l in open(LIST)] if os.path.exists(LIST) else []
        header = [l for l in header if l.startswith("#")]
        rows = [f"{k}\t{pinned.get(k, ('review', ''))[0]}\t{pinned.get(k, ('review', ''))[1]}" for k in sorted(have)]
        open(LIST, "w").write("".join(header) + "\n".join(rows) + "\n")
        print(f"global-state.txt: {len(rows)} entries ({len(set(have) - set(pinned))} new as review, {len(set(pinned) - set(have))} gone)")
        return 0
    new = sorted(set(have) - set(pinned))
    gone = sorted(set(pinned) - set(have))
    bad = sorted(k for k, (c, _) in pinned.items() if c not in CLASSES)
    ok = True
    if new:
        ok = False
        print("check_no_global_state: new process-global state (decision 0442): a run's data goes in the run, a")
        print("connection's on it, and what is shared by design comes from the shared object the caller passes:")
        for k in new:
            print(f"  {k}: {have[k][:100]}")
    if gone:
        ok = False
        print("check_no_global_state: listed statics no longer there (cut them from global-state.txt):")
        for k in gone:
            print(f"  {k}")
    if bad:
        ok = False
        print(f"check_no_global_state: entries with a class not in {CLASSES}:", *bad, sep="\n  ")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
