#!/usr/bin/env python3
"""Checks that the engine's `test-hooks` feature turns on everything its dev-dependencies add to its
dependencies, so that a development build of the CLI (`cargo build --bin rerail-engine --features
test-hooks`) and the tests build every dependency crate alike: one build per edit, not two
(docs/final-shape/notes/t37-edit-loop-time.md, "One feature set").

    scripts/check/check_dev_features.py [native/Cargo.toml]

For each dev-dependency that is also a dependency, each feature it names and the dependency does not
must be in `test-hooks` as `dep/feature`, and default features the dependency turns off must stay
off. A dev-dependency from the registry that is not a dependency must be an optional dependency
that `test-hooks` turns on (`dep:name`). Path crates only the tests use (the lean parsers) are
leaves of their own, and are not checked. And each workspace crate's `default` turns on the `default`
of each workspace crate it takes without default features (`defaults_carried`), and each crate's
`test-hooks` turns on those of the crates below it (`hooks_carried`). Exits 1 with what is missing.
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
    missing += defaults_carried(os.path.dirname(path))
    missing += hooks_carried(os.path.dirname(path))
    if missing:
        print(f"{path}: these features would make builds compile workspace crates in more than one variant "
              "(a second compile of each per edit, and its own incremental state):", file=sys.stderr)
        for m in missing:
            print(f"  {m}", file=sys.stderr)
        return 1
    return 0


def workspace(native):
    import glob
    mans = {}
    for f in [os.path.join(native, "Cargo.toml")] + glob.glob(os.path.join(native, "crates", "*", "Cargo.toml")):
        with open(f, "rb") as h:
            m = tomllib.load(h)
        mans[m["package"]["name"]] = (f, m)
    return mans


def tables(m, kind):
    """`kind` ("dependencies", "dev-dependencies") of manifest `m`, its target-specific tables included."""
    out = {}
    for t in [m] + list(m.get("target", {}).values()):
        out.update(t.get(kind, {}))
    return out


def hooks_carried(native):
    """A workspace crate's `test-hooks` turns on the `test-hooks` of each workspace crate it depends on
    that has one, and the crate's own test build turns on its own (a dev-dependency on itself, as
    `crates/lang` has): then every test build, of the engine or of one crate, and the CLI with
    `--features test-hooks`, give each crate below the same features. A crate that depends on hooked
    crates has a `test-hooks` of its own."""
    mans = workspace(native)
    hooked = {n for n, (_, m) in mans.items() if "test-hooks" in m.get("features", {})}
    out = []
    for name, (f, m) in sorted(mans.items()):
        rel = os.path.relpath(f, native)
        have = m.get("features", {}).get("test-hooks")
        below = {k: v for k, v in tables(m, "dependencies").items()
                 if isinstance(v, dict) and "path" in v and k in hooked and k != name}
        if have is None:
            if below:
                out.append(f"{rel}: no `test-hooks` feature, though it depends on hooked crates ({', '.join(sorted(below))})")
            continue
        for k, v in sorted(below.items()):
            if f"{k}/test-hooks" not in have and f"{k}?/test-hooks" not in have:
                out.append(f"{rel}: `test-hooks` lacks \"{k}/test-hooks\"")
        own = tables(m, "dev-dependencies").get(name)
        if f != os.path.join(native, "Cargo.toml") and not (isinstance(own, dict) and "test-hooks" in own.get("features", [])):
            out.append(f"{rel}: its own test build does not turn on its test hooks "
                       f"(add `{name} = {{ path = \".\", features = [\"test-hooks\"] }}` to its dev-dependencies)")
    return out


def defaults_carried(native):
    """A workspace crate's `default` turns on the `default` of each workspace crate it depends on
    without default features: else `cargo test -p <crate>` builds those below it with features the
    engine's builds do not give them, a variant of each."""
    mans = workspace(native)
    with_default = {n for n, (_, m) in mans.items() if "default" in m.get("features", {})}
    out = []
    for name, (f, m) in sorted(mans.items()):
        default = m.get("features", {}).get("default")
        if default is None:
            continue
        for t in [m] + list(m.get("target", {}).values()):
            for k, v in t.get("dependencies", {}).items():
                if (isinstance(v, dict) and "path" in v and k in with_default
                        and not v.get("default-features", True) and f"{k}/default" not in default):
                    out.append(f"{os.path.relpath(f, native)}: `default` lacks \"{k}/default\"")
    return out


if __name__ == "__main__":
    sys.exit(main(sys.argv))
