"""rrlab dl: score a hand-written data-layer rule against the data-layer port's recorded events.

  rrlab dl CHECK [RULE.rr] [--fixtures DIR] [--entry NAME] [--test TEXT] [--show N] [--jobs N] [--codeless CODE]

CHECK is the linter's name (`bootstrap_cache_health`). RULE defaults to docs/rule-syntax/data-layer/<check-with-
hyphens>.rr. The events come from the Data-layer-rule-expressivity worktree's fixtures/host/data-layer-port:

- batch-NN-recording.json and batch-NN-*probes.json: one event per linter call in its upstream test suite, with
  the files it was given and the findings it returned (path, line, code);
- batch-NN-complete-api.json[.gz]: the same for batches 9 and 10, files given by content hash.

Each event is run on its own files only, in a directory of its own. The comparison is the loose findings contract:
the set of (path, line, code) keys.

- **Entries.** Many events call one of the linter's internal functions (`_collect_file_violations`), which
  reports only some of its codes. The rule is the whole linter, so an event is compared on the codes its entry
  reports in any event of the check. An entry with no finding in any event is compared on every code.
- **Options.** An event's options (`entryPoint`, `targetFunction`, …) emulate those internal calls for the JSON
  ports. They are passed to the rule as `option` rows; a rule may ignore them.
- **Scope.** A direct call takes the files it is given, so `.scope` is ignored, except on production (plugin)
  events, which also drop the rule's `.mode log` codes.
- A record with no code (an internal entry's plain dict) gets the check's name, or `--codeless CODE`. A record whose
  call raised is left out.
- Events that the upstream run did not pass, retained ones, and ones that compare messages or fields are left out.
"""
import ast, collections, concurrent.futures, glob, gzip, json, os, re, shutil


def _load(p):
    return json.load(gzip.open(p, "rt") if p.endswith(".gz") else open(p))


def events(fixtures, check):
    out = []
    for p in sorted(glob.glob(os.path.join(fixtures, "batch-*-recording.json")) +
                    glob.glob(os.path.join(fixtures, "batch-*probes.json"))):
        rec = _load(p)
        for e in rec.get("events", []):
            if e.get("check") != check or e.get("retained_reason") or e.get("upstream_passed") is not True:
                continue
            if any(e.get(k) for k in ("compare_message", "compare_fields")) or e.get("path_skip_override"):
                continue
            files = e.get("files") or rec.get("files") or []
            out.append({"batch": os.path.basename(p).split(".")[0], "test": e["test"], "entry": e.get("entry", ""),
                        "options": (e.get("oracle_config") or {}).get("options") or {},
                        "production": bool(e.get("production_scope")),
                        "files": [(f["path"], f["source"]) for f in files],
                        "want": {(f["path"], int(f["line"]), f["code"]) for f in e["findings"]}})
    for p in sorted(glob.glob(os.path.join(fixtures, "batch-*-complete-api.json*"))):
        d = _load(p)
        ok = {t for t, phases in d.get("cases", {}).items() if all(x.get("outcome") == "passed" for x in phases)}
        src = d.get("sources", {})
        for r in d.get("records", []):
            if r.get("check") != check or (ok and r["test"] not in ok):
                continue
            lit = lambda v: ast.literal_eval(v) if isinstance(v, str) else v
            if not isinstance(lit(r["expected"]), list):   # the call raised ({"exception": …}): nothing to compare
                continue
            files = [(f["path"], src[f["sourceSha256"]]) for f in lit(r["files"]) if f.get("sourceSha256") in src]
            params = r.get("parameters") or "{}"
            out.append({"batch": os.path.basename(p).split(".")[0], "test": r["test"], "entry": r.get("entry", ""),
                        "options": json.loads(params) if isinstance(params, str) else params, "production": False,
                        "files": files, "want": {(f.get("path", f.get("file_path")), int(f.get("line", f.get("line_number")) or 1),
                                           f.get("code", check)) for f in lit(r["expected"])}})
    return out


def log_codes(rule_text, codes_of):
    names = []
    for l in rule_text.splitlines():
        l = l.split("//")[0].strip()
        if l.startswith(".mode log "):
            names += [n.strip() for n in l[len(".mode log "):].split(",") if n.strip()]
    return {codes_of.get(n, n) for n in names}


def score(args, Run, here):
    check = args.check
    rule = os.path.abspath(args.rule or os.path.join(here, "..", "..", "data-layer", check.replace("_", "-") + ".rr"))
    fixtures = args.fixtures or os.path.join(here, "..", "..", "..", "..", "..", "Data-layer-rule-expressivity",
                                             "fixtures", "host", "data-layer-port")
    evs = events(fixtures, check)
    if args.codeless:   # a record that carries no code: the code the linter's plugin gives every finding
        for e in evs:
            e["want"] = {(p, l, args.codeless if c == check else c) for p, l, c in e["want"]}
    if args.entry:
        evs = [e for e in evs if e["entry"] == args.entry]
    if args.test:
        evs = [e for e in evs if args.test in e["test"]]
    if not evs:
        raise SystemExit(f"rrlab dl: no events for {check} under {fixtures}")
    reported = collections.defaultdict(set)
    for e in evs:
        reported[e["entry"]] |= {c for _, _, c in e["want"]}
    text = open(rule).read()
    codes_of = dict(re.findall(r'^\.violation\s+(\w+)\([^)]*\)\s*code\s+"([^"]+)"', text, re.M))
    log = log_codes(text, codes_of)
    base = os.path.join(here, "target", "dl", f"{check}-{os.getpid()}")
    shutil.rmtree(base, ignore_errors=True)

    def one(k, e):
        root = os.path.join(base, f"e{k}")
        for path, source in e["files"]:
            p = os.path.join(root, path)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            open(p, "w", encoding="utf-8", errors="surrogateescape").write(source)
        if not e["files"]:
            os.makedirs(root, exist_ok=True)
        rels = [p for p, _ in e["files"]]
        if not rels:
            return k, set(), None
        try:
            run = Run(rule, rels, root, options=e["options"] or None)
            run.use_scope = e["production"]
            got = {(f["path"], int(f["line"]), f["code"]) for f in run.execute()}
            return k, got, None
        except SystemExit as x:
            return k, set(), str(x)[:300]

    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        results = list(pool.map(lambda ke: one(*ke), enumerate(evs)))
    per = collections.defaultdict(lambda: [0, 0, 0])   # batch -> events, exact, errors
    misses = []
    for (k, got, err), e in zip(results, evs):
        row = per[e["batch"]]
        row[0] += 1
        if err:
            row[2] += 1
            misses.append(f"{e['test']} [{e['entry']}]: error {err}")
            continue
        if e["production"]:
            got = {g for g in got if g[2] not in log}
        codes = reported[e["entry"]]
        if codes:
            got = {g for g in got if g[2] in codes}
        if got == e["want"]:
            row[1] += 1
        else:
            misses.append(f"{e['test']} [{e['entry']}]: missing {sorted(e['want'] - got)} extra {sorted(got - e['want'])}")
    tot = [sum(r[i] for r in per.values()) for i in range(3)]
    for b, r in sorted(per.items()):
        print(f"{b:34} {r[0]:4} events {r[1]:4} exact {r[2]:3} errors")
    print(f"{'total':34} {tot[0]:4} events {tot[1]:4} exact {tot[2]:3} errors   ({rule})")
    for m in misses[:args.show]:
        print("  " + m)
    if not args.keep:
        shutil.rmtree(base, ignore_errors=True)
