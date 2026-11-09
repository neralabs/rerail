#!/usr/bin/env python3
"""Run SQL examples and retain requests, profiles, answers and timings outside the repository."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
CLOSURE = """.decl edge(a: number, b: number)
.input edge
.decl reach(a: number, b: number)
reach(a, b) :- edge(a, b).
reach(a, c) :- reach(a, b), edge(b, c).
.output reach
"""
SHORTEST = """.decl edge(a:number,b:number,w:number)
.decl root(n:number)
.input edge,root
.decl distance(n:number,d:min<>) widen 3
distance(n,0) :- root(n).
distance(b,d+w) :- distance(a,d), edge(a,b,w).
.output distance
"""


def relation(name, columns, rows):
    return {"name": name, "columns": columns, "rows": rows}


def recursive_lattice(groups):
    if groups < 1:
        raise ValueError("regions must be positive")
    edges = [[g*10+a, g*10+b, w] for g in range(groups)
             for a,b,w in [(0,1,5),(0,2,1),(2,1,1),(1,3,2),(3,2,1)]]
    request = {
        "relations": [relation("edge", ["a","b","w"], edges),
                      relation("root", ["n"], [[g*10] for g in range(groups)])],
        "program": SHORTEST,
        "after_run_sql": [
            "CREATE TEMP TRIGGER recursive_delete_guard BEFORE DELETE ON distance WHEN OLD.c0<170 OR OLD.c0>179 BEGIN SELECT RAISE(ABORT,'unrelated recursive delete'); END",
            "CREATE TEMP TRIGGER recursive_insert_guard BEFORE INSERT ON distance WHEN NEW.c0<170 OR NEW.c0>179 BEGIN SELECT RAISE(ABORT,'unrelated recursive insert'); END"],
        "changes": [[{"name":"edge","delete":[[172,171,1]]}],
                    [{"name":"root","delete":[[170]]}],
                    [{"name":"root","insert":[[170]]},
                     {"name":"edge","insert":[[172,171,1]]}]],
        "queries": ["SELECT * FROM distance WHERE c0>=170 AND c0<180 ORDER BY c0",
                    "SELECT count(*) FROM distance"],
    }
    request["changes"] *= 2
    return request


def session_request(groups):
    request = recursive_lattice(groups)
    changes = request["changes"]
    steps = [
        {"changes": changes[0], "limits": {"max_steps": 1}, "expected_error": "execution-limit"},
        {"changes": changes[0], "max_waves": 1, "expected_error": "wave-limit"},
    ] + [{"changes": change} for change in changes]
    # Complete-state observations replace the sampled queries and publication guards.
    return {"program": request["program"], "relations": request["relations"], "steps": steps,
            "queries": ["SELECT count(*) FROM distance"]}


def cases(corpus):
    result = [(f"update-recursive-lattice-{groups}", recursive_lattice(groups)) for groups in [1000, 10000]]
    for n in [64, 128, 256, 512, 1024]:
        result.append((f"closure-{n}", {
            "relations": [relation("edge", ["src", "dst"], [[i, i+1] for i in range(n-1)])],
            "program": CLOSURE, "max_waves": 4096,
            "queries": ["SELECT count(*), min(c0), max(c1), sum(c0), sum(c1) FROM reach",
                        "WITH RECURSIVE r(a,b) AS (SELECT src,dst FROM edge UNION SELECT r.a,e.dst FROM r JOIN edge e ON e.src=r.b) SELECT count(*),min(a),max(b),sum(a),sum(b) FROM r",
                        f"SELECT count(*) FROM reach WHERE c0<0 OR c1>={n} OR c0>=c1"],
        }))
    for n in [512, 1024]:
        result.append((f"update-closure-{n}", {
            "relations": [relation("edge", ["src", "dst"], [[i,i+1] for i in range(n-1)])],
            "program": CLOSURE, "max_waves": 4096,
            "changes": [
                [{"name":"edge","insert":[[10000,10001]]}],
                [{"name":"edge","insert":[[20000,20001]]}],
                [{"name":"edge","delete":[[10000,10001]]}],
                [{"name":"edge","insert":[[n-1,n]]}],
                [{"name":"edge","delete":[[n-1,n]]}],
            ],
            "queries": ["SELECT count(*),sum(c0),sum(c1) FROM reach"],
        }))
    for n in [100, 1000, 10000]:
        files = [{"path": f"m{i}.py", "language": "python", "source": f"def f{i}():\n    x{i} = 1\n    return x{i}\n"} for i in range(n)]
        initial = {"files": files}
        edited = {"files": [{**files[0], "source": files[0]["source"] + "extra = 2\n"}, *files[1:]]}
        result.append((f"update-native-{n}", {
            "snapshot": initial,
            "program": ".decl name(n:node,s:symbol)\n.input name\n.decl named(n:node,s:symbol)\nnamed(n,s) :- name(n,s).\n.decl total(k:number)\ntotal(k) :- k = count : {name(_,_)}.\n.output named,total\n",
            "snapshots": [edited, initial, edited, initial],
            "queries": ["SELECT * FROM total", "SELECT count(*) FROM named"],
        }))
        result.append((f"update-native-files-{n}", {
            **result[-1][1], "snapshots": [],
            "file_changes": [{"upsert": [edited["files"][0]]}, {"upsert": [files[0]]}, {"upsert": [edited["files"][0]]}, {"upsert": [files[0]]}],
        }))
    for n in [10000, 100000]:
        result.append((f"update-reducers-{n}", {
            "relations":[relation("event",["id","g","v"],[[i,1,i%31] for i in range(n)]),relation("owner",["g"],[[i] for i in range(n)])],
            "program":""".decl event(id:number,g:number,v:number)
.decl owner(g:number)
.input event,owner
.decl total(n:number,s:number,lo:number,hi:number)
total(n,s,lo,hi) :- n = count : {event(_,_,_)}, s = sum v : {event(_,_,v)}, lo = min v : {event(_,_,v)}, hi = max v : {event(_,_,v)}.
.decl grouped(g:number,n:number,s:number,lo:number,hi:number)
grouped(g,n,s,lo,hi) :- owner(g), n = count : {event(_,g,_)}, s = sum v : {event(_,g,v)}, lo = min v : {event(_,g,v)}, hi = max v : {event(_,g,v)}.
.decl vacant(g:number)
vacant(g) :- owner(g), !event(_,g,_).
.output total,grouped,vacant
""",
            "changes":[[{"name":"event","insert":[[n,1,17]]}],[{"name":"event","delete":[[n,1,17]]}],[{"name":"event","delete":[[0,1,0]]}],[{"name":"event","insert":[[n+1,2,99]]}],[{"name":"event","delete":[[n+1,2,99]]}],[{"name":"event","insert":[[n+2,1,30]]}],[{"name":"event","delete":[[n+2,1,30]]}]],
            "queries":["SELECT * FROM total","SELECT * FROM grouped","SELECT count(*),sum(c0) FROM vacant"],
        }))
    result.append(("fold-antijoin-512", {
        "relations": [relation("edge", ["src", "dst"], [[i, i+1] for i in range(511)])],
        "program": CLOSURE,
        "after_run_sql": ["CREATE TEMP TABLE candidate(c0 INTEGER,c1 INTEGER,PRIMARY KEY(c0,c1)) WITHOUT ROWID; INSERT INTO candidate SELECT c0,c1 FROM reach WHERE c0=510 AND c1=511",
                          "CREATE TEMP TABLE fold_except(c0 INTEGER,c1 INTEGER,PRIMARY KEY(c0,c1)) WITHOUT ROWID; CREATE TEMP TABLE fold_probe(c0 INTEGER,c1 INTEGER,PRIMARY KEY(c0,c1)) WITHOUT ROWID",
                          "INSERT INTO fold_except SELECT * FROM candidate EXCEPT SELECT * FROM reach",
                          "INSERT INTO fold_probe SELECT * FROM candidate c WHERE NOT EXISTS (SELECT 1 FROM reach r WHERE r.c0=c.c0 AND r.c1=c.c1)"],
        "queries": ["SELECT count(*) FROM fold_except", "SELECT count(*) FROM fold_probe"],
    }))
    for n in [1024, 4096, 16384]:
        edges = [[(i-1)//2, i] for i in range(1, n) if i % 1024 != 0]
        providers = [[i] for i in range(n) if i % 257 == 0]
        consumers = [[i] for i in range(n) if i % 7 == 0]
        result.append((f"react-boundaries-{n}", {
            "relations": [relation("render", ["parent", "child"], edges),
                          relation("provider", ["id"], providers), relation("consumer", ["id"], consumers)],
            "program": """.decl render(p: number, c: number)
.decl provider(n: number)
.decl consumer(n: number)
.input render, provider, consumer
.decl beneath(n: number, boundary: number)
beneath(n, p) :- provider(p), render(p, n).
beneath(n, b) :- beneath(p, b), render(p, n).
.decl nested(n: number, boundary: number)
nested(n, b) :- provider(n), beneath(n, b).
.decl uncovered(n: number)
uncovered(n) :- consumer(n), !beneath(n, _).
.output nested, uncovered
""",
            "queries": ["SELECT c0,c1 FROM nested ORDER BY c0,c1", "SELECT c0 FROM uncovered ORDER BY c0",
                        "SELECT count(*) FROM beneath"],
        }))
    for n in [10000, 100000]:
        result.append((f"aggregate-{n}", {
            "relations": [relation("event", ["id", "owner", "value"], [[i, i % 1000, i % 31] for i in range(n)]),
                          relation("owner", ["id"], [[i] for i in range(1000)]),
                          relation("blocked", ["id"], [[i] for i in range(0, 1000, 5)])],
            "program": """.decl event(id: number, owner: number, value: number)
.decl owner(id: number)
.decl blocked(id: number)
.input event, owner, blocked
.decl totals(owner: number, n: number, amount: number)
totals(o,n,s) :- owner(o), !blocked(o), n = count : { event(_,o,_) }, s = sum v : { event(_,o,v) }.
.output totals
""",
            "queries": ["SELECT c0,c1,c2 FROM totals ORDER BY c0",
                        "SELECT e.owner,count(*),sum(e.value) FROM event e WHERE NOT EXISTS (SELECT 1 FROM blocked b WHERE b.id=e.owner) GROUP BY e.owner ORDER BY e.owner"],
        }))
    for name, request in list(result):
        if name.startswith("aggregate-"):
            n=int(name.split("-")[-1])
            result.append(("update-"+name,dict(request,changes=[
                [{"name":"event","insert":[[n,1,17]]}],
                [{"name":"event","delete":[[n,1,17]]}],
            ])))
    for n in [10000, 100000]:
        result.append((f"secondary-join-{n}", {
            "relations": [relation("event", ["id", "owner"], [[i, i % 1000] for i in range(n)]),
                          relation("enabled", ["owner"], [[i] for i in range(1000)]),
                          relation("target", ["owner"], [[i] for i in range(1000)])],
            "program": """.decl event(id: number, owner: number)
.decl enabled(owner: number)
.decl target(owner: number)
.input event, enabled, target
.decl picked(id: number, owner: number)
picked(i,o) :- event(i,o), enabled(o).
.decl hit(id: number)
hit(i) :- target(o), picked(i,o).
.output hit
""",
            "after_run_sql": [
                "SELECT count(*) FROM target t CROSS JOIN picked p ON p.c1=t.owner",
                "SELECT count(*) FROM target t CROSS JOIN picked p ON p.c1=t.owner",
                "CREATE INDEX temp.bench_picked_owner ON picked(c1)",
                "SELECT count(*) FROM target t CROSS JOIN picked p ON p.c1=t.owner"],
            "queries": ["SELECT count(*) FROM hit", "EXPLAIN QUERY PLAN SELECT p.c0 FROM target t CROSS JOIN picked p ON p.c1=t.owner"],
        }))
    for count in [1, 9]:
        files = [{"path": p, "source": s, "language": "python"} for p, s in corpus[:count]]
        result.append((f"core-bind-{count}", {"snapshot": {"files": files}, "queries": [
            "SELECT count(*) FROM node", "SELECT count(*) FROM name", "SELECT sym_text(path) FROM doc",
            "SELECT d.d, count(*) FROM doc d JOIN node n ON n.d=d.d GROUP BY d.d",
            "WITH counts AS (SELECT d,kind,count(*) AS n FROM node GROUP BY d,kind), ranked AS (SELECT *,row_number() OVER (PARTITION BY d ORDER BY n DESC,kind) AS rank FROM counts) SELECT sym_text(doc.path),rank,kind,n FROM ranked JOIN doc USING(d) WHERE rank<=3 ORDER BY doc.path,rank"]}))
    tiny = {"files": [{"path": "a.py", "language": "python", "source": "def f():\n    return 42\n"}]}
    result.extend([
        ("feature-global-count", {"relations": [relation("item", ["id"], [[1],[2]])], "program": ".decl item(x:number)\n.input item\n.decl total(n:number)\ntotal(n) :- n = count : { item(_) }.\n.output total\n", "queries":["SELECT c0 FROM total"]}),
        ("feature-zero-arity", {"program": ".decl enabled()\nenabled().\n.output enabled\n", "queries":["SELECT EXISTS(SELECT 1 FROM enabled)"]}),
        ("feature-min-lattice", {"program": ".decl distance(n:number,v:min<>) widen 100\ndistance(1,0).\n.output distance\n", "queries":["SELECT * FROM distance"]}),
        ("feature-symbol-input", {"relations": [relation("label", ["id", "name"], [[1, "hello"]])],"queries":["SELECT id,sym_text(name) FROM label"]}),
        ("feature-kind-decoding", {"snapshot": tiny, "queries": ["SELECT DISTINCT kind_name('python',kind) FROM node ORDER BY 1"]}),
        ("missing-precision", {"queries": ["SELECT 9007199254740993 AS id"]}),
        ("missing-repeat", {"relations": [relation("edge", ["src", "dst"], [[1,2],[2,3]])], "program": CLOSURE, "queries": ["SELECT count(*) FROM reach"], "repeat": True}),
        ("missing-wave-budget", {"relations": [relation("edge", ["src", "dst"], [[i,i+1] for i in range(1100)]), relation("seed",["id"],[[0]])], "program": ".decl edge(a:number,b:number)\n.decl seed(n:number)\n.input edge,seed\n.decl reached(n:number)\nreached(n) :- seed(n).\nreached(n) :- reached(p),edge(p,n).\n.output reached\n"}),
    ])
    retry = dict(result[-1][1], max_waves=4096, queries=["SELECT count(*) FROM reached"])
    result.append(("wave-budget-raised", retry))
    for name in ["workload", "calls"]:
        src = (ROOT/f"native/src/code_rule_dsl/logic/plan/golden/programs/{name}.rr").read_text()
        result.append((f"wave-{name}-cpython", {"snapshot": {"files": [{"path":p,"source":s,"language":"python"} for p,s in corpus]}, "program":src, "queries":["SELECT count(*) FROM finding"]}))
        # Diagnostic workaround only: avoid the projection names already bound by the session.
        # Explicit rules retain the same node projections; the original case remains above.
        renamed = re.sub(r"\b(kind|docof)(?=\s*\()", lambda m: "bench_"+m[0], src)
        renamed = re.sub(r"(?m)^\.input (.*)$", lambda m: ".input "+", ".join(x.strip() for x in m[1].split(",") if x.strip() not in {"kind", "docof"}), renamed)
        renamed += "\nbench_kind(n,k) :- node(_,n,k,_,_).\nbench_docof(n,d) :- node(d,n,_,_,_).\n"
        outputs = ["type_of", "cv_const"] if name == "workload" else ["call_target", "call_open", "base", "async_open", "chain_step", "calls"]
        queries = []
        for out in outputs:
            nodes = 1 if out in {"type_of", "cv_const", "async_open", "call_open"} else 2
            cols = [f"sym_text(d{i}.path)||':'||(r.c{i}&4294967295)" for i in range(nodes)]
            if out == "type_of": cols.append("sym_text(r.c1)")
            if out == "cv_const": cols.append("r.c1")
            joins = " ".join(f"JOIN doc d{i} ON d{i}.d=(r.c{i}>>32)" for i in range(nodes))
            queries.append("SELECT "+",".join(cols)+f" FROM {out} r "+joins+" ORDER BY "+",".join(str(i+1) for i in range(len(cols))))
        result.append((f"wave-{name}-renamed", {"snapshot": {"files": [{"path":p,"source":s,"language":"python"} for p,s in corpus]}, "program":renamed,
            "queries":queries}))
        inlined = re.sub(r"(?m)^\.decl (kind|docof)\(.*\)\n", "", src)
        inlined = re.sub(r"(?m)^\.input (.*)$", lambda m: ".input "+", ".join(x.strip() for x in m[1].split(",") if x.strip() not in {"kind", "docof"}), inlined)
        inlined = re.sub(r"\bkind\(([^,()]+),([^()]+)\)", r"node(_,\1,\2,_,_)", inlined)
        inlined = re.sub(r"\bdocof\(([^,()]+),([^()]+)\)", r"node(\2,\1,_,_,_)", inlined)
        result.append((f"wave-{name}-inlined", {"snapshot": {"files": [{"path":p,"source":s,"language":"python"} for p,s in corpus]}, "program":inlined,"queries":queries}))
    return result


def validate(name, request, result):
    """Use Python oracles, SQL references, and explicit expected failures."""
    expected_errors = {"missing-wave-budget": "fixpoint"}
    if name in expected_errors:
        assert expected_errors[name] in (result["error"] or ""), result["error"]
        return "expected failure reproduced"
    assert not result["error"], result["error"]
    queries = result["result"]["queries"]
    assert all(not q["truncated"] for q in queries), "truncated result"
    rows = [q["rows"] for q in queries]
    if name.startswith("update-recursive-lattice-"):
        groups = int(name.split("-")[-1])
        assert rows == [[[170,0],[171,2],[172,1],[173,4]], [[groups*4]]], rows
        for index, (update, expected) in enumerate(zip(result["result"]["updates"], [(2,2),(0,4),(4,0)]*2, strict=True)):
            assert not update["initializations"], update
            assert not update["migrated_inputs"], update
            if index >= 3:
                assert update["scratch_tables_created"] == 0, update
            report = update["programs"][0]
            assert not report["refreshed_strata"], report
            assert report["recursive_frames"] > 0, report
            assert report["outputs"]["distance"] == dict(zip(["inserted","deleted"],expected)), report
        for phase in result["phases"]:
            if phase["phase"].startswith("update-"):
                assert phase["vm_steps"] < 50000, phase["vm_steps"]
        return "passed"
    if name.startswith("update-native-"):
        n = int(name.split("-")[-1])
        assert rows == [[[n*3]], [[n*3]]], rows
        updates = result["result"]["updates"]
        assert len(updates) == 4, updates
        for update in updates:
            assert update["rebuilt_files"] == 1, update
            assert update["changed_files"] == ["m0.py"], update
            assert not update["programs"][0]["refreshed_strata"], update
            if name.startswith("update-native-files-"):
                assert update["examined_files"] == 1, update
                assert update["metadata_nodes"] < 2000, update
        return "passed"
    if name.startswith("update-reducers-"):
        n=int(name.split("-")[-1]);total=[n-1,sum(i%31 for i in range(n)),0,30]
        assert rows==[[total],[[1,*total]],[[n-1,n*(n-1)//2-1]]],rows
        for update in result["result"]["updates"]:
            assert not update["initializations"],update
            report=update["programs"][0]
            assert not report["refreshed_strata"],report
            assert report["context_rows"]<=5,report
        return "passed"
    if name.startswith("update-closure-"):
        n=int(name.split("-")[-1])
        assert rows == [[[n*(n-1)//2+1,sum(a*(n-a-1) for a in range(n))+20000,sum(b*b for b in range(n))+20001]]],rows
        updates=result["result"]["updates"]
        expected=[(1,0),(1,0),(0,1),(n,0),(0,n)]
        for update,(inserted,deleted) in zip(updates,expected):
            report=update["programs"][0]
            assert report["outputs"]["reach"] == {"inserted":inserted,"deleted":deleted},report
            assert not report["refreshed_strata"],report
        return "passed"
    if name.startswith("closure-"):
        n = int(name.split("-")[-1])
        expected = [[n*(n-1)//2, 0, n-1, sum(a*(n-a-1) for a in range(n)), sum(b*b for b in range(n))]]
        assert rows[0] == rows[1] == expected and rows[2] == [[0]], rows
    elif name.startswith("react-boundaries-"):
        inputs = {r["name"]:r["rows"] for r in request["relations"]}
        parents = {child:parent for parent,child in inputs["render"]}
        providers = {p for p, in inputs["provider"]}
        consumers = {c for c, in inputs["consumer"]}
        below = set()
        for node in range(int(name.split("-")[-1])):
            parent = parents.get(node)
            while parent is not None:
                if parent in providers: below.add((node,parent))
                parent = parents.get(parent)
        nested = sorted([n,p] for n,p in below if n in providers)
        covered = {n for n,_ in below}
        uncovered = sorted([n] for n in consumers if n not in covered)
        assert rows == [nested,uncovered,[[len(below)]]], rows
    elif name.startswith(("aggregate-","update-aggregate-")):
        n = int(name.split("-")[-1]); totals = [[i,0,0] for i in range(1000)]
        for i in range(n): totals[i%1000][1] += 1; totals[i%1000][2] += i%31
        assert rows[0] == rows[1] == [r for r in totals if r[0]%5], rows
        if name.startswith("update-"):
            for update in result["result"]["updates"]:
                assert not update["initializations"],update
                assert not update["programs"][0]["refreshed_strata"],update
                assert update["programs"][0]["context_rows"]==1,update
    elif name.startswith("secondary-join-"):
        assert rows[0] == [[int(name.split("-")[-1])]], rows
    elif name.startswith("core-bind-"):
        assert sum(r[1] for r in rows[3]) == rows[0][0][0], rows
        assert len(rows[4]) == 3*int(name.split("-")[-1]), rows
    elif name == "fold-antijoin-512": assert rows == [[[0]],[[0]]], rows
    elif name == "wave-budget-raised": assert rows == [[[1101]]], rows
    elif name == "missing-precision": assert rows == [[[{"integer":"9007199254740993"}]]], rows
    elif name == "missing-repeat": assert "Ok" in result["result"]["repeated"][0]["same_program"]
    elif name == "feature-global-count": assert rows == [[[2]]], rows
    elif name == "feature-zero-arity": assert rows == [[[1]]], rows
    elif name == "feature-min-lattice": assert rows == [[[1,0]]], rows
    elif name == "feature-symbol-input": assert rows == [[[1,"hello"]]], rows
    elif name == "feature-kind-decoding": assert ["function_definition"] in rows[0] and all(r[0] for r in rows[0]), rows
    return "passed"


def run(command, timeout, env=None):
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.perf_counter()
    try:
        p = subprocess.run(command, capture_output=True, text=True, timeout=timeout, env=env)
        result = {"returncode":p.returncode,"stdout":p.stdout,"stderr":p.stderr}
    except subprocess.TimeoutExpired as e:
        def text(s): return s.decode(errors="replace") if isinstance(s, bytes) else s or ""
        result = {"timeout":timeout,"stdout":text(e.stdout),"stderr":text(e.stderr)}
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    result.update(wall_s=time.perf_counter()-started, cpu_s=(after.ru_utime+after.ru_stime)-(before.ru_utime+before.ru_stime))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runner", required=True)
    parser.add_argument("--engine", required=True)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--repeats", default=3, type=int)
    parser.add_argument("--timeout", default=45, type=float)
    parser.add_argument("--only", default="")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    corpus = json.loads(subprocess.check_output(["zstd", "-dc", str(ROOT/"native/src/code_rule_dsl/facts/vtab/golden/corpus.json.zst")]))
    corpus_root = args.out/"cpython"
    for path, source in corpus:
        target = corpus_root/path; target.parent.mkdir(parents=True, exist_ok=True); target.write_text(source)
    records = []
    for name, request in cases(corpus):
        if args.only and args.only not in name: continue
        directory = args.out/name; directory.mkdir(exist_ok=True)
        path = directory/"request.json"; path.write_text(json.dumps(request))
        repeats = 1 if name.startswith(("missing-", "feature-")) else args.repeats
        for trial in range(repeats):
            result = run([args.runner,str(directory/f"store-{trial}.sqlite"),str(path)],args.timeout)
            stdout = result.pop("stdout")
            (directory/f"trial-{trial}.stdout.json").write_text(stdout)
            (directory/f"trial-{trial}.stderr.txt").write_text(result.pop("stderr"))
            if result.get("returncode") == 0:
                try: result["validation"] = validate(name, request, json.loads(stdout))
                except (AssertionError, KeyError, TypeError, ValueError) as error: result["validation_error"] = str(error)
                if name == "missing-precision":
                    decoded = subprocess.check_output(["node", "-e", "const fs=require('node:fs'); const n=JSON.parse(fs.readFileSync(0,'utf8')).result.queries[0].rows[0][0]; console.log(JSON.stringify({decoded:n,exact:BigInt(n.integer??n)===9007199254740993n}));"], input=stdout, text=True)
                    result["javascript_integer"] = json.loads(decoded)
            records.append({"case":name,"trial":trial,**result})
            print(name,trial,result,flush=True)
            if "timeout" in result: break
    if not args.only or args.only == "corpus":
        for workload in ["workload", "workload_demand", "calls"]:
            program = ROOT/f"native/src/code_rule_dsl/logic/plan/golden/programs/{workload}.rr"
            for trial in range(args.repeats):
                modes = ["sql","native"] if trial%2==0 else ["native","sql"]
                for mode in modes:
                    directory = args.out/f"corpus-{workload}-{mode}-{trial}"; directory.mkdir(exist_ok=True)
                    env = {**os.environ,"RERAIL_CORPUS_MEMO":"off","RERAIL_TAPE_CACHE":str(args.out/"tapes.sqlite")}
                    if trial == 0:
                        env.update(RERAIL_PROFILE=str(directory/"profile"),RDL_STMTS=str(directory/"sql.tsv"))
                    flags = ["--sql"] if mode=="sql" else ["--modes","program"]
                    result = run([args.engine,"logic",str(program),str(corpus_root),str(directory/"out"),*flags],args.timeout,env)
                    (directory/"stdout.txt").write_text(result.pop("stdout")); (directory/"stderr.txt").write_text(result.pop("stderr"))
                    outputs = {p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in (directory/"out").glob("*.csv")}
                    records.append({"case":f"corpus-{workload}","mode":mode,"trial":trial,"profiled":trial==0,"outputs":outputs,**result})
                    print(workload,mode,trial,result,flush=True)
                    if "timeout" in result: break
    (args.out/"runs.json").write_text(json.dumps(records,indent=2)+"\n")
    failures = [r for r in records if "timeout" in r or r.get("returncode") != 0 or "validation_error" in r]
    for workload in ["workload", "workload_demand", "calls"]:
        outputs = [r["outputs"] for r in records if r["case"] == "corpus-"+workload]
        if outputs and (not outputs[0] or any(o != outputs[0] for o in outputs)):
            failures.append({"case":workload,"validation_error":"SQL/native CSV output differs or is empty"})
    if failures: raise SystemExit(json.dumps(failures,indent=2))


if __name__ == "__main__":
    if sys.argv[1:2] == ["--recursive-edit-request"]:
        parser = argparse.ArgumentParser(description="Generate repeated sparse recursive edits for CPU measurements.")
        parser.add_argument("--recursive-edit-request", type=int, required=True, metavar="REGIONS")
        parser.add_argument("--cycles", type=int, default=10)
        parser.add_argument("output", type=Path)
        args = parser.parse_args()
        if args.recursive_edit_request < 18 or args.cycles < 1:
            parser.error("use at least 18 regions and one cycle")
        request = recursive_lattice(args.recursive_edit_request)
        request["changes"] *= args.cycles
        args.output.write_text(json.dumps(request))
    elif sys.argv[1:2] == ["--session-request"]:
        parser = argparse.ArgumentParser(description="Generate a recursive request with rollback and recovery checks.")
        parser.add_argument("--session-request", type=int, required=True, metavar="REGIONS")
        parser.add_argument("output", type=Path)
        args = parser.parse_args()
        args.output.write_text(json.dumps(session_request(args.session_request)))
    else:
        main()
