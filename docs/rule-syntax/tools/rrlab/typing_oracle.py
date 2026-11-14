"""rrlab typing-oracle: score the Python typing program, run by rrlab, against the CPython runtime oracle.

  rrlab typing-oracle [--fixture DIR] [--show N]

The fixture is the typing branch's fixtures/python/calls: a project, and python.expected.json, which records every
call CPython ran when the project's main ran, with the definitions it entered and the classes of its receivers.
The scoring is the engine's (typing/oracle_tests.rs, score_python):

- a call agrees when the program names exactly the definitions CPython ran, or names none and CPython ran no
  workspace function; a lambda callee is no definition, so a call that ran one departs;
- otherwise it departs: unknown (call_open), no edge (no row), fewer (a strict subset) or wrong (anything else);
- a definition named at a call CPython never ran is listed, as the oracle cannot check it.

Receivers (python.expected.json's "receiver" classes) are scored by the type relations: exact when the program's
type_of set is the recorded classes and no type_open; wider, unknown (type_open with no class) or wrong otherwise.
"""
import collections, json, os

RULE = """.policy "rrlab-typing-oracle" revision 1
.description "every answer of the Python typing program, for scoring"
.language python
.mode log
#include "python/calls.rr"
#include "python/types.rr"
.violation TGT(at: node, f: node) "target"
.violation OPEN(at: node) "open"
.violation TY(at: node, t: symbol) "{t}"
.violation TYOPEN(at: node) "type open"
TGT(c, f) :- call_target(c, f).
OPEN(c) :- call_open(c).
TY(n, t) :- type_of(n, t).
TYOPEN(n) :- type_open(n).
"""


def line_of(src, off):
    return src[:off].count(b"\n") + 1


def score(args, Run, here):
    fixture = args.fixture or os.path.join(here, "..", "..", "..", "..", "..", "Typing-support", "fixtures", "python",
                                           "calls")
    project = os.path.abspath(os.path.join(fixture, "project"))
    expected = json.load(open(os.path.join(fixture, "python.expected.json")))
    files = sorted(os.path.relpath(os.path.join(d, f), project) for d, _, fs in os.walk(project) for f in fs
                   if f.endswith(".py"))
    src = {f: open(os.path.join(project, f), "rb").read() for f in files}
    rule = os.path.join(here, "target", f"typing-oracle-{os.getpid()}.rr")
    os.makedirs(os.path.dirname(rule), exist_ok=True)
    open(rule, "w").write(RULE)
    run = Run(rule, files, project)
    run.execute()
    t = lambda name: run.table(name)
    named = collections.defaultdict(set)   # (file, start, end) -> {(def file, def line)}
    for p, _, s, e, q, _, fs, _ in t("typing_calls__call_target"):
        named[(p, int(s), int(e))].add((q, line_of(src[q], int(fs))))
    unknown = {(p, int(s), int(e)) for p, _, s, e in t("typing_calls__call_open")}
    agree, dep, ran_spans = 0, collections.Counter(), set()
    misses = []
    for f, calls in expected["runtime"].items():
        for c in calls:
            k = (f, c["start"], c["end"])
            ran_spans.add(k)
            ran = {(x[1], x[2]) for x in c["callees"] if x[0] == "def"}
            lam = any(x[0] == "lambda" for x in c["callees"])
            at = f"{f}:{c['line']} `{src[f][c['start']:c['end']].decode()}`"
            if k in unknown:
                d = "unknown"
            elif k in named:
                d = None if named[k] == ran and not lam else "fewer" if named[k] <= ran else "wrong"
            else:
                d = None if not ran and not lam else "no edge"
            if d is None:
                agree += 1
            else:
                dep[d] += 1
                misses.append(f"{d:8} {at}" + (f": named {sorted(named[k])}, ran {sorted(ran)}" if d == "wrong" else ""))
    never = [f"{f}:{line_of(src[f], s)}: named, never run" for (f, s, e), v in named.items() if v and (f, s, e) not in ran_spans]
    print(f"calls: {agree} agree, {sum(dep.values())} depart ({', '.join(f'{v} {k}' for k, v in sorted(dep.items()))}), "
          f"{dep['wrong']} wrong; {len(never)} named but never run")
    # receivers
    types = collections.defaultdict(set)
    for p, _, s, e, ty in t("typing_calls__type_of"):
        types[(p, int(s), int(e))].add(ty)
    topen = {(p, int(s), int(e)) for p, _, s, e in t("typing_calls__type_open")}
    rc = collections.Counter()
    for f, calls in expected["runtime"].items():
        for c in calls:
            r = c.get("receiver")
            if not r:
                continue
            k = (f, r["start"], r["end"])
            want = {cl[0] for cl in r["classes"]}
            got = types.get(k, set())
            if k in topen and not got:
                rc["unknown"] += 1
            elif got == want and k not in topen:
                rc["exact"] += 1
            elif want <= got or k in topen:
                rc["wider"] += 1
            else:
                rc["wrong"] += 1
                misses.append(f"type     {f}:{c['line']}: got {sorted(got)}, ran {sorted(want)}")
    print(f"receivers: {sum(rc.values())}: " + ", ".join(f"{rc[k]} {k}" for k in ("exact", "wider", "unknown", "wrong")))
    print(f"typing program CPU {run.typing_cpu:.1f} s (souffle, every file at once)")
    for m in (misses + never)[:args.show]:
        print("  " + m)
    os.remove(rule)
