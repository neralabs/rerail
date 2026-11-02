#!/usr/bin/env python3
"""The pack's policies cut to the clauses the slice lowers, for native like for like (notes/t18-slice-at-scale.md).

    benchmark/final-shape/slice-at-scale/lowered-rules.py <slice.json> <out dir> <rules dir> [<rules dir> …]

Each policy with a lowered clause (slice.json's `lowered`) is written to <out dir> with its `forbid` clauses cut to the
lowered ones; its other clauses (`define`s, which the forbids may use) stay. Policies with no lowered clause are left
out, so native evaluates the slice's clauses and nothing else.
"""
import json
import sys
from pathlib import Path


def main(slice_json: Path, out: Path, directories: list[Path]) -> None:
    lowered: dict[str, set[str]] = {}
    for clause in json.loads(slice_json.read_text())["lowered"]:
        lowered.setdefault(clause["rule"], set()).add(clause["clause"])
    out.mkdir(parents=True, exist_ok=True)
    written = clauses = 0
    for directory in directories:
        for path in sorted(directory.rglob("*.json")):
            try:
                document = json.loads(path.read_text())
            except json.JSONDecodeError:
                continue
            keep = lowered.get(document.get("id")) if isinstance(document, dict) else None
            if not keep:
                continue
            document["clauses"] = [c for c in document["clauses"] if "forbid" not in c or c["id"] in keep]
            (out / path.name).write_text(json.dumps(document, indent=2) + "\n")
            written += 1
            clauses += sum(1 for c in document["clauses"] if "forbid" in c)
    print(f"{written} policies, {clauses} forbid clauses -> {out}")


if __name__ == "__main__":
    main(Path(sys.argv[1]), Path(sys.argv[2]), [Path(p) for p in sys.argv[3:]])
