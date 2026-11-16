#!/usr/bin/env python3
"""Checks that the engine's `test-hooks` feature turns on everything its dev-dependencies add to its
dependencies, so that a development build of the CLI (`cargo build --bin rerail-engine --features
test-hooks`) and the tests build every dependency crate alike: one build per edit, not two
(docs/final-shape/notes/t37-edit-loop-time.md, "One feature set").

    scripts/check/check-dev-features.py [native/Cargo.toml]

For each dev-dependency that is also a dependency, each feature it names and the dependency does not
must be in `test-hooks` as `dep/feature`, and default features the dependency turns off must stay
off. A dev-dependency from the registry that is not a dependency must be an optional dependency
that `test-hooks` turns on (`dep:name`). Path crates only the tests use (the lean parsers) are
leaves of their own, and are not checked. Exits 1 with what is missing.
"""
import os
import sys
import tomllib


def spec(v):
    return v if isinstance(v, dict) else {"version": v}


def main(argv):
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    path = argv[1] if len(argv) > 1 else os.path.join(root, "native", "Cargo.toml")
    with open(path, "rb") as f:
        manifest = tomllib.load(f)
    # Target-specific tables count too (`[target.'cfg(target_os = "macos")'.dev-dependencies]`): on the
    # machine that builds, they are the same as the plain ones.
    tables = [manifest] + list(manifest.get("target", {}).values())
    deps, dev = {}, {}
    for t in tables:
        for k, v in t.get("dependencies", {}).items():
            d = deps.setdefault(k, {"features": []})
            d.update({x: y for x, y in spec(v).items() if x != "features"})
            d["features"] = d["features"] + spec(v).get("features", [])
        for k, v in t.get("dev-dependencies", {}).items():
            d = dev.setdefault(k, {"features": []})
            d.update({x: y for x, y in spec(v).items() if x != "features"})
            d["features"] = d["features"] + spec(v).get("features", [])
    hooks = set(manifest.get("features", {}).get("test-hooks", []))
    missing = []
    for name, d in dev.items():
        n = deps.get(name)
        if n is None:
            if "path" in d:
                continue
            if f"dep:{name}" not in hooks:
                missing.append(f"{name}: a dev-dependency only; make it an optional dependency and add \"dep:{name}\"")
            continue
        have = set(n.get("features", []))
        for feature in d.get("features", []):
            if feature not in have and f"{name}/{feature}" not in hooks:
                missing.append(f"\"{name}/{feature}\"")
        if d.get("default-features", True) and not n.get("default-features", True):
            missing.append(f"{name}: the dev-dependency turns its default features on (add default-features = false)")
    if missing:
        print(f"{path}: the engine's `test-hooks` feature lacks what its dev-dependencies turn on, so a CLI build "
              "with it and the tests would build its dependencies apart:", file=sys.stderr)
        for m in missing:
            print(f"  {m}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
