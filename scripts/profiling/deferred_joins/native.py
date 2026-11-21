"""Exercise deferred joins through the native CLI. Check the plan and every answer."""

import argparse
import csv
import json
import os
from pathlib import Path
import resource
import subprocess
import tempfile
import time


def example(case):
    # Several witnesses prove (1,1). Some requested endpoints have no shared key.
    # Reader variables deliberately include a possible generated witness name.
    left = {(1, 10), (1, 20), (2, 10), (3, 99)}
    right = {(1, 10), (1, 20), (2, 99), (3, 10), (7, 77)}
    enabled = {1, 2, 7}
    count = {"below": 7, "at": 8, "above": 9}.get(case, 0)
    asked = {(i, 1) for i in range(1, count + 1)}
    if count:
        asked.update({(3, 2), (1, 3), (1, 7)})
    if case in ("fanout", "fanout-mixed"):
        left = {(a, k) for a in range(240) for k in (0, 1 + a % 16)}
        right = {(b, k) for b in range(240) for k in (0, 1 + b % 16)}
        enabled = set(range(239))
        # 57,360 possible answers. Only 480 distinct pairs get requested.
        asked = {(a, b) for a in range(240) for b in (17, 239)}
    wide = {(a, b) for a, k in left for b, j in right if k == j and b in enabled}
    src = [
        ".decl lhs(a: number, k: number)",
        ".decl rhs(b: number, k: number)",
        ".decl enabled(b: number)",
        ".decl request(a: number, b: number)",
        ".decl wide(a: number, b: number)",
        "wide(a,b) :- lhs(a,k), rhs(b,k), enabled(b).",
    ]
    for name, rows in [
        ("lhs", left),
        ("rhs", right),
        ("enabled", {(x,) for x in enabled}),
        ("request", asked),
    ]:
        src.extend(f"{name}({','.join(map(str, row))})." for row in sorted(rows))
    if case == "one-sided":
        src.extend(
            [
                ".decl matches(a: number, b: number)",
                ".output matches",
                "matches(a,b) :- enabled(a), wide(a,b).",
            ]
        )
        expect = {"matches": {(a, b) for a, b in wide if a in enabled}}
    elif case == "fanout-mixed":
        src.extend(
            [
                ".decl chosen(a: number)",
                "chosen(17). chosen(238). chosen(239).",
                ".decl forwards(a: number)",
                ".decl backwards(b: number)",
                ".output forwards, backwards",
                "forwards(a) :- chosen(a), wide(a,_).",
                "backwards(b) :- chosen(b), wide(_,b).",
            ]
        )
        chosen = {17, 238, 239}
        expect = {
            "forwards": {(a,) for a, b in wide if a in chosen},
            "backwards": {(b,) for a, b in wide if b in chosen},
        }
    elif case == "mixed":
        src.extend(
            [
                ".decl forwards(a: number, b: number)",
                ".decl backwards(a: number, b: number)",
                ".output forwards, backwards",
                "forwards(a,b) :- enabled(a), wide(a,b).",
                "backwards(a,b) :- enabled(b), wide(a,b).",
            ]
        )
        expect = {
            "forwards": {(a, b) for a, b in wide if a in enabled},
            "backwards": {(a, b) for a, b in wide if b in enabled},
        }
    elif case == "broad":
        src.extend(
            [
                ".decl pairs(a: number, b: number)",
                ".decl diagonal(a: number)",
                ".output pairs, diagonal",
                "pairs(a,b) :- wide(a,b), enabled(fz1_k).",
                "diagonal(a) :- wide(a,a), wide(a,_).",
            ]
        )
        expect = {"pairs": wide, "diagonal": {(a,) for a, b in wide if a == b}}
    else:
        src.extend(
            [
                ".decl yes(a: number, b: number)",
                ".decl missing(a: number, b: number)",
                ".output yes, missing",
                "yes(a,b) :- request(a,b), wide(a,b).",
                "missing(a,b) :- request(a,b), !wide(a,b).",
            ]
        )
        expect = {"yes": asked & wide, "missing": asked - wide}
    return "\n".join(src), expect


EDIT_PHASES = (
    ("initial", None, None),
    ("promote", "add", "extra.py"),
    ("first-witness", "add", "first.py"),
    ("second-witness", "add", "second.py"),
    ("drop-first", "remove", "first.py"),
    ("drop-last", "remove", "second.py"),
    ("demote", "remove", "extra.py"),
)

def document_lookup(engine, work, mode, cache_rows):
    from corpus import native

    sources = work / "sources"
    sources.mkdir()
    (sources / "probe.py").write_text("def probe(other):\n    return other._join_lab_probe\n")
    out = work / "scan"
    result = native(engine, sources, out, work / "store.sqlite", mode=mode, cache_rows=cache_rows, profile=True)
    findings = json.loads((out / "report.json").read_text())["diagnostics"]
    if len(findings) != 1 or findings[0]["parameters"].get("attr") != "_join_lab_probe":
        raise ValueError(f"document-lookup/{mode}: {findings!r}")
    profile = (out / "profile/profile.txt").read_text()
    return dict(case="document-lookup", mode=mode, phase="cold", rows={"findings": 1},
                placement_retry="deferred join placement:" in profile, **result)


def check(engine, work, case, mode, phase=None, cache_rows=None):
    if case == "document-lookup":
        return document_lookup(engine, work, mode, cache_rows)
    src, expect = example("below" if phase else case)
    rules = work / "join.rerail"
    corpus = work / "corpus"
    corpus.mkdir(exist_ok=True)
    if not (corpus / "input.py").exists():
        (corpus / "input.py").write_text("x = 1\n")
    if case == "cache-edits":
        _, action, name = phase
        if action == "add":
            (corpus / name).write_text("extra = 1\n")
        elif action == "remove":
            (corpus / name).unlink()
        src = """
.decl doc(d: number, path: symbol, module: symbol)
.input doc
.decl lhs(a: number, k: number)
.decl rhs(b: number, k: number)
.decl wide(a: number, b: number)
.decl lefts(a: number)
.decl rights(b: number)
.output lefts, rights
wide(a,b) :- lhs(a,k), rhs(b,k).
lefts(a) :- wide(a,_).
rights(b) :- wide(_,b).
lhs(64,0) :- doc(_, "extra.py", _).
"""
        src += "\n".join(f"lhs({i},0). rhs({i},0)." for i in range(64))
        count = 64 + int((corpus / "extra.py").exists())
        expect = {"lefts": {(i,) for i in range(count)}, "rights": {(i,) for i in range(64)}}
    elif phase:
        _, action, name = phase
        if action == "add":
            (corpus / name).write_text(f"{Path(name).stem} = 1\n")
        elif action == "remove":
            (corpus / name).unlink()
        src += """
.decl doc(d: number, path: symbol, module: symbol)
.input doc
request(8,1) :- doc(_, "extra.py", _).
rhs(7,10) :- doc(_, "first.py", _).
rhs(7,20) :- doc(_, "second.py", _).
"""
        if (corpus / "extra.py").exists():
            expect["missing"].add((8, 1))
        if any((corpus / name).exists() for name in ("first.py", "second.py")):
            expect["missing"].remove((1, 7))
            expect["yes"].add((1, 7))
    if not rules.exists():
        rules.write_text(src + "\n")
    out = work / "out"
    profile = work / ("profile-" + phase[0] if phase else "profile")
    env = {k: v for k, v in os.environ.items() if not k.startswith("RERAIL_")}
    env.update(
        RERAIL_DEFERRED_JOINS=mode,
        RERAIL_THREADS="1",
        RERAIL_PROFILE=str(profile),
        RERAIL_TAPE_CACHE=str(work / "store.sqlite"),
    )
    if cache_rows is not None:
        env["RERAIL_JOIN_CACHE_ROWS"] = str(cache_rows)
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.monotonic()
    with (work / "run.log").open("w") as log:
        process = subprocess.run(
            [
                "nice",
                "-n",
                "19",
                engine,
                "logic",
                rules,
                corpus,
                out,
                "--modes",
                "program",
            ],
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            timeout=45,
        )
    if process.returncode:
        raise RuntimeError(f"{case}/{mode} exited {process.returncode}: " + (work / "run.log").read_text()[-2000:])
    elapsed = time.monotonic() - start
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    cpu = after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime
    exported = work / ("csv-" + phase[0] if phase else "csv")
    subprocess.run(
        ["nice", "-n", "19", engine, "csv", out, exported],
        capture_output=True,
        timeout=15,
        check=True,
    )
    counts = {}
    for name, expected in expect.items():
        path = exported / f"{name}.csv"
        lines = path.read_text().splitlines() if path.exists() else []
        rows = [tuple(map(int, row.split("\t"))) for row in lines]
        if sorted(rows) != sorted(expected):
            raise ValueError(
                f"{case}/{mode}/{name}: got {rows}, expected {sorted(expected)}"
            )
        counts[name] = len(rows)
    lines = (profile / "profile.txt").read_text().splitlines()
    decisions = []
    for i, line in enumerate(lines):
        if line == "deferred joins:":
            for text in lines[i + 1 :]:
                if not text.startswith("  - "):
                    break
                if "wide" in text:
                    decisions.append(text.strip())
    if mode != "off" and case != "empty" and (not phase or phase[0] == "initial"):
        if case in ("broad", "mixed", "fanout-mixed", "cache-edits"):
            wanted = "wide: handed on as parts"
        elif case == "one-sided":
            wanted = 'endpoint demand: ["wide column 0"]'
        elif mode in ("adaptive", "auto"):
            wanted = "wide column"
        else:
            wanted = 'pair demand: ["wide"]'
        if not any(wanted in d for d in decisions):
            raise ValueError(f"{case}/{mode}: missing plan {wanted!r}: {decisions!r}")
    with (profile / "heads.tsv").open() as heads:
        measured = [
            dict(head=r["head"], cpu_s=float(r["cpu_s"]), work=int(r["work"]))
            for r in csv.DictReader(heads, delimiter="\t")
            if any(w in r["head"] for w in ("wide", *expect))
        ]
    owner_fallback = any(
        "projected input omits its owner column" in line for line in lines
    )
    return dict(
        case=case,
        owner_fallback=owner_fallback,
        phase=phase[0] if phase else "cold",
        mode=mode,
        cache_rows=cache_rows,
        rows=counts,
        decisions=decisions,
        heads=measured,
        profiled_cpu_s=round(cpu, 3),
        elapsed_s=round(elapsed, 3),
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--cache-rows", type=int, help="allow this many witnesses across optional pair caches")
    cases = [
        "empty",
        "below",
        "at",
        "above",
        "one-sided",
        "mixed",
        "broad",
        "fanout",
        "fanout-mixed",
        "edits",
        "cache-edits",
        "document-lookup",
    ]
    parser.add_argument("--cases", nargs="+", choices=cases, default=cases)
    parser.add_argument(
        "--modes",
        nargs="+",
        choices=["off", "on", "adaptive", "auto"],
        default=["off", "on", "adaptive"],
    )
    args = parser.parse_args()
    if args.cache_rows is not None and args.cache_rows < 0:
        parser.error("--cache-rows must be nonnegative")
    engine = args.engine.resolve(strict=True)
    with args.out.open("x") as report:
        for case in args.cases:
            for mode in args.modes:
                with tempfile.TemporaryDirectory(prefix="deferred-native-") as tmp:
                    phases = EDIT_PHASES if case == "edits" else [None]
                    if case == "cache-edits":
                        phases = (("initial", None, None), ("exceed", "add", "extra.py"), ("fit", "remove", "extra.py"))
                    for phase in phases:
                        record = check(engine, Path(tmp), case, mode, phase, args.cache_rows)
                        report.write(json.dumps(record) + "\n")
                        report.flush()
                        print(case, record["phase"], mode, record["rows"], flush=True)


if __name__ == "__main__":
    os.nice(19)
    main()
