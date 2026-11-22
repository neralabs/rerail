#!/usr/bin/env python3
"""Keeps the crate split's graph as it was cut (docs/final-shape/proposals/crate-split/incremental-plan.md, §5).

    scripts/check/check_crate_dag.py [--write-hooks]

1. **No reference up the graph.** `tools/cratify/crate-order.tsv` lists every workspace crate, bottom first, with
   its class. A crate's dependencies (normal and build, any target) are workspace crates on earlier lines. A new
   crate gets its line after the crates it depends on; a dependency that needs a crate moved is a change to that file, seen in
   review, not a line in a manifest.
2. **No core crate names a provider.** A `core` crate depends on no `provider` (a language pack, the Metal
   device), a library on no `test` crate, and nothing but a `test` crate on a `root` (boot, the engine) — but the
   engine on boot. The providers register through boot (`rerail-registry`).
3. **No new install hook outside the registry.** A function named `install` or `install_*` that stores what it is
   handed in a `OnceLock` (`.set(`, `.get_or_init(`) and not in a `rerail_registry` `Slot` or `Registry`
   (`.register(`) is refused, but for the ones `install-hooks.txt` lists, which were there when the check came.
   A file with no `OnceLock` holds no such hook (its `.set(` is another type's). The list only shrinks: a listed
   hook that is gone, or now registers, must be cut from it (`--write-hooks` rewrites it, and refuses to add one).

Exits 1 with what is wrong.
"""
import graphlib
import os
import re
import sys
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ORDER = os.path.join(ROOT, "tools", "cratify", "crate-order.tsv")
HOOKS = os.path.join(ROOT, "scripts", "check", "install-hooks.txt")
CLASSES = {"core", "provider", "root", "test"}


def manifests():
    members = tomllib.load(open(os.path.join(ROOT, "Cargo.toml"), "rb"))["workspace"]["members"]
    out = {}
    for member in members:
        path = os.path.normpath(os.path.join(ROOT, member))
        out[path] = tomllib.load(open(os.path.join(path, "Cargo.toml"), "rb"))
    return out


def graph():
    by_path = manifests()
    name = {path: m["package"]["name"] for path, m in by_path.items()}
    deps = {}
    for path, m in by_path.items():
        found = set()
        for table in [m] + list(m.get("target", {}).values()):
            for section in ("dependencies", "build-dependencies"):
                for dep in table.get(section, {}).values():
                    if isinstance(dep, dict) and "path" in dep:
                        target = os.path.normpath(os.path.join(path, dep["path"]))
                        if target in name and name[target] != name[path]:
                            found.add(name[target])
        deps[name[path]] = found
    return deps


def read_order():
    rows = []
    for n, line in enumerate(open(ORDER), 1):
        line = line.rstrip("\n")
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 2 or parts[1] not in CLASSES:
            sys.exit(f"{ORDER}:{n}: want `crate<TAB>class`, the class one of {sorted(CLASSES)}: {line!r}")
        rows.append((parts[0], parts[1]))
    return rows


def check_graph(errors):
    deps = graph()
    rows = read_order()
    rank = {}
    cls = {}
    for i, (crate, c) in enumerate(rows):
        if crate in rank:
            errors.append(f"crate-order.tsv lists {crate} twice")
        rank[crate] = i
        cls[crate] = c
    for crate in sorted(set(deps) - set(rank)):
        errors.append(f"{crate} is a workspace crate crate-order.tsv does not list: give it its line, before the "
                      f"crates that depend on it")
    for crate in sorted(set(rank) - set(deps)):
        errors.append(f"crate-order.tsv lists {crate}, which is no workspace crate: cut its line")
    for crate, below in sorted(deps.items()):
        if crate not in rank:
            continue
        for dep in sorted(below):
            if dep not in rank:
                continue
            if rank[dep] > rank[crate]:
                errors.append(f"{crate} depends on {dep}, which crate-order.tsv puts after it (higher): a "
                              f"reference up the graph (cut it: move what it reads down, or a trait the upper crate registers)")
            a, b = cls[crate], cls[dep]
            if a == "core" and b == "provider":
                errors.append(f"{crate} (core) depends on {dep}, a provider: core names no provider (a trait it "
                              f"declares, which the provider registers through boot)")
            if a != "test" and b == "test":
                errors.append(f"{crate} depends on {dep}, a test crate")
            if b == "root" and a != "test" and not (crate == "rerail-engine" and dep == "rerail-boot"):
                errors.append(f"{crate} depends on {dep}, the composition root: only the engine and the test "
                              f"crates do")
    try:
        graphlib.TopologicalSorter(deps).prepare()
    except graphlib.CycleError as cycle:
        errors.append(f"the crates depend on each other in a cycle: {cycle.args[1]}")


SOURCES = [os.path.join(ROOT, "native", "src"), os.path.join(ROOT, "native", "crates")]
HOOK_FN = re.compile(r"^(\s*)(?:pub(?:\([^)]*\))?\s+)?fn (install(?:_[a-z0-9_]+)?)\b")
STORES = re.compile(r"\.set\(|\.get_or_init\(")


def is_test(path):
    base = os.path.basename(path)
    return base == "tests.rs" or base.endswith("_tests.rs") or "/tests/" in path or "/tests_" in path


def hooks():
    """(file, function) of every install hook that stores in a `OnceLock` and does not register."""
    found = set()
    for top in SOURCES:
        for dirpath, dirnames, files in os.walk(top):
            dirnames[:] = [d for d in dirnames if d not in ("target", "vendor")]
            for f in files:
                if not f.endswith(".rs"):
                    continue
                path = os.path.join(dirpath, f)
                if is_test(path):
                    continue
                source = open(path, encoding="utf-8", errors="replace").read()
                # A hook stores in a OnceLock: a file with none has no hook (its `.set(` is another type's).
                if "OnceLock" not in source:
                    continue
                lines = source.split("\n")
                for i, line in enumerate(lines):
                    m = HOOK_FN.match(line)
                    if not m:
                        continue
                    indent = m.group(1)
                    body = []
                    for later in lines[i + 1:]:
                        if later.startswith(indent + "}") and later.strip() == "}":
                            break
                        body.append(later)
                    text = "\n".join(body)
                    if STORES.search(text) and ".register(" not in text:
                        found.add((os.path.relpath(path, ROOT), m.group(2)))
    return found


def read_hooks():
    if not os.path.exists(HOOKS):
        return set()
    out = set()
    for line in open(HOOKS):
        line = line.strip()
        if line and not line.startswith("#"):
            path, _, function = line.partition(" ")
            out.add((path, function))
    return out


HEADER = """# Install hooks that store in a OnceLock, not a rerail_registry Slot or Registry: the ones there were when
# scripts/check/check_crate_dag.py came. The list only shrinks: a hook moved onto the registry leaves it.
# file function
"""


def check_hooks(errors, write):
    now = hooks()
    listed = read_hooks()
    for path, function in sorted(now - listed):
        errors.append(f"{path}: `{function}` keeps what it is handed in a OnceLock: register it in a "
                      f"rerail_registry Slot or Registry instead (a provider names itself; a second registration "
                      f"is an error, not ignored)")
    gone = listed - now
    if write:
        if not (now - listed):
            with open(HOOKS, "w") as f:
                f.write(HEADER)
                for path, function in sorted(listed & now):
                    f.write(f"{path} {function}\n")
            return
    for path, function in sorted(gone):
        errors.append(f"install-hooks.txt lists {path} `{function}`, which no longer keeps a OnceLock: cut it "
                      f"(--write-hooks)")


def main(argv):
    errors = []
    check_graph(errors)
    check_hooks(errors, "--write-hooks" in argv)
    if errors:
        print("check_crate_dag:", file=sys.stderr)
        for e in errors:
            print("  " + e, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
