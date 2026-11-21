"""Measure the cost of inlining nameref on the original SLF001 policy."""

import argparse
import csv
import json
import os
from pathlib import Path
import shutil
import tempfile

import corpus


def compare(engine, sources, out):
    original = corpus.ROOT / "docs/rule-syntax/lib"
    records = {}
    with tempfile.TemporaryDirectory(dir=out) as tmp:
        scratch = Path(tmp)
        library = scratch / "lib"
        # Include resolution requires canonical paths inside the selected library.
        shutil.copytree(original, library)
        names = library / "python/names.rerail"
        source = names.read_text()
        decl = ".decl nameref(n: node, x: symbol)"
        if source.splitlines().count(decl) != 1:
            raise ValueError("the nameref declaration changed")
        names.write_text(source.replace(decl + "\n", decl + " no_inline\n"))
        for name in (
            "profile-stored",
            "default-1",
            "stored-1",
            "stored-2",
            "default-2",
        ):
            stored = "stored" in name
            with tempfile.TemporaryDirectory(dir=scratch) as store:
                result = corpus.native(
                    engine,
                    sources,
                    out / name,
                    Path(store) / "state.sqlite",
                    mode="off",
                    profile=name == "profile-stored",
                    library=library if stored else original,
                )
            if result["sha256"] != corpus.SAMPLES[1664][1]:
                raise ValueError(f"{name} changed SLF001 diagnostics")
            if name == "profile-stored":
                with (out / name / "profile/native.tsv").open() as f:
                    plans = [r["plan"] for r in csv.DictReader(f, delimiter="\t")]
                if not any(p.startswith("seed nameref(") for p in plans) or not any(
                    p.startswith("seed wstart(") and " nameref(" in p for p in plans
                ):
                    raise ValueError(
                        "the profile does not show wstart reading stored nameref"
                    )
            records[name] = result
            (out / "results.json").write_text(json.dumps(records, indent=2) + "\n")
            print(name, result, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--git", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(dir=out) as tmp:
        sources = Path(tmp) / "sources"
        if corpus.prepare(args.git, 1664, sources) != corpus.SAMPLES[1664][0]:
            raise ValueError("pinned source digest differs")
        compare(args.engine.resolve(strict=True), sources, out)


if __name__ == "__main__":
    os.nice(19)
    main()
