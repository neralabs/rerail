#!/usr/bin/env python3
"""rrlab: run a .rr rule on real files through plain Soufflé, and score it against an upstream recording.

  rrlab.py run RULE.rr FILE... [--options JSON] [--lang LANG] [--root DIR] [-v]
  rrlab.py score PACK RULE-ID [RULE.rr] [--all] [--options JSON] [--data DIR] [--show N] [--column]
  rrlab.py lower RULE.rr                 the Soufflé program, with its warnings
  rrlab.py pattern LANG 'PATTERN' [KIND]  the structural rule a backtick pattern compiles to

README.md in this directory: what is modelled and what is not.
"""
import argparse, collections, csv, json, os, re, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import lower as L
import patterns as P

RR = L.RR
REPO = os.path.dirname(os.path.dirname(RR))
SOUFFLE = shutil.which("souffle") or "/opt/homebrew/bin/souffle"
FACTS_BIN = os.path.join(HERE, "target", "release", "rrlab-facts")
FN_DIR = os.path.join(HERE, "target", "functors")
ECMA = (".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx")
SKIP_DIRS = {".git", "node_modules", "__pycache__", ".venv", "venv", "target", "dist", "build"}
csv.field_size_limit(1 << 30)


def log(*a):
    print(*a, file=sys.stderr)


# ---- building the native parts -----------------------------------------------------------------------------------

def ensure_built():
    src = [os.path.join(HERE, p) for p in ("src/main.rs", "build.rs", "Cargo.toml")]
    if not os.path.exists(FACTS_BIN) or max(map(os.path.getmtime, src)) > os.path.getmtime(FACTS_BIN):
        log("rrlab: building rrlab-facts (cargo build --release --offline) …")
        subprocess.run(["cargo", "build", "--release", "--offline", "--quiet"], cwd=HERE, check=True)
    lib = os.path.join(FN_DIR, "librrlabfn.dylib" if sys.platform == "darwin" else "librrlabfn.so")
    cpp = os.path.join(HERE, "functors", "functors.cpp")
    if not os.path.exists(lib) or os.path.getmtime(cpp) > os.path.getmtime(lib):
        os.makedirs(FN_DIR, exist_ok=True)
        subprocess.run([os.environ.get("CXX", "c++"), "-std=c++17", "-O2", "-shared", "-fPIC", "-o", lib, cpp], check=True)
        if sys.platform == "darwin":  # souffle looks for lib<name>.so on some builds
            alt = os.path.join(FN_DIR, "librrlabfn.so")
            if os.path.lexists(alt):
                os.remove(alt)
            os.symlink(os.path.basename(lib), alt)


# ---- inputs beyond the tree --------------------------------------------------------------------------------------

def tsv(rows):
    out = []
    for r in rows:
        cells = []
        for c in r:
            s = str(c)
            if any(ch in s for ch in '\t\n\r"'):
                s = '"' + s.replace('"', '""') + '"'
            cells.append(s)
        out.append("\t".join(cells) + "\n")
    return "".join(out)


def option_rows(opts):
    """rerail.policy/v1 `options`: {"name": value}. A list gives one option row per element and option_item rows
    with positions; true/false are "true"/"false"; nested objects are flattened with '/'.
    An ESLint-style array ["all", "consistent"] or ["error", {...}] is accepted too: its strings are option,
    option2, …; its objects are merged in."""
    if isinstance(opts, list):
        merged, k = {}, 0
        for x in opts:
            if isinstance(x, dict):
                merged.update(x)
            else:
                merged["option" if k == 0 else f"option{k + 1}"] = x
                k += 1
        opts = merged
    o, oi = [], []

    def scalar(v):
        return "true" if v is True else "false" if v is False else "null" if v is None else str(v)

    def walk(prefix, v):
        if isinstance(v, dict):
            for k2, x in v.items():
                walk(f"{prefix}/{k2}" if prefix else k2, x)
        elif isinstance(v, list):
            for i, x in enumerate(v):
                o.append((prefix, scalar(x)))
                oi.append((prefix, i, scalar(x)))
        else:
            o.append((prefix, scalar(v)))

    walk("", opts or {})
    return o, oi


def flatten_json(prefix, v, out):
    if isinstance(v, dict):
        for k, x in v.items():
            flatten_json(f"{prefix}/{k}" if prefix else k, x, out)
    elif isinstance(v, list):
        for i, x in enumerate(v):
            flatten_json(f"{prefix}/{i}" if prefix else str(i), x, out)
    else:
        out.append((prefix, "true" if v is True else "false" if v is False else "null" if v is None else str(v)))


def repo_inputs(root, docs):
    """file, package, package_field, package_for from the files under root."""
    files, pkgs = [], {}
    for d, dirs, fs in os.walk(root):
        dirs[:] = [x for x in dirs if x not in SKIP_DIRS]
        for f in fs:
            rel = os.path.relpath(os.path.join(d, f), root).replace(os.sep, "/")
            files.append((rel,))
            if f == "package.json":
                pdir = os.path.dirname(rel)
                try:
                    pkgs[pdir] = json.load(open(os.path.join(d, f)))
                except Exception:
                    pkgs[pdir] = None
    package, field, pfor = [], [], []
    for pdir, j in pkgs.items():
        if isinstance(j, dict):
            if isinstance(j.get("name"), str):
                package.append((pdir, j["name"]))
            rows = []
            flatten_json("", j, rows)
            field += [(pdir,) + r for r in rows]
    for path in docs:
        d = os.path.dirname(path)
        while True:
            if d in pkgs:
                pfor.append((path, d)); break
            if not d:
                break
            d = os.path.dirname(d)
    return {"file": files, "package": package, "package_field": field, "package_for": pfor}


def io_input(spec, root):
    """A policy's `.input r(IO=json|lines|file, filename=…)`, read from root."""
    fn = os.path.join(root, spec.get("filename", ""))
    if not os.path.isfile(fn):
        return [], f"input {spec.get('filename')} is missing"
    io = spec.get("IO", "file")
    if io == "lines":
        return [(i + 1, l.rstrip("\n")) for i, l in enumerate(open(fn))], None
    if io == "file":
        return [tuple(l.rstrip("\n").split("\t")) for l in open(fn) if l.strip()], None
    if io == "json":
        v = json.load(open(fn))
        for step in [s for s in spec.get("path", "").split("/") if s]:
            v = v[int(step)] if isinstance(v, list) else v[step]
        if isinstance(v, dict):
            return [(k, json.dumps(x) if isinstance(x, (dict, list)) else x) for k, x in v.items()], None
        if isinstance(v, list):
            return [(x,) for x in v], None
        return [(v,)], None
    return [], f"IO={io} is not modelled"


# ---- running ------------------------------------------------------------------------------------------------------

class Run:
    def __init__(self, rule, files, root, options=None, lang=None, keep=None, verbose=False):
        ensure_built()
        self.rule, self.files, self.root, self.verbose = rule, files, root, verbose
        self.prog = L.Program(rule, P.Compiler(FACTS_BIN))
        self.work = keep or os.path.join(HERE, "target", "work", os.path.splitext(os.path.basename(rule))[0])
        self.inp, self.out = os.path.join(self.work, "in"), os.path.join(self.work, "out")
        for d in (self.inp, self.out):
            shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
        t0 = time.process_time() + cpu_children()
        cmd = [FACTS_BIN, "facts", self.inp, "--root", root] + (["--lang", lang] if lang else [])
        # one extractor call for every file: documents share one id space
        r = subprocess.run(cmd + files, capture_output=True, text=True)
        if r.returncode:
            raise SystemExit(r.stderr)
        self.prog_text = self.prog.text()
        open(os.path.join(self.work, "rule.dl"), "w").write(self.prog_text)
        self.write_inputs(options)
        self.facts_cpu = time.process_time() + cpu_children() - t0

    def write_inputs(self, options):
        p = self.prog
        o, oi = option_rows(options)
        extra = {"option": o, "option_item": oi}
        docs = [l.split("\t")[1] for l in open(os.path.join(self.inp, "doc.facts")) if l.strip()]
        extra.update(repo_inputs(self.root, docs))
        shutil.copy(os.path.join(self.inp, "value.facts"), os.path.join(self.inp, "static_value.facts"))
        for name, spec in p.io_inputs.items():
            rows, err = io_input(spec, self.root)
            if err:
                p.warn(f"{name}: {err}")
            extra[name] = rows
        for name, rows in extra.items():
            open(os.path.join(self.inp, f"{name}.facts"), "w").write(tsv(rows))
        used = set(re.findall(r"\b([a-z_]\w*)\(", "\n".join(p.lowered)))
        for name in p.input_names + [o + "__prev" for o in p.ordered]:
            f = os.path.join(self.inp, f"{name}.facts")
            if not os.path.exists(f):
                open(f, "w").close()
                if name in used and name not in L.FILLED and not name.endswith("__prev") and \
                        not any(w.startswith(name + " ") for w in p.warnings):
                    p.warn(f"input {name} has no rows in rrlab")

    def souffle(self):
        cmd = [SOUFFLE, "-w", "-F", self.inp, "-D", self.out, "-L", FN_DIR, "-l", "rrlabfn",
               os.path.join(self.work, "rule.dl")]
        env = dict(os.environ, RRLAB_PLACES=os.path.join(self.work, "places.tsv"))
        r = subprocess.run(cmd, capture_output=True, text=True, env=env)
        if r.returncode:
            ls = r.stderr.split("\n")
            keep = [l for i, l in enumerate(ls) if not l.startswith("Warning") and not (i and ls[i - 1].startswith("Warning"))
                    and not (i > 1 and ls[i - 2].startswith("Warning")) and l.strip()]
            msg = "\n".join(keep)[:4000]
            raise SystemExit(f"souffle failed ({os.path.join(self.work, 'rule.dl')}):\n{msg}")
        return r

    def execute(self, max_rounds=60):
        t0 = time.time()
        c0 = cpu_children()
        rounds = 0
        prev = None
        while True:
            self.souffle()
            rounds += 1
            if not self.prog.ordered:
                break
            cur = {o: open(os.path.join(self.out, f"{o}.csv")).read() for o in self.prog.ordered}
            if cur == prev:
                break
            if rounds >= max_rounds:
                self.prog.warn(f"ordered by: no fixpoint after {rounds} rounds")
                break
            prev = cur
            for o, text in cur.items():
                open(os.path.join(self.inp, f"{o}__prev.facts"), "w").write(text)
        self.rounds = rounds
        self.souffle_cpu = cpu_children() - c0
        self.wall = time.time() - t0
        return self.findings()

    def table(self, rel):
        f = os.path.join(self.inp, f"{rel}.facts")
        with open(f, newline="") as fh:
            return list(csv.reader(fh, delimiter="\t", quotechar='"'))

    def findings(self):
        p = self.prog
        line = {r[0]: int(r[1]) for r in self.table("line")}
        col = {r[0]: int(r[1]) for r in self.table("col")}
        docof = {r[0]: r[1] for r in self.table("docof")}
        path = {r[0]: r[1] for r in self.table("doc")}
        errs = {r[0] for r in self.table("syntax_error")}
        places = {}
        pf = os.path.join(self.work, "places.tsv")
        if os.path.exists(pf):
            for l in open(pf):
                i, pth, ln = l.rstrip("\n").split("\t")
                places[i] = (pth, int(ln))
        self.texts = None
        out = []
        for v, meta in p.violations.items():
            f = os.path.join(self.out, f"{v}.csv")
            if not os.path.exists(f):
                continue
            with open(f, newline="") as fh:
                # souffle's rfc4180 writer escapes a quote as \" before doubling it
                rows = [[c.replace('\\"', '"') for c in r] for r in csv.reader(fh, delimiter="\t", quotechar='"')]
            for r in rows:
                at = r[0]
                if at in places:
                    pth, ln = places[at]; c = 0; d = None
                elif at in docof:
                    d = docof[at]; pth = path[d]; ln = line.get(at, 0); c = col.get(at, 0)
                else:
                    continue
                code = meta["code"] if meta["code"] is not None else r[meta["cols"].index(meta["code_col"])]
                out.append({"path": pth, "line": ln, "col": c, "code": code, "violation": v, "row": r,
                            "message": self.message(meta, r, line), "syntax_error": d in errs})
        out = self.suppress(out)
        uniq = {}
        for f in out:
            uniq.setdefault((f["path"], f["line"], f["code"], f["col"]), f)
        return sorted(uniq.values(), key=lambda f: (f["path"], f["line"], f["col"], f["code"]))

    def message(self, meta, row, line):
        def sub(m):
            expr = m.group(1)
            name, _, attr = expr.partition(".")
            if name not in meta["cols"]:
                return m.group(0)
            val = row[meta["cols"].index(name)]
            if attr == "line":
                return str(line.get(val, "?"))
            t = self.prog.decls[meta_name(self.prog, meta)][meta["cols"].index(name)][1]
            if self.prog.base(t) == "node":
                if self.texts is None:
                    self.texts = {r[0]: r[1] for r in self.table("text")}
                return self.texts.get(val, "?")[:60]
            return val
        return re.sub(r"\{([A-Za-z_][\w.]*)\}", sub, meta["message"])

    def suppress(self, found):
        """eslint-disable / eslint-disable-line / eslint-disable-next-line / eslint-enable, and `# noqa`."""
        p = self.prog
        tools = {v: meta["suppress"] for v, meta in p.violations.items()}
        if not any(tools.values()):
            return found
        line = {r[0]: int(r[1]) for r in self.table("line")}
        end = {r[0]: int(r[1]) for r in self.table("end_line")}
        docof = {r[0]: r[1] for r in self.table("docof")}
        path = {r[0]: r[1] for r in self.table("doc")}
        directives = collections.defaultdict(list)   # path -> [(kind, line, end, rules)]
        for nid, body in self.table("comment"):
            b = body.strip()
            m = re.match(r"(eslint-disable-next-line|eslint-disable-line|eslint-disable|eslint-enable)(?:\s+(.*))?$", b, re.S)
            if m:
                rules = m.group(2) or ""
                rules = rules.split("--")[0]
                names = {x.strip() for x in rules.replace("\n", " ").split(",") if x.strip()}
                directives[path[docof[nid]]].append((m.group(1), line[nid], end[nid], names))
            m = re.match(r"noqa(?::\s*(.*))?", b, re.I)
            if m:
                names = {x.strip() for x in re.split(r"[,\s]+", m.group(1) or "") if x.strip()}
                directives[path[docof[nid]]].append(("noqa", line[nid], end[nid], names))
        keep = []
        for f in found:
            names = {r for tool, r in tools[f["violation"]]}
            off = False
            state = False
            for kind, a, b, rules in sorted(directives.get(f["path"], []), key=lambda x: x[1]):
                hit = not rules or rules & names
                if not hit:
                    continue
                if kind == "eslint-disable-line" and a == f["line"]:
                    off = True
                elif kind == "eslint-disable-next-line" and b + 1 == f["line"]:
                    off = True
                elif kind == "noqa" and a == f["line"] and any(t == "ruff" or t == "flake8" for t, _ in tools[f["violation"]]):
                    off = True
                elif kind == "eslint-disable" and a <= f["line"]:
                    state = True
                elif kind == "eslint-enable" and a <= f["line"]:
                    state = False
            if not (off or state):
                keep.append(f)
        return keep


def meta_name(prog, meta):
    for k, v in prog.violations.items():
        if v is meta:
            return k


def cpu_children():
    t = os.times()
    return t.children_user + t.children_system


# ---- score --------------------------------------------------------------------------------------------------------

def data_root(arg, pack):
    """The checkout whose packs/<pack>/upstream/*/fixtures.expected.json the score reads."""
    import glob
    for c in [arg, os.environ.get("RRLAB_DATA"), REPO, os.path.join(os.path.dirname(REPO), "Data-layer-rule-expressivity")]:
        if c and glob.glob(os.path.join(c, "packs", pack, "upstream", "*", "fixtures.expected.json")):
            return c
    raise SystemExit(f"no packs/{pack}/upstream/*/fixtures.expected.json found: pass --data DIR or set RRLAB_DATA")


def score(args):
    data = data_root(args.data, args.pack)
    pack = os.path.join(data, "packs", args.pack)
    conf = json.load(open(os.path.join(pack, "conformance.json")))
    entry = next((r for r in conf["rules"] if r["id"] == args.rule_id), None)
    rule = args.rule or os.path.join(RR, "packs", args.pack, args.rule_id + ".rr")
    if not os.path.exists(rule):
        cands = [f for f in os.listdir(os.path.join(RR, "packs", args.pack)) if f.endswith(".rr") and args.rule_id in f]
        if len(cands) == 1:
            rule = os.path.join(RR, "packs", args.pack, cands[0])
    if entry is None:
        # a rule the pack's conformance.json does not list (yet): take the tool and rule from the .rr's code
        prog = L.Program(rule)
        codes = {v["code"] for v in prog.violations.values() if v["code"]}
        tools = [d for d in os.listdir(os.path.join(pack, "upstream")) if os.path.isdir(os.path.join(pack, "upstream", d))]
        upstream = {"tool": tools[0] if len(tools) == 1 else None, "rule": next(iter(codes), args.rule_id)}
        log(f"rrlab: {args.rule_id} is not in {args.pack}/conformance.json; scoring code {upstream['rule']} "
            f"against {upstream['tool']}")
        match = "line"
    else:
        upstream, match = entry["upstream"], entry.get("match", "line")
    if upstream["tool"] is None:
        raise SystemExit("cannot tell which upstream tool to score against")
    rec_dir = os.path.join(pack, "upstream", upstream["tool"])
    rec = json.load(open(os.path.join(rec_dir, "fixtures.expected.json")))
    fixtures = os.path.join(rec_dir, "fixtures")
    urule = upstream["rule"]
    sub = urule.split("/", 1)[1] if "/" in urule and not os.path.isdir(os.path.join(fixtures, urule)) else urule
    # a unit is a file, or [file, context files…] (a package.json the file is linted under)
    allfiles = [f[0] if isinstance(f, list) else f for f in rec["source"]["files"]]
    files = allfiles if args.all else [f for f in allfiles if f.startswith(sub + "/")]
    expected = [f for f in rec["findings"] if f["rule"] == urule and f["path"] in set(files)]
    print(f"rule      {os.path.relpath(rule, RR)}")
    print(f"upstream  {rec['tool']} {rec['version']} {urule}  ({os.path.relpath(rec_dir, data)})")
    print(f"files     {len(files)} ({'every fixture' if args.all else 'fixtures/' + sub + '/'}); "
          f"{len(expected)} recorded findings")
    if not files:
        print(f"score     n/a: the recording has no fixtures for {urule}")
        return
    t0 = time.time()
    # Files a recording gives their own language (source.units[path].languageOptions) run as their own group, with
    # that language as options: sourceType, and parserOptions.ecmaFeatures' globalReturn and impliedStrict.
    units = rec["source"].get("units") or {}
    groups = collections.defaultdict(list)
    for f in files:
        groups[json.dumps(unit_options(units.get(f)), sort_keys=True)].append(f)
    user = json.loads(args.options) if args.options else {}
    found, runs = [], []
    for k, (g, members) in enumerate(sorted(groups.items())):
        opts = dict(option_dict(user), **json.loads(g))
        keep = args.keep and (args.keep if len(groups) == 1 else f"{args.keep}-{k}")
        run = Run(rule, members, fixtures, options=opts, keep=keep)
        found += run.execute()
        runs.append(run)
    if len(groups) > 1:
        print(f"groups    {len(groups)} language groups from source.units")
    actual = [f for f in found if f["code"] == urule]
    other = collections.Counter(f["code"] for f in found if f["code"] != urule)
    divs = []
    dv = os.path.join(pack, "divergences.json")
    if os.path.exists(dv):
        divs = [d for d in json.load(open(dv)) if d.get("rule") in (args.rule_id, urule)]
    errp = set().union(*(err_paths(r) for r in runs))
    tp, fp, fn, explained = compare(expected, actual, divs, lambda x: (x["path"], x["line"]))
    report(tp, fp, fn, explained, args.show, errp)
    if args.column or match == "line+column":
        cps = column_points(actual, fixtures)
        ctp, cfp, cfn, _ = compare(expected, cps, divs, lambda x: (x["path"], x["line"], x.get("column")))
        print(f"columns   {ctp} TP / {len(cfp)} FP / {len(cfn)} FN at (path, line, column)"
              f"{'  (the conformance runner matches line+column)' if match == 'line+column' else ''}")
    if other:
        print(f"other     codes not scored: {dict(other)}")
    if errp:
        print(f"parse     {len(errp)} files have tree-sitter ERROR/MISSING nodes (the engine withholds such files; "
              f"rrlab scores them; marked * above)")
    for w in dict.fromkeys(w for r in runs for w in r.prog.warnings):
        print(f"warning   {w}")
    for src, at, st in dict.fromkeys(x for r in runs for x in r.prog.pattern_report):
        print(f"pattern   `{src}`{' at ' + at if at else ''}: {st}")
    rounds = max(r.rounds for r in runs)
    print(f"time      facts {sum(r.facts_cpu for r in runs):.1f}s + souffle {sum(r.souffle_cpu for r in runs):.1f}s CPU "
          f"({rounds} round{'s' if rounds != 1 else ''}), {time.time() - t0:.1f}s wall; work dir {runs[0].work}")


def option_dict(o):
    """Options as one object (an ESLint-style array's strings become option, option2, …)."""
    if isinstance(o, list):
        out, k = {}, 0
        for x in o:
            if isinstance(x, dict):
                out.update(x)
            else:
                out["option" if k == 0 else f"option{k + 1}"] = x; k += 1
        return out
    return dict(o or {})


def unit_options(unit):
    lo = (unit or {}).get("languageOptions") or {}
    out = {}
    if "sourceType" in lo:
        out["sourceType"] = lo["sourceType"]
    feats = (lo.get("parserOptions") or {}).get("ecmaFeatures") or {}
    for k in ("globalReturn", "impliedStrict"):
        if feats.get(k):
            out[k] = True
    return out


def err_paths(run):
    path = {r[0]: r[1] for r in run.table("doc")}
    return {path[r[0]] for r in run.table("syntax_error")}


def compare(expected, actual, divs, key):
    pending = collections.defaultdict(list)
    for a in actual:
        pending[key(a)].append(a)
    tp, missed = 0, []
    for e in expected:
        b = pending.get(key(e))
        if b:
            b.pop(); tp += 1
        else:
            missed.append(e)
    extra = [a for b in pending.values() for a in b]
    explained = []

    def unexplained(side, pts):
        keep = []
        for x in pts:
            d = next((d for d in divs if d.get("path") == x["path"] and d.get("line") == x["line"]
                      and (side == "fn" or d.get("kind") != "rerail-unknown")), None)
            if d:
                explained.append((side, x, d))
            else:
                keep.append(x)
        return sorted(keep, key=lambda x: (x["path"], x["line"]))
    return tp, unexplained("fp", extra), unexplained("fn", missed), explained


def report(tp, fp, fn, explained, show, errp):
    lost = sum(1 for s, _, d in explained if s == "fn" and d.get("kind") != "upstream-wrong")
    prec = tp / (tp + len(fp)) if tp + len(fp) else None
    rec = tp / (tp + len(fn) + lost) if tp + len(fn) + lost else None
    f = lambda x: "n/a" if x is None else f"{x:.3f}"
    print(f"score     {tp} TP / {len(fp)} FP / {len(fn)} FN, {len(explained)} explained by divergences.json; "
          f"precision {f(prec)}, recall {f(rec)}  (path, line, code)")
    for side, pts in (("FP", fp), ("FN", fn)):
        for x in pts[:show]:
            star = "*" if x["path"] in errp else " "
            msg = f"  {x.get('message', '')}" if side == "FP" else ""
            print(f"  {side}{star} {x['path']}:{x['line']}{msg}")
        if len(pts) > show:
            print(f"  {side}  … {len(pts) - show} more (--show N)")
    for side, x, d in explained[:show]:
        print(f"  ex {side} {x['path']}:{x['line']}  {d.get('kind')}")


def column_points(found, root):
    """rrlab's 0-based byte column as the recording's 1-based code-point column."""
    out, cache = [], {}
    for f in found:
        if f["path"] not in cache:
            cache[f["path"]] = open(os.path.join(root, f["path"]), "rb").read().split(b"\n")
        lines = cache[f["path"]]
        b = lines[f["line"] - 1] if 0 < f["line"] <= len(lines) else b""
        out.append(dict(f, column=len(b[:f["col"]].decode("utf-8", "replace")) + 1))
    return out


# ---- CLI ----------------------------------------------------------------------------------------------------------

def cmd_run(args):
    root = os.path.abspath(args.root or os.path.commonpath([os.path.dirname(os.path.abspath(f)) for f in args.files]))
    files = [os.path.relpath(os.path.abspath(f), root) for f in args.files]
    run = Run(args.rule, files, root, options=json.loads(args.options) if args.options else None, lang=args.lang,
              keep=args.keep, verbose=args.verbose)
    found = run.execute()
    for f in found:
        star = " (syntax error in file)" if f["syntax_error"] else ""
        print(f"{f['path']}:{f['line']}:{f['col'] + 1}: {f['code']}: {f['message']}{star}")
    print(f"{len(found)} findings; {run.rounds} souffle round(s), {run.souffle_cpu:.1f}s CPU; work dir {run.work}",
          file=sys.stderr)
    for w in run.prog.warnings:
        print(f"warning: {w}", file=sys.stderr)
    for src, at, st in run.prog.pattern_report:
        print(f"pattern `{src}`: {st}", file=sys.stderr)


def cmd_lower(args):
    ensure_built()
    prog = L.Program(args.rule, P.Compiler(FACTS_BIN))
    sys.stdout.write(prog.text())
    for w in prog.warnings:
        print(f"warning: {w}", file=sys.stderr)


def cmd_pattern(args):
    ensure_built()

    class Stub:
        n, extra, pattern_report = 0, [], []
    s = Stub()
    name, vs = P.Compiler(FACTS_BIN)(s, args.lang, "`" + args.pattern + "`", args.kind)
    print("\n".join(s.extra))
    print(f"// binds: {', '.join(vs) or '(nothing)'}; {s.pattern_report[-1][2]}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = ap.add_subparsers(dest="cmd", required=True)
    r = sp.add_parser("run", help="run a rule on files")
    r.add_argument("rule"); r.add_argument("files", nargs="+")
    r.add_argument("--options"); r.add_argument("--lang"); r.add_argument("--root"); r.add_argument("--keep")
    r.add_argument("-v", "--verbose", action="store_true")
    s = sp.add_parser("score", help="score a rule against its pack's upstream recording")
    s.add_argument("pack"); s.add_argument("rule_id"); s.add_argument("rule", nargs="?")
    s.add_argument("--all", action="store_true", help="every fixture of the recording, not only the rule's own")
    s.add_argument("--options"); s.add_argument("--data"); s.add_argument("--keep")
    s.add_argument("--show", type=int, default=25); s.add_argument("--column", action="store_true")
    lw = sp.add_parser("lower", help="print the Soufflé program"); lw.add_argument("rule")
    pt = sp.add_parser("pattern", help="compile a backtick pattern")
    pt.add_argument("lang"); pt.add_argument("pattern"); pt.add_argument("kind", nargs="?")
    args = ap.parse_args()
    try:
        {"run": cmd_run, "score": score, "lower": cmd_lower, "pattern": cmd_pattern}[args.cmd](args)
    except L.LowerError as e:
        raise SystemExit(f"rrlab: {e}")


if __name__ == "__main__":
    main()
