#!/usr/bin/env python3
"""rrlab: run a .rr rule on real files through plain Soufflé, and score it against an upstream recording.

  rrlab.py run RULE.rr FILE... [--options JSON] [--lang LANG] [--root DIR] [-v]
  rrlab.py score PACK RULE-ID [RULE.rr] [--all] [--options JSON] [--data DIR] [--show N] [--column]
  rrlab.py dl CHECK [RULE.rr] [--entry NAME] [--test TEXT]   score a data-layer rule on the port's events (dl.py)
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
    """Build rrlab-facts, the regex crate and the functor library when their sources are newer. Several rrlab
    processes may run at once: builds take a lock, and the functor library is installed by rename, so a running
    souffle keeps the file it loaded."""
    import fcntl
    os.makedirs(os.path.join(HERE, "target"), exist_ok=True)
    with open(os.path.join(HERE, "target", ".build-lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        newest = lambda paths: max(os.path.getmtime(p) for p in paths if os.path.exists(p))
        src = [os.path.join(HERE, p) for p in ("src/main.rs", "build.rs", "Cargo.toml")]
        if not os.path.exists(FACTS_BIN) or newest(src) > os.path.getmtime(FACTS_BIN):
            log("rrlab: building rrlab-facts (cargo build --release --offline) …")
            subprocess.run(["cargo", "build", "--release", "--offline", "--quiet"], cwd=HERE, check=True)
        rx_dir = os.path.join(HERE, "rx")
        rx_lib = os.path.join(rx_dir, "target", "release", "librrlab_rx.a")
        rx_src = [os.path.join(rx_dir, p) for p in ("src/lib.rs", "Cargo.toml")]
        if not os.path.exists(rx_lib) or newest(rx_src) > os.path.getmtime(rx_lib):
            log("rrlab: building the regex functors (cargo build --release --offline in rx/) …")
            subprocess.run(["cargo", "build", "--release", "--offline", "--quiet"], cwd=rx_dir, check=True)
        lib = os.path.join(FN_DIR, "librrlabfn.dylib" if sys.platform == "darwin" else "librrlabfn.so")
        cpp = os.path.join(HERE, "functors", "functors.cpp")
        if not os.path.exists(lib) or newest([cpp, rx_lib]) > os.path.getmtime(lib):
            os.makedirs(FN_DIR, exist_ok=True)
            tmp = lib + f".{os.getpid()}.tmp"
            # the whole archive: nothing in functors.cpp calls the Rust functors, souffle looks them up by name
            whole = (["-Wl,-force_load," + rx_lib, "-framework", "CoreFoundation"] if sys.platform == "darwin"
                     else ["-Wl,--whole-archive", rx_lib, "-Wl,--no-whole-archive", "-lpthread", "-ldl"])
            # libsodium, when installed, gives a real @ed25519_ok (static, so souffle needs no library path)
            sodium = next((d for d in ("/opt/homebrew/opt/libsodium", "/usr/local/opt/libsodium", "/usr")
                           if os.path.exists(os.path.join(d, "include", "sodium.h"))
                           and os.path.exists(os.path.join(d, "lib", "libsodium.a"))), None)
            na = (["-DRRLAB_SODIUM", "-I" + os.path.join(sodium, "include"), os.path.join(sodium, "lib", "libsodium.a")]
                  if sodium else [])
            subprocess.run([os.environ.get("CXX", "c++"), "-std=c++17", "-O2", "-shared", "-fPIC", "-o", tmp, cpp]
                           + na + whole, check=True)
            os.replace(tmp, lib)
            if sys.platform == "darwin":  # souffle looks for lib<name>.so on some builds
                alt = os.path.join(FN_DIR, "librrlabfn.so")
                if not os.path.lexists(alt):
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


MAX_WALK = 200_000


def shallow(root):
    """A root too near / to walk: fewer than 3 path components (/, /Users, /private/tmp)."""
    return len([p for p in os.path.abspath(root).split(os.sep) if p]) < 3


def repo_inputs(root, docs):
    """file, package, package_field, package_for from the files under root (at most MAX_WALK files)."""
    if shallow(root):
        raise SystemExit(f"rrlab: refusing to walk {root}: pass a --root at least 3 levels deep")
    files, pkgs = [], {}
    for d, dirs, fs in os.walk(root):
        dirs[:] = [x for x in dirs if x not in SKIP_DIRS]
        if len(files) > MAX_WALK:
            log(f"rrlab: {root} has over {MAX_WALK} files; file() facts are truncated")
            break
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

_RUNS = []


_PY_STR = re.compile(r"(?s)([A-Za-z]*)('\'\'|\"\"\"|'|\")(.*)\2")
_PY_ESC = re.compile(r"\\(N\{[^}]*\}|x[0-9a-fA-F]{2}|u[0-9a-fA-F]{4}|U[0-9a-fA-F]{8}|[0-7]{1,3}|\r\n|[\s\S])")
_PY_ESC1 = {"n": "\n", "t": "\t", "r": "\r", "a": "\a", "b": "\b", "f": "\f", "v": "\v", "\\": "\\", "'": "'",
            '"': '"', "\n": "", "\r\n": ""}


def py_string(lit):
    """A Python string literal's (value, prefix, plain, prefix length, quote length): plain when the value is the
    text between the quotes as written. None for bytes and f-strings (no value Ruff parses)."""
    m = _PY_STR.fullmatch(lit)
    if not m:
        return None
    pre, q, body = m.groups()
    if set(pre.lower()) & {"b", "f", "t"}:
        return None
    if "r" in pre.lower() or "\\" not in body:
        return body, pre, True, len(pre), len(q)

    def esc(e):
        t = e.group(1)
        if t in _PY_ESC1:
            return _PY_ESC1[t]
        if t[0] in "xuU":
            return chr(int(t[1:], 16))
        if t[0] == "N":
            import unicodedata
            try:
                return unicodedata.lookup(t[2:-1])
            except KeyError:
                return e.group(0)
        if t[0] in "01234567":
            return chr(int(t, 8))
        return e.group(0)
    return _PY_ESC.sub(esc, body), pre, False, len(pre), len(q)


def scope_glob(g):
    """A .scope glob as a regex over a root-relative path: `**/` any directories (none too), `**` anything, `*` and
    `?` within one segment, `{a,b}` either."""
    out, i = [], 0
    while i < len(g):
        c = g[i]
        if g.startswith("**/", i):
            out.append("(?:.*/)?"); i += 3; continue
        if g.startswith("**", i):
            out.append(".*"); i += 2; continue
        if c == "*":
            out.append("[^/]*")
        elif c == "?":
            out.append("[^/]")
        elif c == "{":
            j = g.find("}", i)
            if j > i:
                out.append("(?:" + "|".join(re.escape(x) for x in g[i + 1:j].split(",")) + ")"); i = j + 1; continue
            out.append(re.escape(c))
        else:
            out.append(re.escape(c))
        i += 1
    return re.compile("".join(out), re.S)


_RX = None


def rx_lib():
    """rr_find_all from the functor library (the Rust regex crate), through ctypes."""
    global _RX
    if _RX is None:
        import ctypes
        lib = ctypes.CDLL(os.path.join(FN_DIR, "librrlabfn.dylib" if sys.platform == "darwin" else "librrlabfn.so"))
        f = lib.rr_find_all
        f.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_int]
        f.restype = ctypes.c_char_p
        _RX = f
    return _RX


def cleanup():
    """Remove the work directories of this process's runs that --keep did not name."""
    for r in _RUNS:
        if not r.keep:
            shutil.rmtree(r.work, ignore_errors=True)


class Run:
    def __init__(self, rule, files, root, options=None, lang=None, keep=None, verbose=False):
        ensure_built()
        self.rule, self.files, self.root, self.verbose = rule, files, root, verbose
        self.prog = L.Program(rule, P.Compiler(FACTS_BIN))
        # one directory per process (writers run rrlab side by side), removed after the run unless --keep names one
        self.keep = bool(keep)
        self.work = keep or os.path.join(HERE, "target", "work",
                                         f"{os.path.splitext(os.path.basename(rule))[0]}-{os.getpid()}-{len(_RUNS)}")
        _RUNS.append(self)
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
        extra["directive_global"] = self.directive_globals()
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
                if name in used and name not in L.FILLED and not name.endswith("__prev") and not name.startswith("rr_mres") and \
                        not any(w.startswith(name + " ") for w in p.warnings):
                    p.warn(f"input {name} has no rows in rrlab")

    def in_scope(self, path):
        """The rule's .scope: some include glob matches (when it names any), no exclude glob does, and no `test` or
        `tests` path part under `tests false`."""
        p = self.prog
        if not getattr(self, "use_scope", True):
            return True
        if not hasattr(self, "_scope"):
            self._scope = ([scope_glob(g) for g in p.scope], [scope_glob(g) for g in p.scope_exclude])
        inc, exc = self._scope
        path = path[2:] if path.startswith("./") else path
        if inc and not any(r.fullmatch(path) for r in inc):
            return False
        if any(r.fullmatch(path) for r in exc):
            return False
        if not p.scope_tests and {"test", "tests"} & set(path.split("/")):
            return False
        return True

    def directive_globals(self):
        """directive_global(d, x): the names a `/* global a, b: writable */` (or `globals`) block comment declares,
        as ESLint reads them: items split at commas and spaces, `name:value`, a ` -- note` dropped; `x: off`
        declares nothing (eslint-b-scope's global_off reads those from the comment itself)."""
        texts = {r[0]: r[1] for r in self.table("text")}
        docof = {r[0]: r[1] for r in self.table("docof")}
        rows = []
        for cid, body in self.table("comment"):
            if not texts.get(cid, "").startswith("/*"):
                continue
            m = re.match(r"\s*globals?(?:\s+(.*))?$", body, re.S)
            if not m or cid not in docof:
                continue
            items = re.split(r"\s+--\s", m.group(1) or "", maxsplit=1)[0]
            items = re.sub(r"\s*:\s*", ":", re.sub(r"\s*,\s*", ",", items.strip()))
            for it in re.split(r"[\s,]+", items):
                name, _, val = it.partition(":")
                if name and val.lower() != "off":
                    rows.append((docof[cid], name))
        return sorted(set(rows))

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
        # a regex the Rust regex crate refuses (look-around, backreferences) matches nothing: say so once
        for l in r.stderr.splitlines():
            if l.startswith("rrlab: bad regex") and l[7:] not in self.prog.warnings:
                self.prog.warn(l[7:])
        return r

    def execute(self, max_rounds=60):
        t0 = time.time()
        c0 = cpu_children()
        rounds = 0
        prev = None
        while True:
            self.souffle()
            rounds += 1
            served = self.serve_matches()
            served = self.serve_parsed() or served
            served = self.serve_dfs() or served
            if not self.prog.ordered and not served:
                break
            cur = {o: open(os.path.join(self.out, f"{o}.csv")).read() for o in self.prog.ordered}
            if cur == prev and not served:
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

    # -- dfs_first / dfs_first_memo: souffle writes edge, target (and roots); rrlab searches and answers rr_dfs{k}
    def serve_dfs(self):
        """core.rr's dfs_first(r, hit, first, edge, target): from r, a node's edges are tried in increasing (ord,
        dst), each node is entered once and a node on the current path is skipped; the first edge that reaches a
        target gives hit, and first is the ord of r's edge that starts the path. r is never its own hit.
        dfs_first_memo searches the roots in increasing order of their second column with one shared memo, and a
        node finished while a cycle was open is remembered as reaching nothing. True if the answers changed."""
        p = self.prog
        if not p.dfs:
            return False
        if not hasattr(self, "_dserved"):
            self._dserved = {}
        def rows(rel):
            f = os.path.join(self.out, f"{rel}.csv")
            if not os.path.exists(f):
                return []
            with open(f, newline="") as fh:
                return [r for r in csv.reader(fh, delimiter="\t", quotechar='"') if r]
        num = lambda v: int(v) if re.fullmatch(r"-?\d+", v) else v
        changed = False
        for k, (name, edge, target, *roots) in p.dfs.items():
            out = collections.defaultdict(list)
            for src, dst, o in rows(edge):
                out[src].append((num(o), num(dst), dst))
            for v in out.values():
                v.sort(key=lambda t: (t[0], t[1]))
            tgt = {r[0] for r in rows(target)}
            ans = []
            if name == "dfs_first":
                for r in out:
                    seen = {r}
                    stack = [(r, iter(out[r]), None)]
                    hit = None
                    while stack and hit is None:
                        n, it, first = stack[-1]
                        nxt = next(it, None)
                        if nxt is None:
                            stack.pop(); continue
                        o, _, d = nxt
                        f0 = o if n == r else first
                        if d in tgt and d != r:
                            hit = (d, f0); break
                        if d not in seen:
                            seen.add(d)
                            stack.append((d, iter(out.get(d, [])), f0))
                    if hit:
                        ans.append((r, hit[0], hit[1]))
            else:
                order = sorted(rows(roots[0]), key=lambda t: (num(t[1]) if len(t) > 1 else 0, num(t[0])))
                memo = {}
                sys.setrecursionlimit(max(10000, sys.getrecursionlimit()))
                def search(n, path):
                    # (hit, cycle_open): the first target below n, and whether a node on the path was met
                    if n in memo:
                        return memo[n], False
                    if n in path:
                        return None, True
                    path.add(n)
                    res, cyc = None, False
                    for o, _, d in out.get(n, []):
                        if d in tgt and d not in path:
                            res = d; break
                        sub, c = search(d, path)
                        cyc = cyc or c
                        if sub is not None:
                            res = sub; break
                    path.discard(n)
                    memo[n] = res if (res is not None or not cyc) else None
                    return res, cyc
                for row in order:
                    r = row[0]
                    path = {r}
                    for o, _, d in out.get(r, []):
                        if d in tgt and d != r:
                            ans.append((r, d, o)); break
                        sub, _ = search(d, path)
                        if sub is not None:
                            ans.append((r, sub, o)); break
            text = tsv(sorted(set(ans)))
            if self._dserved.get(k) != text:
                self._dserved[k] = text
                open(os.path.join(self.inp, f"rr_dfs{k}.facts"), "w").write(text)
                changed = True
        return changed

    # -- match_all, and match captures that are spans: souffle asks (rr_mreq{k}), rrlab answers (rr_mres{k})
    def serve_matches(self):
        """Answer the match requests of the last round. Spans are new nodes with text, line, end_line, col and
        docof rows; True if any rows were added (the program must run again)."""
        p = self.prog
        if not p.matches:
            return False
        self.node_cache()
        if not hasattr(self, "_mserved"):
            self._mserved = set()
            self._spans = {}
            self._fn = rx_lib()
        add = collections.defaultdict(list)
        for k, m in p.matches.items():
            f = os.path.join(self.out, f"rr_mreq{k}.csv")
            if not os.path.exists(f):
                continue
            with open(f, newline="") as fh:
                subjects = [r[0].replace('\\"', '"') for r in csv.reader(fh, delimiter="\t", quotechar='"') if r]
            for subj in subjects:
                if (k, subj) in self._mserved:
                    continue
                self._mserved.add((k, subj))
                text = self._ntext.get(subj, "") if m["node"] else subj
                b = text.encode("utf-8", "surrogateescape")
                raw = self._fn(m["regex"].encode(), b.replace(b"\0", b""), 0 if m["all"] else 1)
                for n, ln in enumerate((raw or b"").decode().splitlines()):
                    offs = [int(x) for x in ln.split()]
                    row = [subj, n]
                    for g, gi, sp in zip(m["groups"], m["index"], m["span"]):
                        st, en = offs[2 * gi], offs[2 * gi + 1]
                        if st < 0:
                            row = None if sp else row + [""]
                            if row is None:
                                break
                            continue
                        row.append(self.span(subj, b, st, en, add) if sp else b[st:en].decode("utf-8", "replace"))
                    if row is not None:
                        add[f"rr_mres{k}"].append(row)
        for rel, rows in add.items():
            with open(os.path.join(self.inp, f"{rel}.facts"), "a") as fh:
                fh.write(tsv(rows))
        return bool(add)

    def node_cache(self):
        """text, line, col and docof of every node (spans and grafts added as they are made), and the next free id."""
        if hasattr(self, "_ntext"):
            return
        self._ntext = {r[0]: r[1] for r in self.table("text")}
        self._nline = {r[0]: int(r[1]) for r in self.table("line")}
        self._ncol = {r[0]: int(r[1]) for r in self.table("col")}
        self._ndoc = {r[0]: r[1] for r in self.table("docof")}
        self._next_id = max((int(i) for i in self._ndoc if i.lstrip("-").isdigit()), default=0) + 1

    def serve_parsed(self):
        """Answer rr_preq(s, g): parse each Python string s the rule asks about (grammar "annotation") and graft its
        tree, ids above every other, with parsed(s, g, root). A plain string's tree has its own place in the file;
        an escaped or concatenated one is parsed from its value, every node at the string. Bytes and f-strings
        give no tree, as Ruff refuses them."""
        if not self.prog.parse_requests:
            return False
        f = os.path.join(self.out, "rr_preq.csv")
        if not os.path.exists(f):
            return False
        self.node_cache()
        if not hasattr(self, "_pserved"):
            self._pserved = set()
            self._nodes = {r[1]: (r[0], r[2], int(r[3]), int(r[4])) for r in self.table("node")}
            self._kids = collections.defaultdict(list)
            for r in self.table("child"):
                self._kids[r[0]].append((int(r[2]), r[1]))
            self._paths = {r[0]: r[1] for r in self.table("doc")}
            self._visit = max((int(r[1]) for r in self.table("visit_index")), default=0) + 1
        with open(f, newline="") as fh:
            reqs = [tuple(c.replace('\\"', '"') for c in r) for r in csv.reader(fh, delimiter="\t", quotechar='"') if r]
        spec = []
        for s_id, g in reqs:
            if (s_id, g) in self._pserved:
                continue
            self._pserved.add((s_id, g))
            if g != "annotation" or s_id not in self._nodes:
                continue
            d, kind, a, b = self._nodes[s_id]
            parts = [s_id] if kind == "string" else \
                [c for _, c in sorted(self._kids[s_id]) if self._nodes.get(c, ("", ""))[1] == "string"] \
                if kind == "concatenated_string" else []
            lits = [py_string(self._ntext.get(c, "")) for c in parts]
            if not lits or any(x is None for x in lits):
                continue
            one = {"d": int(d), "host": int(s_id), "grammar": g, "lang": "python"}
            if len(lits) == 1 and lits[0][2]:
                pre, q = lits[0][3], lits[0][4]
                one.update(path=self._paths[d], lo=a + pre + q, hi=b - q)
            else:
                one.update(value="".join(x[0] for x in lits), line=self._nline.get(s_id, 1),
                           col=self._ncol.get(s_id, 0), a=a, b=b)
            spec.append(json.dumps(one))
        if not spec:
            return False
        tmp = os.path.join(self.work, "graft")
        shutil.rmtree(tmp, ignore_errors=True)
        r = subprocess.run([FACTS_BIN, "graft", tmp, "--root", self.root, "--next-id", str(self._next_id),
                            "--visit", str(self._visit)], input="\n".join(spec) + "\n", capture_output=True, text=True)
        if r.returncode:
            raise SystemExit(f"rrlab-facts graft failed:\n{r.stderr[-2000:]}")
        self._next_id = int(r.stdout.strip() or self._next_id) + 1
        added = False
        for fn in os.listdir(tmp):
            body = open(os.path.join(tmp, fn), newline="").read()
            if not body:
                continue
            added = True
            with open(os.path.join(self.inp, fn), "a", newline="") as fh:
                fh.write(body)
            rel = fn[:-6]
            if rel in ("text", "line", "col", "docof", "visit_index"):
                with open(os.path.join(tmp, fn), newline="") as fh:
                    for row in csv.reader(fh, delimiter="\t", quotechar='"'):
                        if rel == "text":
                            self._ntext[row[0]] = row[1]
                        elif rel == "line":
                            self._nline[row[0]] = int(row[1])
                        elif rel == "col":
                            self._ncol[row[0]] = int(row[1])
                        elif rel == "docof":
                            self._ndoc[row[0]] = row[1]
                        else:
                            self._visit = max(self._visit, int(row[1]) + 1)
        shutil.rmtree(tmp, ignore_errors=True)
        return added

    def span(self, node, b, st, en, add):
        """The id of the span b[st:en] of node's text, a node of its own (made once)."""
        key = (node, st, en)
        if key in self._spans:
            return self._spans[key]
        i = str(self._next_id); self._next_id += 1
        self._spans[key] = i
        def at(o):
            pre = b[:o].replace(b"\r\n", b"\n").replace(b"\r", b"\n")
            nl = pre.count(b"\n")
            last = pre.rfind(b"\n")
            return self._nline.get(node, 1) + nl, (self._ncol.get(node, 0) + len(pre) if nl == 0 else len(pre) - last - 1)
        (l0, c0), (l1, _) = at(st), at(en)
        d = self._ndoc.get(node)
        t = b[st:en].decode("utf-8", "replace")
        add["text"].append([i, t]); add["line"].append([i, l0]); add["end_line"].append([i, l1])
        add["col"].append([i, c0])
        if d is not None:
            add["docof"].append([i, d])
        self._ntext[i], self._nline[i], self._ncol[i] = t, l0, c0
        if d is not None:
            self._ndoc[i] = d
        return i

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
                # No `code` clause: the code is the violation's own name, as in the engine.
                code = (meta["code"] if meta["code"] is not None
                        else r[meta["cols"].index(meta["code_col"])] if meta["code_col"] is not None else v)
                out.append({"path": pth, "line": ln, "col": c, "code": code, "violation": v, "row": r,
                            "message": self.message(meta, r, line), "syntax_error": d in errs, "at": at})
        if p.on_syntax:
            # .on syntax_error: the document is withheld (its other findings go) and the error is the finding
            code, at1 = p.on_syntax
            out = [f for f in out if not f["syntax_error"]]
            for r in self.table("syntax_error"):
                if r[0] in path:
                    out.append({"path": path[r[0]], "line": 1 if at1 else int(r[1]), "col": 0, "code": code,
                                "violation": None, "row": r, "message": r[2] if len(r) > 2 else "syntax error",
                                "syntax_error": True, "at": None})
        out = [f for f in out if self.in_scope(f["path"])]
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
        """Each tool's own suppression comments, applied only to findings whose violation names that tool:
          eslint, oxlint   eslint-disable[-line|-next-line] / eslint-enable (oxlint also reads oxlint-…), rule lists
          ruff, flake8     `# noqa[: codes]` anywhere in a comment, on the finding's line or, inside a multi-line
                           string or a backslash continuation, on its last line; `# ruff: noqa[: codes]` and
                           `# flake8: noqa[: codes]` for the whole file
          sqlfluff         `-- noqa[: codes]` on the line; `-- noqa: disable=codes|all` … `enable=…`
          clippy           #[allow(clippy::x)] / #[expect(…)] on an item covers the item and its attributes;
                           #![allow(…)] its enclosing item or the file
          nosec            Bandit's `# nosec`, as nosec.rs ports it (below)"""
        p = self.prog
        tools = {v: meta["suppress"] for v, meta in p.violations.items()}
        if not any(tools.values()):
            return found
        used = {t for v in tools.values() for t, _ in v}
        line = {r[0]: int(r[1]) for r in self.table("line")}
        end = {r[0]: int(r[1]) for r in self.table("end_line")}
        docof = {r[0]: r[1] for r in self.table("docof")}
        path = {r[0]: r[1] for r in self.table("doc")}
        directives = collections.defaultdict(list)   # path -> [(tool, kind, line, end, codes)]
        file_noqa = collections.defaultdict(list)    # path -> [codes] (empty set: every code)
        for nid, body in self.table("comment"):
            if nid not in docof:
                continue
            pth, a, b = path[docof[nid]], line[nid], end[nid]
            text = body.strip()
            m = re.match(r"(eslint|oxlint)-(disable-next-line|disable-line|disable|enable)(?:\s+(.*))?$", text, re.S)
            if m and used & {"eslint", "oxlint"}:
                rules = (m.group(3) or "").split("--")[0]
                names = {x.strip() for x in rules.replace("\n", " ").split(",") if x.strip()}
                who = {"eslint", "oxlint"} if m.group(1) == "eslint" else {"oxlint"}
                directives[pth].append((who, m.group(2), a, b, names))
            if used & {"ruff", "flake8"}:
                m = re.match(r"(?i)(?:ruff|flake8)\s*:\s*noqa(?:\s*:\s*(?P<codes>.*))?$", text)
                if m:
                    file_noqa[pth].append(set(re.findall(r"[A-Z]+[0-9]+", m.group("codes") or "")))
                else:
                    m = re.search(r"(?i)#\s*noqa(?::\s*(?P<codes>[A-Z]+[0-9]+(?:[\s,]+[A-Z]+[0-9]+)*))?", "#" + body)
                    if m:
                        codes = set(re.findall(r"[A-Z]+[0-9]+", m.group("codes") or ""))
                        directives[pth].append(({"ruff", "flake8"}, "noqa", a, b, codes))
            if "sqlfluff" in used:
                m = re.match(r"(?i)noqa(?:\s*:\s*(?P<rest>.*))?$", text)
                if m:
                    rest = (m.group("rest") or "").strip()
                    r2 = re.match(r"(disable|enable)\s*=\s*(.*)", rest)
                    kind = r2.group(1) if r2 else "noqa"
                    codes = {x.strip() for x in (r2.group(2) if r2 else rest).split(",") if x.strip()}
                    if "all" in codes:
                        codes = set()
                    directives[pth].append(({"sqlfluff"}, kind, a, b, codes))
        noqa_line = self.noqa_lines() if used & {"ruff", "flake8"} else {}
        clippy = self.clippy_scopes() if "clippy" in used else {}
        nosec = nosec_lines(self, path, docof, line) if "nosec" in used else {}
        keep = []
        for f in found:
            mine = tools[f["violation"]]
            names = lambda tool_set: {r for t, r in mine if t in tool_set}
            codes = names({"nosec"})
            if codes and nosec_suppresses(nosec.get(f["path"], {}), codes, f["line"], line.get(f.get("at")), end.get(f.get("at"))):
                continue
            rn = names({"ruff", "flake8"})
            if rn and any(not c or c & rn for c in file_noqa.get(f["path"], [])):
                continue
            cn = names({"clippy"})
            # suppress clippy names lints as written ("clippy::x", "clippy::pedantic", "warnings"); attributes
            # name them with or without the clippy:: path
            cn = {re.sub(r"^clippy\s*::\s*", "", c) for c in cn}
            if cn and any(a <= f["line"] <= b and lint in cn for a, b, lint in clippy.get(f["path"], [])):
                continue
            target = noqa_line.get(f["path"], {}).get(f["line"], f["line"])
            off = state = sql_off = False
            for who, kind, a, b, rules in sorted(directives.get(f["path"], []), key=lambda x: x[2]):
                own = names(who)
                if not own or (rules and not rules & own):
                    continue
                if kind == "disable-line" and a == f["line"]:
                    off = True
                elif kind == "disable-next-line" and b + 1 == f["line"]:
                    off = True
                elif kind == "disable" and a <= f["line"]:
                    if "sqlfluff" in who:
                        sql_off = True
                    else:
                        state = True
                elif kind == "enable" and a <= f["line"]:
                    if "sqlfluff" in who:
                        sql_off = False
                    else:
                        state = False
                elif kind == "noqa" and "sqlfluff" in who and a == f["line"]:
                    off = True
                elif kind == "noqa" and a == target:
                    off = True
            if not (off or state or sql_off):
                keep.append(f)
        return keep

    def noqa_lines(self):
        """path -> {line: the line its `# noqa` sits on}: every line of a multi-line string, and every line of a
        backslash continuation, maps to the last, as Ruff's noqa line mapping does."""
        line = {r[0]: int(r[1]) for r in self.table("line")}
        end = {r[0]: int(r[1]) for r in self.table("end_line")}
        docof = {r[0]: r[1] for r in self.table("docof")}
        path = {r[0]: r[1] for r in self.table("doc")}
        out = collections.defaultdict(dict)
        for nid, k in self.table("kind"):
            if k == "string" and nid in docof and end.get(nid, 0) > line.get(nid, 0):
                m = out[path[docof[nid]]]
                for l in range(line[nid], end[nid] + 1):
                    m[l] = max(m.get(l, l), end[nid])
        cont = collections.defaultdict(list)
        for d, l, sp in self.table("src_line"):
            cont[d].append((int(l), sp))
        texts = None
        for d, rows in cont.items():
            if texts is None:
                ids = {sp for rs in cont.values() for _, sp in rs}
                texts = {r[0]: r[1] for r in self.table("text") if r[0] in ids}
            m = out[path[d]]
            rows.sort()
            run = []
            for l, sp in rows:
                run.append(l)
                if not texts.get(sp, "").endswith("\\"):
                    if len(run) > 1:
                        for x in run:
                            m[x] = max(m.get(x, x), run[-1])
                    run = []
        return out

    def clippy_scopes(self):
        """path -> [(first line, last line, lint)] for each clippy lint an allow/expect attribute names."""
        kind = {r[0]: r[1] for r in self.table("kind")}
        rel = [nid for nid, k in kind.items() if k in ("attribute_item", "inner_attribute_item")]
        if not rel:
            return {}
        text = {r[0]: r[1] for r in self.table("text") if r[0] in set(rel)}
        line = {r[0]: int(r[1]) for r in self.table("line")}
        end = {r[0]: int(r[1]) for r in self.table("end_line")}
        docof = {r[0]: r[1] for r in self.table("docof")}
        path = {r[0]: r[1] for r in self.table("doc")}
        parent = {r[0]: r[1] for r in self.table("rrlab_parent")}
        nxt = {r[0]: r[1] for r in self.table("next_sibling")}
        out = collections.defaultdict(list)
        for nid in rel:
            m = re.search(r"\b(?:allow|expect)\s*\((.*)\)", text.get(nid, ""), re.S)
            if not m:
                continue
            # every lint path the list names (reason = "…" aside), clippy:: dropped: clippy::x, a group, `warnings`
            args = re.sub(r'\breason\s*=\s*"(?:[^"\\]|\\.)*"', "", m.group(1))
            lints = [re.sub(r"^clippy::", "", re.sub(r"\s+", "", x)) for x in re.findall(r"[A-Za-z_][\w\s:]*", args)]
            if kind[nid] == "inner_attribute_item":
                scope = parent.get(nid)
                a, b = (line.get(scope, 1), end.get(scope, 10 ** 9)) if scope else (1, 10 ** 9)
            else:
                item = nxt.get(nid)
                while item and kind.get(item) in ("attribute_item", "line_comment", "block_comment"):
                    item = nxt.get(item)
                if not item:
                    continue
                a, b = line[nid], end[item]
            for lint in lints:
                out[path[docof[nid]]].append((a, b, lint))
        return out


# Bandit 1.9.4's `# nosec`, as native/src/code_rule_dsl/suppress/nosec.rs ports it: a comment's test list (ids or
# names; none known is a blanket nosec), read on the reported line and the first `# nosec` among the node's lines.
NOSEC_COMMENT = re.compile(r"#\s*nosec:?\s*(?P<tests>[^#]+)?#?")
NOSEC_TESTS = re.compile(r"(?i)(?:(B\d+|[a-z\d_]+),?)+")
_NOSEC_NAMES = None


def nosec_names():
    global _NOSEC_NAMES
    if _NOSEC_NAMES is None:
        src = os.path.join(REPO, "native", "src", "code_rule_dsl", "suppress", "nosec.rs")
        pairs = re.findall(r'\("(B\d+)", "([A-Za-z_\d]+)"\)', open(src).read()) if os.path.exists(src) else []
        _NOSEC_NAMES = {"B001": "B001", **{i: i for i, _ in pairs}}
        for i, n in pairs:
            _NOSEC_NAMES.setdefault(n, i)
    return _NOSEC_NAMES


def nosec_parse(comment):
    m = NOSEC_COMMENT.search(comment)
    if not m:
        return None
    ids = []
    for t in NOSEC_TESTS.finditer(m.group("tests") or ""):
        i = nosec_names().get(t.group(1))
        if i and i not in ids:
            ids.append(i)
    return ids


def nosec_lines(run, path, docof, line):
    """path -> {line: [test ids]} for every `# nosec` comment."""
    out = collections.defaultdict(dict)
    for nid, body in run.table("comment"):
        ids = nosec_parse("#" + body)
        if ids is not None and nid in docof:
            out[path[docof[nid]]].setdefault(line[nid], ids)
    return out


def nosec_suppresses(lines, codes, at_line, first, last):
    reported = lines.get(at_line)
    context = next((lines[l] for l in range(first, (last or first) + 1) if l in lines), None) if first else None
    if reported is None and context is None:
        return False
    tests = (reported or []) + (context or [])
    return not tests or any(t in codes for t in tests)


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


def ruff_snapshots(src, code, rule_id):
    """Ruff's own snapshot tests as a recording: (fixtures root, files, findings) for code. The snapshots read are
    the default-settings ones: named after the fixture (`UP006_0.py`), or after the rule's code or kebab name and
    the fixture (`F401_F401_0.py`, `assert-tuple_F631.py`); settings variants (`preview_…`, `…_py310`) are left
    out. Paths are relative to crates/ruff_linter/resources/test/fixtures."""
    import glob
    lin = src if os.path.isdir(os.path.join(src, "resources")) else os.path.join(src, "crates", "ruff_linter")
    root = os.path.join(lin, "resources", "test", "fixtures")
    if not os.path.isdir(root):
        raise SystemExit(f"--snapshots: no crates/ruff_linter/resources/test/fixtures under {src}")
    kebab = re.sub(r"^ruff-[a-z]+\d+-", "", rule_id)
    files, found = set(), []
    for snap in sorted(glob.glob(os.path.join(lin, "src", "rules", "*", "snapshots", "*.snap"))):
        linter = snap.split(os.sep)[-3]
        name = os.path.basename(snap)[:-5].split("__tests__", 1)[-1]
        if not re.fullmatch(r"[A-Za-z0-9_.-]+\.pyi?", name):
            continue
        # the name is the fixture's (UP006_0.py), or a code or kebab name, `_`, and the fixture's
        cands, ok = [], False
        for prefix, fname in [(None, name)] + [(name[:i], name[i + 1:]) for i, ch in enumerate(name) if ch == "_"]:
            if prefix not in (None, code, kebab) or prefix is None and not fname.startswith(code):
                continue
            cands = glob.glob(os.path.join(root, "**", fname), recursive=True)
            if cands:
                ok = True
                break
        if not ok:
            continue
        mine = [c for c in cands if os.path.basename(os.path.dirname(c)) == linter]
        rel = os.path.relpath((mine or cands)[0], root)
        files.add(rel)
        last = None
        for l in open(snap, encoding="utf-8", errors="replace"):
            h = re.match(r"([A-Z]+[0-9]+)\b", l)
            if h:
                last = h.group(1)
            a = re.match(r"\s*-->\s*(\S+?):(\d+):(\d+)", l) or re.match(r"(\S+?):(\d+):(\d+): ([A-Z]+[0-9]+)", l)
            if a:
                c = a.group(4) if a.re.groups == 4 else last
                if c == code:
                    found.append({"rule": code, "path": rel, "line": int(a.group(2)), "column": int(a.group(3))})
    return root, sorted(files), found


def score(args):
    if getattr(args, "snapshots", None):
        return score_snapshots(args)
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
        # several tools (python: ruff and bandit): the one the rule id starts with
        tool = tools[0] if len(tools) == 1 else next(
            (t for t in sorted(tools, key=len, reverse=True) if args.rule_id.startswith(t + "-")), None)
        upstream = {"tool": tool, "rule": next(iter(codes), args.rule_id)}
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
    allfiles = unit_files(rec)
    if args.all:
        files, how = allfiles, "every fixture"
    else:
        files, how = own_files(rec_dir, upstream["tool"], urule, allfiles, entry)
        if not files and any(f["rule"] == urule for f in rec["findings"]):
            files, how = allfiles, "every fixture: no own fixtures found for this rule"
    expected = [f for f in rec["findings"] if f["rule"] == urule and f["path"] in set(files)]
    print(f"rule      {os.path.relpath(rule, RR)}")
    print(f"upstream  {rec['tool']} {rec['version']} {urule}  ({os.path.relpath(rec_dir, data)})")
    score_files(args, rule, rec, fixtures, files, how, expected, urule, match, pack)


def score_snapshots(args):
    """score --snapshots RUFF_SRC: the rule against Ruff's snapshot tests in a Ruff checkout."""
    rule = args.rule or os.path.join(RR, "packs", args.pack, args.rule_id + ".rr")
    if not os.path.exists(rule):
        cands = [f for f in os.listdir(os.path.join(RR, "packs", args.pack)) if f.endswith(".rr") and args.rule_id in f]
        if len(cands) == 1:
            rule = os.path.join(RR, "packs", args.pack, cands[0])
    prog = L.Program(rule)
    code = next((v["code"] for v in prog.violations.values() if v["code"]), None)
    if not code:
        raise SystemExit(f"{rule}: no violation code to look up in the snapshots")
    root, files, expected = ruff_snapshots(args.snapshots, code, args.rule_id)
    print(f"rule      {os.path.relpath(rule, RR)}")
    print(f"upstream  ruff snapshots {code}  ({args.snapshots})")
    score_files(args, rule, {"source": {}}, root, files, "Ruff's default-settings snapshots", expected, code,
                "line+column", os.path.join(RR, "packs", args.pack))


def score_files(args, rule, rec, fixtures, files, how, expected, urule, match, pack):
    print(f"files     {len(files)} ({how}); {len(expected)} recorded findings")
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
        run.use_scope = not args.no_scope
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
    out_scope = [x for x in fn if runs and not runs[0].in_scope(x["path"])] if not args.no_scope else []
    if out_scope:
        print(f"scope     {len(out_scope)} of the FN are on files outside the rule's .scope (the engine never reads "
              f"them): {', '.join(sorted({x['path'] for x in out_scope})[:4])}")
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
          f"({rounds} round{'s' if rounds != 1 else ''}), {time.time() - t0:.1f}s wall"
          f"{'; work dir ' + runs[0].work if runs[0].keep else ''}")


def unit_files(rec):
    """The scored files of a recording. A unit is a file, or a sorted list of files analysed together
    (["package.json", "prefer-global/buffer/invalid-001.js"], two PHP files of a cross-file case); every member
    but a JSON manifest is scored. The manifests are read from the fixture tree like any other file."""
    out = []
    for u in rec["source"]["files"]:
        members = u if isinstance(u, list) else [u]
        out += [m for m in members if not m.endswith(".json")] or members[:1]
    return list(dict.fromkeys(out))


def _norm(s):
    s = re.sub(r"[^a-z0-9]", "", s.lower())
    for suffix in ("validator", "unittest", "test"):
        if s.endswith(suffix) and len(s) > len(suffix):
            s = s[: -len(suffix)]
    return s


def bandit_tests(rec_dir):
    """Bandit's generate.py TESTS: test id -> the examples it fires in (read as data, not run)."""
    import ast
    gen = os.path.join(rec_dir, "..", "..", "upstream", "bandit", "generate.py")
    gen = os.path.join(os.path.dirname(rec_dir), "bandit", "generate.py") if not os.path.exists(gen) else gen
    if not os.path.exists(gen):
        return {}
    for node in ast.parse(open(gen).read()).body:
        target = getattr(node, "target", None) or (node.targets[0] if isinstance(node, ast.Assign) else None)
        if isinstance(target, ast.Name) and target.id == "TESTS" and node.value is not None:
            return ast.literal_eval(node.value)
    return {}


def ruff_fixtures(rec_dir):
    """fixtures.json (code -> ruff's test fixture paths), beside the recording or in this checkout's pack, with
    ruff's fixture root stripped so paths read as the recording's."""
    rel = os.path.join("packs", "python", "upstream", "ruff", "fixtures.json")
    for f in (os.path.join(rec_dir, "fixtures.json"), os.path.join(REPO, rel)):
        if os.path.exists(f):
            rules = json.load(open(f)).get("rules", {})
            root = "crates/ruff_linter/resources/test/fixtures/"
            return {k: [p[len(root):] if p.startswith(root) else p for p in v] for k, v in rules.items()}
    return {}


def own_files(rec_dir, tool, urule, files, entry):
    """(files, description): the fixtures written for this rule. In order: the pack's own `fixtures` globs
    (conformance.json); Bandit's TESTS map plus fixtures/rerail/*/<code>*; else a path segment named after the rule:
    a directory (`curly/`, `ansi/AL04/`, `DebugDirectiveTest/` for DebugDirectiveValidator) or a file stem
    (`ASYNC100.py`, `B006_B008.py`, `BLE.py` for BLE001, `EmptyStatementUnitTest.inc.php`). Tool prefixes
    (`@next/next/`, `react/`, `n/`) are dropped."""
    import fnmatch
    if entry and entry.get("fixtures"):
        globs = entry["fixtures"]
        return [f for f in files if any(fnmatch.fnmatch(f, g) for g in globs)], "conformance.json fixtures " + ", ".join(globs)
    if tool == "bandit":
        tests = bandit_tests(rec_dir)
        if urule in tests:
            mine = set(tests[urule])
            own = [f for f in files if f in mine or re.match(rf"rerail/[^/]+/{re.escape(urule)}\b", f)
                   or re.match(rf"rerail/probes/{re.escape(urule.lower())}-", f)]
            return own, "Bandit's TESTS map (generate.py) and its rerail/ cases"
    if tool == "ruff":
        mine = ruff_fixtures(rec_dir).get(urule)
        if mine:
            own = [f for f in files if any(f == m or f.startswith(m.rstrip("/") + "/") for m in mine)
                   or re.match(rf"rerail/[^/]+/{re.escape(urule)}\b", f)]
            if own:
                return own, "ruff's fixtures.json and its rerail/ cases"
    cands = {urule, urule.split("/")[-1], urule.split(".")[-1]}
    if "/" in urule:
        cands.add(urule.split("/", 1)[1])
    norms = {_norm(c) for c in cands if _norm(c)}
    code = re.fullmatch(r"([A-Z]+)[0-9]+", urule)
    own = []
    for f in files:
        parts = f.split("/")
        dirs, stem = parts[:-1], parts[-1].split(".")[0]
        if "/" in urule and f.startswith(urule.split("/", 1)[1] + "/"):
            own.append(f); continue
        if any(_norm(d) in norms for d in dirs) or _norm(stem) in norms:
            own.append(f); continue
        if code and (urule in re.split(r"[_\W]+", stem) or stem.startswith(urule + "_") or stem == code.group(1)):
            own.append(f)
    return own, "own fixtures: " + ("a path segment named " + " or ".join(sorted(cands)))


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
            raw = open(os.path.join(root, f["path"]), "rb").read()
            # Python lines end at \r\n, \n or a lone \r (Ruff), as rrlab-facts counts them
            cache[f["path"]] = re.split(rb"\r\n|\r|\n", raw) if f["path"].endswith((".py", ".pyi")) else raw.split(b"\n")
        lines = cache[f["path"]]
        b = lines[f["line"] - 1] if 0 < f["line"] <= len(lines) else b""
        out.append(dict(f, column=len(b[:f["col"]].decode("utf-8", "replace")) + 1))
    return out


# ---- CLI ----------------------------------------------------------------------------------------------------------

def cmd_run(args):
    root = os.path.abspath(args.root or os.path.commonpath([os.path.dirname(os.path.abspath(f)) for f in args.files]))
    if shallow(root):
        raise SystemExit(f"rrlab: the files' common directory {root} is too shallow to walk for package.json and "
                         f"file facts; put them under one directory or pass --root DIR (3 or more levels deep)")
    files = [os.path.relpath(os.path.abspath(f), root) for f in args.files]
    Run.use_scope = not args.no_scope
    run = Run(args.rule, files, root, options=json.loads(args.options) if args.options else None, lang=args.lang,
              keep=args.keep, verbose=args.verbose)
    found = run.execute()
    for f in found:
        star = " (syntax error in file)" if f["syntax_error"] else ""
        print(f"{f['path']}:{f['line']}:{f['col'] + 1}: {f['code']}: {f['message']}{star}")
    print(f"{len(found)} findings; {run.rounds} souffle round(s), {run.souffle_cpu:.1f}s CPU"
          f"{'; work dir ' + run.work if run.keep else ''}",
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
    s.add_argument("--snapshots", metavar="RUFF_SRC", help="score against Ruff's snapshot tests in a Ruff checkout")
    for x in (r, s):
        x.add_argument("--no-scope", action="store_true", help="report on every file, ignoring the rule's .scope")
    dl = sp.add_parser("dl", help="score a data-layer rule against the data-layer port's recorded events")
    dl.add_argument("check"); dl.add_argument("rule", nargs="?"); dl.add_argument("--fixtures")
    dl.add_argument("--entry"); dl.add_argument("--test"); dl.add_argument("--show", type=int, default=25)
    dl.add_argument("--jobs", type=int, default=4); dl.add_argument("--keep", action="store_true")
    dl.add_argument("--codeless", help="the code for records that carry none")
    dl.add_argument("--max-files", type=int, default=500, help="skip events with more files than this")
    lw = sp.add_parser("lower", help="print the Soufflé program"); lw.add_argument("rule")
    pt = sp.add_parser("pattern", help="compile a backtick pattern")
    pt.add_argument("lang"); pt.add_argument("pattern"); pt.add_argument("kind", nargs="?")
    args = ap.parse_args()
    try:
        {"run": cmd_run, "score": score, "lower": cmd_lower, "pattern": cmd_pattern,
         "dl": lambda a: __import__("dl").score(a, Run, HERE)}[args.cmd](args)
    except L.LowerError as e:
        raise SystemExit(f"rrlab: {e}")
    finally:
        cleanup()


if __name__ == "__main__":
    main()
