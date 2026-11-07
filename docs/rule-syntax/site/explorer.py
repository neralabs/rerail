"""Build the explorer beside the syntax page: out/explorer.html and out/explorer/ (the worker, the engine runner,
python.json and rerail-engine.wasm), from explorer/.

    python3 docs/rule-syntax/site/explorer.py

The page runs the engine in the browser (native/wasm/README.md). It needs the Python-only browser build first:

    . native/wasm/env.sh
    cargo build --release -p rerail-engine --bin rerail-engine --target wasm32-wasip1 \\
      --no-default-features --features headless,lang-python

python.json holds every hand-written Python pack rule (its .rr, its part files, its description) and the library
files they include, transitively: the page mounts them where `logic` looks for them (/lib, and beside the rule).
"""

import glob
import json
import os
import re
import shutil
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
RS = os.path.join(ROOT, "docs", "rule-syntax")
LIB = os.path.join(RS, "lib")
WASM = os.path.join(ROOT, "target", "wasm32-wasip1", "release", "rerail-engine.wasm")
INCLUDE = re.compile(r'^\s*#include\s+"([^"]+)"', re.M)


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def library_closure(texts):
    """Library files (paths relative to lib/) the texts include, and what those include, `core.rr` always."""
    found, todo = {}, ["core.rr"]
    for text in texts:
        todo += INCLUDE.findall(text)
    while todo:
        name = todo.pop()
        path = os.path.join(LIB, name)
        if name in found or not os.path.exists(path):
            continue  # a part file beside its rule, not the library's
        found[name] = read(path)
        todo += [n if os.path.exists(os.path.join(LIB, n)) else os.path.join(os.path.dirname(name), n)
                 for n in INCLUDE.findall(found[name])]
    return found


def main():
    out = os.path.join(HERE, "out")
    os.makedirs(os.path.join(out, "explorer"), exist_ok=True)
    described = {}
    for f in glob.glob(os.path.join(ROOT, "packs", "python", "rules", "*.json")):
        doc = json.loads(read(f))
        described[doc.get("id", os.path.basename(f)[:-5])] = doc.get("description", "")
    rules = []
    for f in sorted(glob.glob(os.path.join(RS, "packs", "python", "*.rr"))):
        rid = os.path.basename(f)[:-3]
        if "." in rid:
            continue  # a part file: <id>.<part>.rr
        parts = {os.path.basename(p): read(p) for p in sorted(glob.glob(os.path.join(RS, "packs", "python", rid + ".*.rr")))}
        rules.append({"id": rid, "d": described.get(rid, ""), "rr": read(f), "parts": parts})
    lib = library_closure([r["rr"] for r in rules] + [t for r in rules for t in r["parts"].values()])
    bundle = {"rules": rules, "lib": lib}
    with open(os.path.join(out, "explorer", "python.json"), "w", encoding="utf-8") as f:
        json.dump(bundle, f, ensure_ascii=False, separators=(",", ":"))
    shutil.copyfile(os.path.join(HERE, "explorer", "explorer.html"), os.path.join(out, "explorer.html"))
    for name in ("worker.js", "engine.js"):
        shutil.copyfile(os.path.join(HERE, "explorer", name), os.path.join(out, "explorer", name))
    if not os.path.exists(WASM):
        sys.exit(f"{WASM}: build the browser engine first (see this file's docstring)")
    shutil.copyfile(WASM, os.path.join(out, "explorer", "rerail-engine.wasm"))
    size = os.path.getsize(os.path.join(out, "explorer", "python.json"))
    print(f"{len(rules)} rules, {len(lib)} library files, python.json {size / 1e6:.1f} MB, "
          f"engine {os.path.getsize(WASM) / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
