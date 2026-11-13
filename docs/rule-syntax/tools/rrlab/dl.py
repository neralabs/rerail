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
- A record with no code (an internal entry's plain dict) gets `--codeless CODE`, or the check's name. When the rule
  has only one code, every recorded finding is compared under it: an internal entry may give the message instead.
  An event whose recorded codes are none of the rule's (fixed) codes is compared on (path, line) only, and counted. A record whose
  call raised is left out.
- A file with a syntax error is withheld, as core.rr says: only `.on syntax_error`'s finding is kept for it.
- Events with more than `--max-files` files (500) are not run: rrlab's facts grow with the whole tree.
- Events the upstream run did not pass, and retained ones, are left out. Events that compare messages or
  fields are compared on their keys.
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
            # compare_message / compare_fields events are compared on their keys only, like every other
            if e.get("path_skip_override"):
                continue
            files = e.get("files") or rec.get("files") or []
            out.append({"batch": os.path.basename(p).split(".")[0], "test": e["test"], "entry": e.get("entry", ""),
                        "options": (e.get("oracle_config") or {}).get("options") or {},
                        "production": bool(e.get("production_scope")),
                        "files": [(f["path"], f["source"]) for f in files],
                        "want": wanted(e["findings"], [f["path"] for f in files], check)})
    for p in sorted(glob.glob(os.path.join(fixtures, "batch-*-complete-api.json*"))):
        d = _load(p)
        ok = {t for t, phases in d.get("cases", {}).items() if all(x.get("outcome") == "passed" for x in phases)}
        src = d.get("sources", {})
        for r in d.get("records", []):
            if r.get("check") != check or (ok and r["test"] not in ok):
                continue
            lit = lambda v: ast.literal_eval(v) if isinstance(v, str) else v
            exp = lit(r["expected"])
            if not isinstance(exp, list):   # the call raised ({"exception": …}): nothing to compare
                continue
            files = [(f["path"], src[f["sourceSha256"]]) for f in lit(r["files"]) if f.get("sourceSha256") in src]
            params = r.get("parameters") or "{}"
            out.append({"batch": os.path.basename(p).split(".")[0], "test": r["test"], "entry": r.get("entry", ""),
                        "options": json.loads(params) if isinstance(params, str) else params, "production": False,
                        "files": files, "want": wanted(exp, [f for f, _ in files], check)})
    return out


def wanted(findings, paths, check):
    """The keys of recorded findings, whatever shape the entry function returned them in: dicts keyed path, file or
    file_path, line or line_number (None is line 1), code or rule; or "path:line: [CODE: ]message" strings. A
    finding with no code gets the check's name (see --codeless). A recorded path that ends with one of the event's
    file paths is that file (some entries report absolute paths)."""
    def norm(fp):
        return next((f.lstrip("/") for f in paths if fp == f or fp.endswith("/" + f.lstrip("/"))), fp.lstrip("/"))
    out = set()
    for x in findings:
        if isinstance(x, str):
            m = re.match(r"(.*?):(\d+): (?:([A-Z][A-Z0-9_-]*\d+): )?", x)
            if m:
                out.add((norm(m.group(1)), int(m.group(2)), m.group(3) or check))
            continue
        fp = x.get("path") or x.get("file") or x.get("file_path") or "?"
        line = x.get("line", x.get("line_number"))
        out.add((norm(fp), int(line or 1), x.get("code") or x.get("rule") or check))
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
    text = open(rule).read()
    codes_of = dict(re.findall(r'^\.violation\s+(\w+)\([^)]*\)\s*code\s+"([^"]+)"', text, re.M))
    names = re.findall(r'^\.violation\s+(\w+)\(', text, re.M)
    one = {codes_of.get(n, n) for n in names}
    # the rule's codes, when every violation's code is fixed (a `code {c}` makes them open)
    closed = not re.search(r'^\.violation\s+\w+\([^)]*\)\s*code\s+\{', text, re.M)
    rule_codes = one
    byplace = [0]
    if len(one) == 1 and closed and not args.codeless:
        # a rule with one code (a plugin that reports every finding as PHA001, say): every recorded finding has
        # it, whatever an internal entry put in its place (nothing, or the message)
        (code,) = one
        for e in evs:
            e["want"] = {(p, l, code) for p, l, _ in e["want"]}
    if args.codeless:   # a record that carries no code: the code the linter's plugin gives every finding
        for e in evs:
            e["want"] = {(p, l, args.codeless if c == check else c) for p, l, c in e["want"]}
    reported = collections.defaultdict(set)   # entry -> the codes it reports in any event, before the filters
    for e in evs:
        reported[e["entry"]] |= {c for _, _, c in e["want"]}
    big = [e for e in evs if len(e["files"]) > args.max_files]
    evs = [e for e in evs if len(e["files"]) <= args.max_files]
    if args.entry:
        evs = [e for e in evs if e["entry"] == args.entry]
    if args.test:
        evs = [e for e in evs if args.test in e["test"]]
    if not evs:
        raise SystemExit(f"rrlab dl: no events for {check} under {fixtures}")
    log = log_codes(text, codes_of)
    base = os.path.join(here, "target", "dl", f"{check}-{os.getpid()}")
    shutil.rmtree(base, ignore_errors=True)

    def one(k, e):
        root = os.path.join(base, f"e{k}")
        for path, source in e["files"]:
            p = os.path.join(root, path.lstrip("/"))   # an absolute recorded path stays inside the event's directory
            os.makedirs(os.path.dirname(p), exist_ok=True)
            open(p, "w", encoding="utf-8", errors="surrogateescape").write(source)
        if not e["files"]:
            os.makedirs(root, exist_ok=True)
        rels = [p.lstrip("/") for p, _ in e["files"]]
        if not rels:
            return k, set(), None
        try:
            run = Run(rule, rels, root, options=e["options"] or None)
            run.use_scope = e["production"]
            found = run.execute()
            # core.rr: a document that does not parse is withheld; only `.on syntax_error`'s finding is left
            on = run.prog.on_syntax[0] if run.prog.on_syntax else None
            got = {(f["path"], int(f["line"]), f["code"]) for f in found if not f["syntax_error"] or f["code"] == on}
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
        want = e["want"]
        if closed and any(c not in rule_codes for _, _, c in want):
            # the record's codes are not codes the rule has (an entry returned messages): compare by place only
            want, got = {(p, l) for p, l, _ in want}, {(p, l) for p, l, _ in got}
            byplace[0] += 1
        else:
            codes = reported[e["entry"]]
            if codes:
                got = {g for g in got if g[2] in codes}
        if got == want:
            row[1] += 1
        else:
            misses.append(f"{e['test']} [{e['entry']}]: missing {sorted(want - got)} extra {sorted(got - want)}")
    tot = [sum(r[i] for r in per.values()) for i in range(3)]
    for b, r in sorted(per.items()):
        print(f"{b:34} {r[0]:4} events {r[1]:4} exact {r[2]:3} errors")
    print(f"{'total':34} {tot[0]:4} events {tot[1]:4} exact {tot[2]:3} errors   ({rule})")
    if byplace[0]:
        print(f"  {byplace[0]} event(s) compared by (path, line) only: their recorded codes are none of the rule's")
    if big:
        print(f"  {len(big)} event(s) with more than {args.max_files} files not run (--max-files): "
              + ", ".join(e["test"] for e in big[:3]))
    for m in misses[:args.show]:
        print("  " + m)
    if not args.keep:
        shutil.rmtree(base, ignore_errors=True)
