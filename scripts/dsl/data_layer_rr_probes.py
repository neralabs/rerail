#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run a ported data-layer program over each recorded probe of a check; compare (path, line, code) keys.

usage: data_layer_rr_probes.py <engine> <ported dir> <main stem> <probes.json> <check> <work dir>
"""
import json, os, re, subprocess, sys, pathlib, time, resource
engine, ported, stem, probes, check, work = sys.argv[1:7]
ported, work = pathlib.Path(ported), pathlib.Path(work)
main = (ported / f"{stem}.rr").read_text()
pol = json.loads(next(l for l in main.splitlines() if l.startswith(".policy"))[len(".policy "):])
code = {c["id"]: c.get("code") or c.get("id") for c in pol["clauses"]}
for c in pol["clauses"]:
    for k in ("forbid", "relational", "require"):
        if isinstance(c.get(k), dict) and c[k].get("code"): code[c["id"]] = c[k]["code"]
prog = work / "prog"
prog.mkdir(parents=True, exist_ok=True)
for f in ported.glob("*.rr"):
    t = f.read_text()
    if f.name == f"{stem}.rr" and "vline(" not in t:
        t += "\n.decl vline(clause: symbol, path: symbol, l: number)\n.output vline\nvline(c, p, l) :- violation(_, c, d, n), doc(d, p, _), line(n, l).\n"
        if (m := re.search(r"^\.decl finding\((\w+): symbol, \w+: symbol, \w+: node\)", t, re.M)):
            # The code is the `key` column (`finding(clause, key, n)`), or the first where it is the `issue`.
            k = "finding(k, _, n)" if m.group(1) == "issue" else "finding(_, k, n)"
            t += f"vline(k, p, l) :- {k}, node(d, n, _, _, _), doc(d, p, _), line(n, l).\n"
    (prog / f.name).write_text(t)
events = [e for e in json.load(open(probes))["events"] if e["check"] == check]
ok = bad = 0
t0 = resource.getrusage(resource.RUSAGE_CHILDREN); w0 = time.time()
for i, e in enumerate(events):
    repo = work / f"repo{i}"
    for f in e["files"]:
        p = repo / f["path"]; p.parent.mkdir(parents=True, exist_ok=True); p.write_text(f["source"])
    out = work / f"out{i}"
    env = dict(os.environ, RERAIL_COSTS="off", RERAIL_TAPE_CACHE=str(work / "cache.sqlite"))
    r = subprocess.run(["nice", "-n", "19", engine, "logic", str(prog), str(repo), str(out)], env=env, capture_output=True, text=True, timeout=115)
    err = [l for l in r.stderr.splitlines() if l.startswith("rerail-engine:")]
    got = set()
    vl = out / "p0_vline.csv"
    if not vl.exists(): vl = out / "vline.csv"
    if vl.exists():
        for line in vl.read_text().splitlines():
            c, p, l = line.split("\t")
            got.add((p, int(l), code.get(c, c)))
    want = {(f["path"], f["line"], f["code"]) for f in e["findings"]}
    same = got == want and not err
    ok += same; bad += not same
    if not same:
        print(f"DIFF {e['test']}: want {sorted(want)} got {sorted(got)} {err[:1]}")
t1 = resource.getrusage(resource.RUSAGE_CHILDREN)
print(f"{check}: {ok}/{len(events)} probes equal; cpu {t1.ru_utime - t0.ru_utime + t1.ru_stime - t0.ru_stime:.1f}s wall {time.time() - w0:.1f}s")
