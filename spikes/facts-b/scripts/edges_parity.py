#!/usr/bin/env python3
"""Linker parity: a greplay parity run's `edges` rows (linker probe rule: path, start_byte,
end_byte, target_path#target_start) against native's reference_edges dump (path, start,
target_path, target_start, target_end, class), compared on (path, start, target_path,
target_start) as multisets. usage: edges_parity.py RUN_DIR [NATIVE_TSV] [--clause NAME]"""
import collections
import json
import re
import sys
from pathlib import Path

NATIVE = Path.home() / "rerail-bench/facts/phase-2/s3-crossfile/linker/native-ground-truth/reference_edges.tsv"
VAL = re.compile(r'Text\("((?:[^"\\]|\\.)*)"\)|Int\((-?\d+)\)|Null|Real\(([^)]*)\)')


def parse(row):
    out = []
    for m in VAL.finditer(row):
        if m.group(1) is not None:
            s = re.sub(r"\\u\{([0-9a-fA-F]+)\}", lambda u: "\\u%04x" % int(u.group(1), 16), m.group(1))
            out.append(json.loads('"' + s + '"'))
        elif m.group(2) is not None:
            out.append(int(m.group(2)))
        elif m.group(3) is not None:
            out.append(float(m.group(3)))
        else:
            out.append(None)
    return out


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    clause = "edges:edges"
    if "--clause" in sys.argv:
        clause = sys.argv[sys.argv.index("--clause") + 1]
        args = [a for a in args if a != clause]
    run = Path(args[0])
    native = Path(args[1]) if len(args) > 1 else NATIVE
    res = json.loads((run / "results.json").read_text())
    rows = res["parity_rows"][clause]
    got = collections.Counter()
    for r in rows:
        path, start, _end, code = parse(r)
        tp, ts = code.rsplit("#", 1)
        got[(path, int(start), tp, int(ts))] += 1
    want = collections.Counter()
    for line in native.read_text().splitlines():
        f = line.split("\t")
        want[(f[0], int(f[1]), f[2], int(f[3]))] += 1
    missing = want - got
    extra = got - want
    out = {"run": str(run), "native": str(native), "generator_rows": sum(got.values()), "native_rows": sum(want.values()),
           "missing": sum(missing.values()), "extra": sum(extra.values()),
           "first_missing": [list(k) for k in list(missing)[:10]], "first_extra": [list(k) for k in list(extra)[:10]],
           "runtime_refusals_live": res.get("runtime_refusals_live")}
    (run / "edges-parity.json").write_text(json.dumps(out, indent=1))
    print(json.dumps(out, indent=1))
    sys.exit(0 if not missing and not extra else 1)


if __name__ == "__main__":
    main()
