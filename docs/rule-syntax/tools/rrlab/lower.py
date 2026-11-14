"""Lower a .rr rule (and everything it includes) to plain Soufflé.

What changes (README, "Lowering"):
  .violation V(…) …        .decl V(…) and .output V; code, message and suppressions are kept as metadata
  rules with `=> edits`    dropped (fixes are not applied); `may` rules dropped, `must` read as plain rules
  types                    node, span, loc and every number subtype erase to number; relation and symbol subtypes
                           to symbol (so core's `node` type no longer clashes with its `node` relation)
  earlier(a, b)            visit_index comparison in the same document (a materialised rr_earlier under `!`)
  within / inside / has / within_stop / nearest / precedes_stop
                           one generated relation per (axis, set), over the extractor's rrlab_parent edges
  text tests on nodes      text(n, t) and the test on t; every test is an @rr_* functor (functors.cpp)
  match captures           (?<X>…) binds X to the captured text (a symbol, not a span)
  join                     rows, ranks and a concatenation chain per join, "" when empty
  ordered by earlier       the self-use under `!` or an aggregate reads R__prev; rrlab.py iterates to a fixpoint
  n ~ `pattern`            patterns.py compiles it to kind/field/child/text/op constraints
"""
import collections, os, re

HERE = os.path.dirname(os.path.abspath(__file__))
RR = os.environ.get("RRLAB_RR") or os.path.dirname(os.path.dirname(HERE))  # docs/rule-syntax
LIB = os.path.join(RR, "lib")

TEXT_TESTS = {"glob", "iglob", "prefix", "iprefix", "suffix", "isuffix", "contains", "icontains", "word", "iword"}
SET_AXES = {"inside", "has", "within_stop", "nearest", "precedes_stop"}
CMP = {"=", "!=", "<", ">", "<=", ">="}
AGGS = {"count", "sum", "min", "max", "mean", "join"}
BUILTIN_FN = {"cat": "symbol", "substr": "symbol", "to_string": "symbol", "strlen": "number", "to_number": "number",
              "ord": "number", "min": "number", "max": "number", "autoinc": "number", "to_float": "float",
              "to_unsigned": "unsigned", "range": "number"}
NODEISH = {"node", "span", "loc"}
DROP_DIRECTIVES = {"policy", "description", "mode", "scope", "severity", "gate", "on", "budget", "group", "artifact",
                   "requires", "pragma", "possible", "plan", "limit", "lattice", "once", "tests"}
SOUFFLE_QUALIFIERS = {"inline", "no_inline", "magic", "no_magic", "brie", "btree", "btree_delete", "eqrel", "override"}
# Inputs rrlab fills (extractor, repository files, options); any other .input a rule uses is empty, with a warning.
TYPING = re.compile(r"typing_([a-z0-9_]+?)__([a-z0-9_]+)")
FILLED = {"directive_global", "str_part", "doc", "pkg", "path_part", "file_name", "node", "kind", "field", "child", "text", "op", "op_at", "docof",
          "line", "end_line", "col", "unset", "value", "str", "strprefix", "name", "comment", "value_type", "source",
          "src_line", "line_count", "syntax_error", "first_token", "last_token", "token_before", "token_after",
          "next_sibling", "precedes", "spread", "visit_index", "ast_depth", "rrlab_parent", "rrlab_lang",
          "static_value", "file", "package", "package_field", "package_for", "option", "option_item", "input_error",
          "parsed"}
APPROX = {"static_value": "static_value is value (no constant following)",
          "visit_index": "visit order is start order (preorder), not CPython's ast order",
          "syntax_error": "syntax_error comes from tree-sitter ERROR/MISSING nodes",
          "value": "value covers string, number and keyword literals only"}


class LowerError(Exception):
    pass


# ---- scanning ------------------------------------------------------------------------------------------------

def strip_comments(text):
    """Comments (// and /* */) blanked to spaces, keeping strings, backtick spans and line numbers."""
    out, i, n = [], 0, len(text)
    while i < n:
        ch = text[i]
        if ch == '"':
            j = i + 1
            while j < n and text[j] != '"' and text[j] != "\n":
                j += 2 if text[j] == "\\" else 1
            out.append(text[i:j + 1]); i = j + 1
        elif ch == "`":
            k = i
            while k < n and text[k] == "`":
                k += 1
            if k - i == 2 and text[k:k + 1] != " ":
                out.append("``"); i = k
                continue
            j = text.find(text[i:k], k)
            j = n if j < 0 else j + (k - i)
            out.append(text[i:j]); i = j
        elif text.startswith("//", i):
            j = text.find("\n", i); j = n if j < 0 else j
            out.append(" " * (j - i)); i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2); j = n if j < 0 else j + 2
            out.append(re.sub(r"[^\n]", " ", text[i:j])); i = j
        else:
            out.append(ch); i += 1
    return "".join(out)


TOK = re.compile(r'\s+|"(?:[^"\\\n]|\\.)*"|`+|:-|=>|!=|<=|>=|0x[0-9a-fA-F]+|\d+(?:\.\d+)?(?![\w])|@[A-Za-z_]\w*|[A-Za-z_]\w*|.', re.S)


def tokenize(s, with_pos=False):
    toks, i = [], 0
    while i < len(s):
        start = i
        if s[i] == "`":
            k = i
            while k < len(s) and s[k] == "`":
                k += 1
            if k - i == 2 and s[k:k + 1] != " ":     # `` is the empty template
                toks.append((("tick", "``"), start) if with_pos else ("tick", "``")); i = k
                continue
            j = s.find(s[i:k], k)
            if j < 0:
                raise LowerError("unterminated backtick")
            tok = ("tick", s[i:j + (k - i)]); i = j + (k - i)
        else:
            m = TOK.match(s, i)
            t = m.group(0); i = m.end()
            if t.isspace():
                continue
            if t.startswith('"'):
                tok = ("str", t)
            elif t[0].isdigit():
                tok = ("num", t)
            elif t.startswith("@"):
                tok = ("fun", t[1:])
            elif t[0].isalpha() or t[0] == "_":
                tok = ("id", t)
            else:
                tok = ("op", t)
        toks.append((tok, start) if with_pos else tok)
    return toks


def split_statements(text):
    """(directives, clauses): directives as (line, text) with their continuation lines, clauses as (line, tokens)."""
    lines = strip_comments(text).split("\n")
    directives, body = [], []
    i = 0
    while i < len(lines):
        ln = lines[i]
        if ln[:1] in (".", "#") and re.match(r"[.#][a-z]", ln):
            start, buf = i, [ln]
            depth = ln.count("(") - ln.count(")")
            i += 1
            cont = ln.startswith(".violation")
            while i < len(lines) and (depth > 0 or (cont and lines[i][:1] in (" ", "\t") and lines[i].strip())):
                buf.append(lines[i]); depth += lines[i].count("(") - lines[i].count(")"); i += 1
            directives.append((start + 1, "\n".join(buf)))
            body.append("")
            body.extend([""] * (len(buf) - 1))
            continue
        body.append(ln); i += 1
    text = "\n".join(body)
    clauses, cur, start = [], [], None
    for tok, pos in tokenize(text, with_pos=True):
        if start is None:
            start = text.count("\n", 0, pos) + 1
        if tok == ("op", ".") and depth_of(cur) == 0:
            clauses.append((start, cur)); cur, start = [], None
        else:
            cur.append(tok)
    if cur:
        raise LowerError(f"line {start}: statement without a final '.'")
    return directives, clauses


def depth_of(toks):
    d = 0
    for k, t in toks:
        if k == "op" and t in "({[":
            d += 1
        elif k == "op" and t in ")}]":
            d -= 1
    return d


# ---- parsing clauses -------------------------------------------------------------------------------------------
# Literals:  ("atom", name, [expr]) ("not", lit) ("cmp", op, e1, e2) ("or", [[lit]]) ("pat", expr, tick, at) ("bool", b)
# Exprs:     ("var", x) ("wild",) ("num", s) ("str", s) ("tick", s) ("call", f, [e]) ("fun", f, [e]) ("bin", op, a, b)
#            ("neg", e) ("agg", kind, target, [lit]) ("join", x, [keys], sep, [lit])

class Parser:
    def __init__(self, toks):
        self.t, self.i = toks, 0

    def peek(self, k=0):
        j = self.i + k
        return self.t[j] if j < len(self.t) else ("eof", "")

    def take(self, want=None):
        tok = self.peek()
        if want is not None and tok[1] != want:
            raise LowerError(f"expected {want!r}, found {tok[1]!r} in {self.context()}")
        self.i += 1
        return tok

    def at(self, *vals):
        return self.peek()[1] in vals and self.peek()[0] in ("op", "id")

    def context(self):
        return " ".join(t for _, t in self.t[max(0, self.i - 8):self.i + 8])

    def clause(self):
        heads = [self.head()]
        while self.at(","):
            self.take(); heads.append(self.head())
        body, edits = [], None
        if self.at(":-"):
            self.take()
            body = self.conj()
        if self.at("=>"):
            self.take()
            edits = self.t[self.i:]
            self.i = len(self.t)
        if self.peek()[0] != "eof":
            raise LowerError(f"unexpected {self.peek()[1]!r} in {self.context()}")
        return heads, body, edits

    def head(self):
        mode = None
        if self.peek()[1] in ("must", "may") and self.peek(1)[0] == "id":
            mode = self.take()[1]
        name = self.take()[1]
        self.take("(")
        args = self.args(")")
        return (mode, name, args)

    def args(self, close):
        out = []
        if self.at(close):
            self.take(); return out
        while True:
            out.append(self.expr())
            if self.at(","):
                self.take(); continue
            self.take(close); return out

    def disj(self):
        conjs = [self.conj()]
        while self.at(";"):
            self.take(); conjs.append(self.conj())
        return conjs

    def conj(self):
        lits = [self.lit()]
        while self.at(","):
            self.take(); lits.append(self.lit())
        if self.at(";"):
            # a top-level disjunction in a rule body: (a, b ; c) without parentheses
            rest = []
            while self.at(";"):
                self.take(); rest.append(self.conj())
            return [("or", [lits] + rest)]
        return lits

    BIT_WORDS = {"band", "bor", "bxor", "bshl", "bshr", "bshru", "land", "lor", "lxor"}

    def continues_expr(self, arith=True):
        """The next token goes on with an expression: a comparison or an operator (arith: + - * / % ^ too)."""
        k, v = self.peek()
        if k == "id":
            return v in self.BIT_WORDS
        return k == "op" and (v in CMP or arith and v in {"+", "-", "*", "/", "%", "^"})

    def lit(self):
        if self.at("!"):
            self.take(); return ("not", self.lit())
        if self.at("("):
            save = self.i
            try:
                self.take("(")
                b = self.disj()
                self.take(")")
                if not self.continues_expr():
                    return ("or", b) if len(b) > 1 else ("or", b)
            except LowerError:
                pass
            self.i = save
        k, v = self.peek()
        if k == "id" and v in ("true", "false") and self.peek(1)[1] != "(":
            self.take(); return ("bool", v == "true")
        if k == "id" and self.peek(1) == ("op", "(") and v not in BUILTIN_FN:
            save = self.i
            self.take()
            self.take("(")
            args = self.args(")")
            if not (self.peek()[0] == "op" and self.peek()[1] in CMP) and not self.continues_expr(arith=False):
                return ("atom", v, args)
            self.i = save
        e1 = self.expr()
        if self.at("~"):
            self.take()
            tick = self.take()
            if tick[0] != "tick":
                raise LowerError(f"~ needs a backtick pattern in {self.context()}")
            at = None
            if self.peek() == ("id", "at") and self.peek(1)[0] == "str":
                self.take(); at = self.take()[1][1:-1]
            return ("pat", e1, tick[1], at)
        op = self.take()
        if op[1] not in CMP:
            raise LowerError(f"expected a comparison, found {op[1]!r} in {self.context()}")
        return ("cmp", op[1], e1, self.expr())

    # Soufflé's precedence, loosest first: lor, lxor, land, bor, bxor, band, the shifts, + -, * / %, ^, unary
    WORD_LEVELS = [("lor",), ("lxor",), ("land",), ("bor",), ("bxor",), ("band",), ("bshl", "bshr", "bshru")]

    def expr(self, level=0):
        if level < len(self.WORD_LEVELS):
            e = self.expr(level + 1)
            while self.peek()[0] == "id" and self.peek()[1] in self.WORD_LEVELS[level]:
                op = self.take()[1]; e = ("bin", op, e, self.expr(level + 1))
            return e
        e = self.term()
        while self.peek()[0] == "op" and self.peek()[1] in ("+", "-"):
            op = self.take()[1]; e = ("bin", op, e, self.term())
        return e

    def term(self):
        e = self.power()
        while self.peek()[0] == "op" and self.peek()[1] in ("*", "/", "%"):
            op = self.take()[1]; e = ("bin", op, e, self.power())
        return e

    def power(self):
        e = self.unary()
        if self.peek() == ("op", "^"):
            self.take(); e = ("bin", "^", e, self.power())
        return e

    def unary(self):
        if self.at("-"):
            self.take(); return ("neg", self.unary())
        if self.peek() == ("id", "bnot"):
            # bnot x is x bxor -1 (two's complement), so the rest of the lowering sees one more binary operator
            self.take(); return ("bin", "bxor", self.unary(), ("num", "-1"))
        return self.primary()

    def primary(self):
        k, v = self.peek()
        if k == "num":
            self.take(); return ("num", v)
        if k == "str":
            self.take(); return ("str", v)
        if k == "tick":
            self.take(); return ("tick", v)
        if k == "fun":
            self.take(); self.take("("); return ("fun", v, self.args(")"))
        if k == "op" and v == "(":
            self.take(); e = self.expr(); self.take(")"); return e
        if k == "id":
            if v in AGGS and self.peek(1)[1] != "(":
                return self.aggregate()
            if self.peek(1) == ("op", "("):
                self.take(); self.take("("); return ("call", v, self.args(")"))
            self.take()
            # a variable named as a Soufflé keyword (`as`, `ord`) is renamed, as declared columns are
            return ("wild",) if v == "_" else ("var", col_name(v))
        raise LowerError(f"unexpected {v!r} in {self.context()}")

    def aggregate(self):
        kind = self.take()[1]
        if kind == "join":
            x = self.expr()
            if self.peek() != ("id", "by"):
                raise LowerError("join needs `by`")
            self.take()
            if self.at("("):
                self.take(); keys = self.args(")")
            else:
                keys = [self.expr()]
            if self.peek() != ("id", "sep"):
                raise LowerError("join needs `sep`")
            self.take()
            sep = self.take()[1]
            self.take(":")
            return ("join", x, keys, sep, self.agg_body())
        target = None
        if not self.at(":"):
            target = self.expr()
        self.take(":")
        return ("agg", kind, target, self.agg_body())

    def agg_body(self):
        if self.at("{"):
            self.take(); b = self.conj(); self.take("}"); return b
        return [self.lit()]


# ---- printing ----------------------------------------------------------------------------------------------------

def pe(e):
    t = e[0]
    if t == "var":
        return e[1]
    if t == "wild":
        return "_"
    if t in ("num", "str"):
        return e[1]
    if t == "tick":
        return '"' + e[1].strip("`").replace("\\", "\\\\").replace('"', '\\"') + '"'
    if t == "call":
        return f"{e[1]}({', '.join(pe(a) for a in e[2])})"
    if t == "fun":
        return f"@{e[1]}({', '.join(pe(a) for a in e[2])})"
    if t == "bin":
        return f"({pe(e[2])} {e[1]} {pe(e[3])})"
    if t == "neg":
        return f"-{pe(e[1])}"
    if t == "agg":
        tgt = f" {pe(e[2])}" if e[2] is not None else ""
        return f"{e[1]}{tgt} : {{ {pc(e[3])} }}"
    raise LowerError(f"cannot print {t}")


def pl(l):
    t = l[0]
    if t == "atom":
        return f"{l[1]}({', '.join(pe(a) for a in l[2])})"
    if t == "not":
        return "!" + pl(l[1])
    if t == "cmp":
        return f"{pe(l[2])} {l[1]} {pe(l[3])}"
    if t == "or":
        return "(" + " ; ".join(pc(c) for c in l[1]) + ")"
    if t == "bool":
        return "true" if l[1] else "false"
    raise LowerError(f"cannot print {t}")


def pc(conj):
    return ", ".join(pl(l) for l in conj)


def evars(e, out):
    t = e[0]
    if t == "var":
        out.append(e[1])
    elif t in ("call", "fun"):
        for a in e[2]:
            evars(a, out)
    elif t == "bin":
        evars(e[2], out); evars(e[3], out)
    elif t == "neg":
        evars(e[1], out)
    elif t == "agg":
        if e[2] is not None:
            evars(e[2], out)
        for l in e[3]:
            lvars(l, out)
    elif t == "join":
        evars(e[1], out)
        for k in e[2]:
            evars(k, out)
        for l in e[4]:
            lvars(l, out)
    elif t == "tick":
        out.extend(re.findall(r"\$\$?\$?([A-Za-z_]\w*)", e[1]))
    return out


def lvars(l, out):
    t = l[0]
    if t == "atom":
        for a in l[2]:
            evars(a, out)
    elif t == "not":
        lvars(l[1], out)
    elif t == "cmp":
        evars(l[2], out); evars(l[3], out)
    elif t == "or":
        for c in l[1]:
            for x in c:
                lvars(x, out)
    elif t == "pat":
        evars(l[1], out)
        out.extend(v for v in re.findall(r"\$([A-Z_][A-Z0-9_]*)", l[2]) if v != "_" and not l[2].count("$$$" + v))
    return out


# ---- the program ------------------------------------------------------------------------------------------------

class Program:
    """A rule and its includes, lowered. `text()` is the Soufflé program."""

    def __init__(self, path, pattern_compiler=None):
        self.path = os.path.abspath(path)
        self.types = {}            # user type -> base: "number" | "symbol" | "node"
        self.decls = {}            # relation -> [(col, raw type)]
        self.qual = {}             # relation -> Soufflé qualifiers kept
        self.inputs = []           # relations declared .input (plain)
        self.io_inputs = {}        # relation -> {IO=…, filename=…}
        self.outputs = set()
        self.functors = {}         # name -> (arg types, return type)
        self.ordered = set()
        self.violations = {}       # name -> {cols, code, code_col, message, suppress, fixes}
        self.fix_names = set()
        self.language = None
        self.scope = []            # .scope include globs (a path must match one, when there are any)
        self.scope_exclude = []    # .scope exclude globs
        self.on_syntax = None      # .on syntax_error report CODE [at line 1]: (code, at line 1)
        self.scope_tests = True    # .scope tests false: no test paths
        self.scope_words = []
        self.warnings = []
        self.clauses = []          # (file, line, heads, body, edits)
        self.extra = []            # generated Soufflé lines
        self.n = 0
        self.sets = {}             # set key -> id
        self.axes = set()          # (axis, set id)
        self.need = set()          # rr_within, rr_earlier, rr_never
        self.empties = {}          # generated empty relations -> arity types
        self.patterns = pattern_compiler
        self.parse_requests = False  # parsed(s, g, r) is served between rounds (Python)
        self.matches = {}          # k -> match spec, served by rrlab.py between souffle rounds (match_all, spans)
        self.dfs = {}              # k -> (edge, target, roots or None): dfs_first(_memo), served between rounds
        self.pattern_report = []
        self.files = []
        self.load(os.path.join(LIB, "core.rr"))      # every rule's implicit base, as check_rr.py reads it
        self.load(self.path)
        self.lowered = self.lower()

    def warn(self, msg):
        if msg not in self.warnings:
            self.warnings.append(msg)

    # -- reading
    def load(self, path, seen=None):
        if path in self.files:
            return
        self.files.append(path)
        text = open(path).read()
        directives, clauses = split_statements(text)
        rel = os.path.relpath(path, RR)
        for line, d in directives:
            self.directive(rel, line, d)
        for line, toks in clauses:
            try:
                heads, body, edits = Parser(toks).clause()
            except LowerError as e:
                raise LowerError(f"{rel}:{line}: {e}")
            self.clauses.append((rel, line, heads, body, edits))

    def directive(self, rel, line, d):
        if d.startswith("#include"):
            m = re.match(r'#include\s+"([^"]+)"', d)
            self.load(os.path.normpath(os.path.join(LIB, m.group(1))))
            return
        m = re.match(r"\.(\w+)\s*(.*)", d, re.S)
        kw, rest = m.group(1), m.group(2)
        if kw == "type":
            m2 = re.match(r"(\w+)\s*(<:|=)\s*(.*)", rest.strip(), re.S)
            name, op, base = m2.group(1), m2.group(2), m2.group(3)
            parts = [p.strip() for p in base.split("|")]
            self.types[name] = "node" if name in NODEISH or any(self.base(p) == "node" for p in parts) else self.base(parts[0])
        elif kw in ("decl", "edit"):
            self.decl(rest, edit=kw == "edit")
        elif kw == "violation":
            self.violation(rel, line, rest)
        elif kw == "input":
            m2 = re.match(r"(\w+)\s*\((.*)\)\s*$", rest.strip(), re.S)
            if m2 and "=" in m2.group(2):
                self.io_inputs[m2.group(1)] = dict(
                    (k.strip(), v.strip().strip('"')) for k, v in
                    (kv.split("=", 1) for kv in re.findall(r'\w+\s*=\s*(?:"[^"]*"|[^,]+)', m2.group(2))))
            else:
                self.inputs += [x.strip() for x in rest.split(",") if x.strip()]
        elif kw == "output":
            self.outputs |= {re.match(r"\w+", x.strip()).group(0) for x in rest.split(",") if x.strip()}
        elif kw == "functor":
            m2 = re.match(r"(\w+)\s*\((.*?)\)\s*:\s*(\w+)", rest, re.S)
            args = [a.split(":")[1].strip() for a in m2.group(2).split(",") if ":" in a]
            self.functors[m2.group(1)] = (args, m2.group(3))
        elif kw == "language":
            self.language = rest.strip().split()[0]
        elif kw == "scope":
            # .scope include "a", "b" exclude "c" …: the words before each run of strings say which list it joins;
            # `tests false` drops test paths; report/text/dirs entries are not modelled
            word = None
            for m in re.finditer(r'"((?:[^"\\]|\\.)*)"|([A-Za-z_]+)', rest):
                if m.group(2):
                    word = m.group(2)
                    if word == "false" and self.scope_words[-1:] == ["tests"]:
                        self.scope_tests = False
                    self.scope_words.append(word)
                elif word == "include":
                    self.scope.append(m.group(1))
                elif word == "exclude":
                    self.scope_exclude.append(m.group(1))
        elif kw == "on" and rest.strip().startswith("syntax_error"):
            m2 = re.match(r'syntax_error\s+report\s+"?([^"\s]+)"?(\s+at\s+line\s+1)?', rest.strip())
            if m2:
                self.on_syntax = (m2.group(1), bool(m2.group(2)))
        elif kw in DROP_DIRECTIVES:
            if kw == "lattice":
                self.warn(".lattice is not modelled (dropped)")
        else:
            self.warn(f"directive .{kw} not modelled (dropped)")

    def base(self, t):
        if t in NODEISH:
            return "node"
        if t in ("number", "unsigned", "float"):
            return t
        if t in ("symbol", "relation"):
            return "symbol"
        return self.types.get(t, "symbol")

    def decl(self, rest, edit=False):
        m = re.match(r"(\w+)\s*\((.*?)\)(.*)$", rest, re.S)
        if not m:
            raise LowerError(f"bad .decl {rest[:60]}")
        name, cols, suffix = m.group(1), m.group(2), m.group(3)
        cs = []
        for c in [c for c in cols.split(",") if c.strip()]:
            a, t = [x.strip() for x in c.split(":")]
            cs.append((a, t))
        self.decls[name] = cs
        if "ordered by earlier" in suffix:
            self.ordered.add(name)
        self.qual[name] = [q for q in re.findall(r"\w+", suffix) if q in SOUFFLE_QUALIFIERS]

    def violation(self, rel, line, rest):
        m = re.match(r"(\w+)\s*\((.*?)\)(.*)$", rest, re.S)
        name, cols, tail = m.group(1), m.group(2), m.group(3)
        self.decl(f"{name}({cols})")
        v = {"cols": [c.split(":")[0].strip() for c in cols.split(",") if c.strip()], "code": None, "code_col": None,
             "message": "", "suppress": [], "fixes": [], "where": f"{rel}:{line}"}
        toks = tokenize(tail)
        i = 0
        while i < len(toks):
            k, t = toks[i]
            if (k, t) == ("id", "code"):
                if toks[i + 1][0] == "str":
                    v["code"] = toks[i + 1][1][1:-1].encode().decode("unicode_escape"); i += 2
                else:
                    v["code_col"] = toks[i + 2][1]; i += 4
                if i < len(toks) and toks[i] == ("id", "in"):
                    i += 2
                continue
            if (k, t) == ("id", "tag"):
                i += 4; continue
            if (k, t) == ("id", "suppress"):
                # suppress TOOL "a", "b", …: every name
                tool = toks[i + 1][1]; i += 2
                while i < len(toks) and toks[i][0] == "str":
                    v["suppress"].append((tool, toks[i][1][1:-1])); i += 1
                    if i < len(toks) and toks[i] == ("op", ","):
                        i += 1
                    else:
                        break
                continue
            if (k, t) == ("id", "fix"):
                v["fixes"].append(toks[i + 1][1]); self.fix_names.add(toks[i + 1][1]); i += 2
                if i < len(toks) and toks[i][0] == "str":
                    i += 1
                continue
            if k == "str" and not v["message"]:
                v["message"] = re.sub(r'\\(["\\])', r'\1', toks[i][1][1:-1])  # \" and \\ as written
            i += 1
        self.violations[name] = v

    # -- types
    def col_types(self, name):
        if name in self.decls:
            return [self.base(t) for _, t in self.decls[name]]
        return None

    def soufflé_type(self, t):
        b = self.base(t)
        return "number" if b == "node" else b

    # -- lowering
    def fresh(self, stem="v"):
        self.n += 1
        return f"rr{stem}{self.n}"

    def lower(self):
        out = []
        for rel, line, heads, body, edits in self.clauses:
            if edits is not None:
                continue                       # a fix rule: edits are not applied
            if any(h[0] == "may" for h in heads):
                continue
            try:
                out.append(self.clause(rel, line, heads, body))
            except LowerError as e:
                raise LowerError(f"{rel}:{line}: {e}")
        return out

    def clause(self, rel, line, heads, body):
        heads, body = rename_apart(heads, body)
        ctx = Ctx(self, heads)
        ctx.type_body(body, heads)
        hs = []
        for _, name, args in heads:
            hs.append(f"{name}({', '.join(pe(ctx.expr(a, [])) for a in args)})")
        pre = ctx.pre_head
        lits = ctx.conj(body)
        if ctx.markers:
            self.requests(pre + lits, [], ctx)
        text = ", ".join(hs)
        if lits or pre:
            text += " :-\n    " + ", ".join(pl(l) for l in pre + lits)
        return f"// {rel}:{line}\n{text}."

    # -- matches served between rounds: match_all, and match whose captures are spans
    def new_match(self, regex, kind, subject_node, groups, span_groups):
        k = len(self.matches)
        idx = [group_index(regex, g) for g in groups]
        types = ["number" if g in span_groups else "symbol" for g in groups]
        self.matches[k] = {"regex": regex, "all": kind == "match_all", "node": subject_node, "groups": groups,
                           "index": idx, "span": [g in span_groups for g in groups]}
        st = "number" if subject_node else "symbol"
        # i: the match's ordinal, so that two matches with the same captured text stay two rows (count)
        self.decls[f"rr_mres{k}"] = [("s", st), ("i", "number")] + [(f"g{i}", t) for i, t in enumerate(types)]
        self.decls[f"rr_mreq{k}"] = [("s", st)]
        self.inputs.append(f"rr_mres{k}")
        self.outputs.add(f"rr_mreq{k}")
        return k

    def requests(self, conj, outer, ctx):
        """rr_mreq{k}(subject) :- the literals around the match: its conjunction's other literals and those of
        every enclosing conjunction, less any that use a capture."""
        caps = ctx.capture_vars
        def usable(l):
            if "rr_mres" in pl(l) or "parsed(" in pl(l):
                return False
            return not (set(lvars(l, [])) & caps)
        for i, l in enumerate(conj):
            around = outer + [x for j, x in enumerate(conj) if j != i and usable(x)]
            if l[0] == "atom" and l[1] == "parsed" and "parsed" in ctx.markers:
                subj, g = l[2][0], l[2][1]
                g = g if g[0] == "str" else ("str", '"annotation"')
                around = grounded(around)
                if subj[0] == "var" and not any(subj[1] in lvars(x, []) for x in around):
                    self.warn(f"parsed: cannot tell which strings {pe(subj)} to parse; no rows")
                    continue
                head = f"rr_preq({pe(subj)}, {pe(g)})"
                self.extra.append(f"{head} :- {pc(around)}." if around else f"{head}.")
            elif l[0] == "atom" and l[1] in ctx.markers:
                k, subj = ctx.markers[l[1]]
                around = grounded(around)
                body = pc(around) if around else None
                head = f"rr_mreq{k}({pe(subj)})"
                if body is None and subj[0] == "var":
                    self.warn(f"match_all/capture spans: cannot tell which subjects {pe(subj)} to match; no rows")
                    continue
                self.extra.append(f"{head} :- {body}." if body else f"{head}.")
            elif l[0] == "not":
                self.requests([l[1]], around, ctx)
            elif l[0] == "or":
                for c in l[1]:
                    self.requests(c, around, ctx)
            elif l[0] == "cmp":
                for e in (l[2], l[3]):
                    for agg in aggs_of(e):
                        self.requests(agg[3], around, ctx)

    def set_id(self, key):
        if key not in self.sets:
            sid = len(self.sets)
            self.sets[key] = sid
        return self.sets[key]

    def text(self):
        L = [f"// lowered by rrlab from {os.path.relpath(self.path, RR)}"]
        for name, cs in self.decls.items():
            cols = ", ".join(f"{col_name(a)}: {self.soufflé_type(t)}" for a, t in cs)
            q = " " + " ".join(self.qual.get(name, [])) if self.qual.get(name) else ""
            L.append(f".decl {name}({cols}){q}")
        for name in self.ordered:
            cs = self.decls[name]
            L.append(f".decl {name}__prev({', '.join(f'{col_name(a)}: {self.soufflé_type(t)}' for a, t in cs)})")
            L.append(f'.input {name}__prev(IO=file, rfc4180=true, delimiter="\\t")')
            self.outputs.add(name)
        # functors: core's, with erased types, plus the lowering's helpers
        fun = dict(self.functors)
        fun.update(HELPERS)
        for f, (args, ret) in fun.items():
            sig = ", ".join(f"a{i}: {self.soufflé_type(t)}" for i, t in enumerate(args))
            L.append(f".functor {f}({sig}): {self.soufflé_type(ret)}")
        L.append(".decl rrlab_parent(c: number, p: number)\n.decl rrlab_lang(d: number, l: symbol)")
        # relations a library declares for native rows (no .input of its own): rrlab fills them
        native = [n for n in ("directive_global",) if n in self.decls]
        # a typing program's answers (`typing_<program>__<output>`): rrlab runs the program first and fills them
        native += [n for n in self.decls if TYPING.fullmatch(n)]
        ins = [i for i in dict.fromkeys(self.inputs + native + ["rrlab_parent", "rrlab_lang"])
               if i in self.decls or i.startswith("rrlab_")]
        ins += [i for i in self.io_inputs if i in self.decls]
        for i in ins:
            L.append(f'.input {i}(IO=file, rfc4180=true, delimiter="\\t")')
        self.input_names = ins
        for v in self.violations:
            self.outputs.add(v)
        L += self.infrastructure()
        L += self.extra
        for c in self.lowered:
            L.append(c)
        for o in sorted(self.outputs):
            if o in self.decls or o.endswith("__prev"):
                L.append(f'.output {o}(IO=file, rfc4180=true, delimiter="\\t")')
        return "\n".join(L) + "\n"

    def infrastructure(self):
        L = []
        if "rr_within" in self.need or self.axes:
            L += [".decl rr_within(n: number, a: number)",
                  "rr_within(n, a) :- rrlab_parent(n, a).",
                  "rr_within(n, a) :- rrlab_parent(n, p), rr_within(p, a)."]
        if "rr_earlier" in self.need:
            L += [".decl rr_earlier(a: number, b: number)",
                  "rr_earlier(a, b) :- docof(a, d), docof(b, d), visit_index(a, i), visit_index(b, j), i < j."]
        if "rr_never" in self.need:
            L += [".decl rr_never()"]
        for key, sid in self.sets.items():
            L.append(f".decl rr_set{sid}(n: number)")
            kind, v = key
            if kind == "kind":
                L.append(f'rr_set{sid}(n) :- kind(n, "{v}").')
            else:
                ts = self.col_types(v)
                if ts is None:
                    raise LowerError(f"set {v} is not a declared relation")
                if len(ts) != 1:
                    raise LowerError(f"set {v} has {len(ts)} columns; a set is unary")
                L.append(f"rr_set{sid}(n) :- {v}(n)." if ts[0] in ("node", "number") else f"rr_set{sid}(n) :- kind(n, k), {v}(k).")
        for axis, sid in sorted(self.axes):
            r = f"rr_{axis}{sid}"
            L.append(f".decl {r}(a: number, b: number)")
            s = f"rr_set{sid}"
            if axis == "inside":
                L.append(f"{r}(n, a) :- rr_within(n, a), {s}(a).")
            elif axis == "has":
                L.append(f"{r}(a, n) :- rr_within(n, a), {s}(n).")
            elif axis == "nearest":
                L.append(f"{r}(n, a) :- rrlab_parent(n, a), {s}(a).")
                L.append(f"{r}(n, a) :- rrlab_parent(n, p), !{s}(p), {r}(p, a).")
            elif axis == "within_stop":
                L.append(f"{r}(n, a) :- rrlab_parent(n, a).")
                L.append(f"{r}(n, a) :- rrlab_parent(n, p), !{s}(p), {r}(p, a).")
            elif axis == "precedes_stop":
                L.append(f".decl rr_between{sid}(a: number, b: number)")
                L.append(f"rr_between{sid}(a, b) :- precedes(a, s), {s}(s), precedes(s, b).")
                L.append(f"{r}(a, b) :- precedes(a, b), !rr_between{sid}(a, b).")
        for name, types in self.empties.items():
            L.append(f".decl {name}({', '.join(f'c{i}: {t}' for i, t in enumerate(types))})")
        return L


HELPERS = {
    "rr_match": (["symbol", "symbol"], "number"), "rr_search": (["symbol", "symbol"], "number"),
    "rr_cap": (["symbol", "symbol", "number"], "symbol"), "rr_capok": (["symbol", "symbol", "number"], "number"),
    "rr_has_dot": (["symbol"], "number"), "rr_dot_init": (["symbol"], "symbol"), "rr_dot_last": (["symbol"], "symbol"),
}
for _t in TEXT_TESTS:
    HELPERS["rr_" + _t] = (["symbol", "symbol"], "number")


def souffle_unescape(s):
    """A string literal's contents as Soufflé reads them: \\ \" \n \t \r decoded, any other escape kept (so a
    regex escape such as backslash-d stays as written), and non-ASCII characters untouched."""
    out, i = [], 0
    while i < len(s):
        c = s[i]
        if c == "\\" and i + 1 < len(s):
            n = s[i + 1]
            out.append({"\\": "\\", '"': '"', "n": "\n", "t": "\t", "r": "\r"}.get(n, "\\" + n))
            i += 2
            continue
        out.append(c)
        i += 1
    return "".join(out)


def group_index(rx, name):
    """The 1-based index of capture group `name` in rx, counting capturing groups left to right."""
    i, n, cls = 0, 0, False
    while i < len(rx):
        c = rx[i]
        if c == "\\":
            i += 2; continue
        if cls:
            cls = c != "]"
        elif c == "[":
            cls = True
        elif c == "(":
            if rx.startswith("(?P<", i) or (rx.startswith("(?<", i) and rx[i + 3:i + 4] not in ("=", "!")):
                n += 1
                j = rx.index(">", i)
                if rx[i + (4 if rx[i + 2] == "P" else 3):j] == name:
                    return n
            elif not rx.startswith("(?", i):
                n += 1
        i += 1
    return None


class Ctx:
    """Lowering state for one clause: variable types, fresh variables, the clause's own head."""

    def __init__(self, prog, heads):
        self.p = prog
        self.heads = [h[1] for h in heads]
        self.vt = {}
        self.pre_head = []
        self.occ = {}
        self.markers = {}          # rr_mres{k} -> (k, subject): matches served between rounds
        self.capture_vars = set()

    # -- typing: a variable is node-ish if any column it fills is node, span or loc
    def type_body(self, body, heads):
        for _, name, args in heads:
            self.type_atom(name, args)
        for l in body:
            self.type_lit(l)
        allv = []
        for _, _, args in heads:
            for a in args:
                evars(a, allv)
        for l in body:
            lvars(l, allv)
        for v in allv:
            self.occ[v] = self.occ.get(v, 0) + 1

    def type_atom(self, name, args):
        ts = self.p.col_types(name)
        if name in SET_AXES or name in ("within", "earlier", "precedes", "next_sibling"):
            ts = ["node", "node"] + (["symbol"] if name in SET_AXES else [])
        if ts is None:
            return
        for a, t in zip(args, ts):
            if a[0] == "var":
                self.vt.setdefault(a[1], set()).add(t)
            self.type_expr(a)

    def type_expr(self, e):
        if e[0] in ("agg",):
            for l in e[3]:
                self.type_lit(l)
        elif e[0] == "join":
            for l in e[4]:
                self.type_lit(l)
        elif e[0] in ("call", "fun"):
            for a in e[2]:
                self.type_expr(a)

    def type_lit(self, l):
        t = l[0]
        if t == "atom":
            self.type_atom(l[1], l[2])
        elif t == "not":
            self.type_lit(l[1])
        elif t == "or":
            for c in l[1]:
                for x in c:
                    self.type_lit(x)
        elif t == "cmp":
            self.type_expr(l[2]); self.type_expr(l[3])
            for a, b in ((l[2], l[3]), (l[3], l[2])):
                if a[0] == "var":
                    bt = self.etype(b)
                    if bt:
                        self.vt.setdefault(a[1], set()).add(bt)
        elif t == "pat":
            if l[1][0] == "var":
                self.vt.setdefault(l[1][1], set()).add("node")
            for v in lvars(l, [])[1:]:
                self.vt.setdefault(v, set()).add("node")

    def etype(self, e):
        t = e[0]
        if t == "var":
            ts = self.vt.get(e[1], set())
            return "node" if "node" in ts else next(iter(ts), None)
        if t in ("str", "tick"):
            return "symbol"
        if t == "num":
            return "number"
        if t == "call":
            return BUILTIN_FN.get(e[1], "number")
        if t == "fun":
            f = self.p.functors.get(e[1]) or HELPERS.get(e[1])
            return self.p.base(f[1]) if f else None
        if t in ("bin", "neg"):
            return "number"
        if t == "agg":
            return "number" if e[1] == "count" else (self.etype(e[2]) if e[2] is not None else "number")
        if t == "join":
            return "symbol"
        return None

    def nodeish(self, e):
        return self.etype(e) == "node"

    # -- rewriting
    def conj(self, lits, neg_scope=False, in_agg=False):
        out = []
        for l in lits:
            out += self.lit(l, in_agg=in_agg)
        return out

    def subject(self, e, pre):
        """A text test's subject: a node's text, else the value itself."""
        e = self.expr(e, pre)
        if self.nodeish(e) or e[0] == "fun" and self.etype(e) == "node":
            t = ("var", self.p.fresh("t"))
            pre.append(("atom", "text", [e, t]))
            return t
        return e

    def lit(self, l, in_agg=False):
        """[literal]: the lowering of l, with any binding literals it needs first."""
        t = l[0]
        pre = []
        if t == "atom":
            core = self.atom(l, pre, negated=False, in_agg=in_agg)
            return pre + core
        if t == "not":
            inner = l[1]
            if inner[0] == "atom":
                core = self.atom(inner, pre, negated=True, in_agg=True)
            elif inner[0] == "pat":
                core = self.pattern(inner, pre, negated=True)
            elif inner[0] == "cmp":
                c = self.lit(inner, in_agg)
                core = c[-1:]; pre += c[:-1]
            else:
                # !( … ): a group, as count : { … } = 0
                return [("cmp", "=", ("agg", "count", None, self.conj(inner[1][0] if inner[0] == "or" and len(inner[1]) == 1 else [inner], in_agg=True)), ("num", "0"))]
            return pre + [negate(c) for c in core] if len(core) == 1 else pre + [self.neg_group(core)]
        if t == "cmp":
            e1 = self.expr(l[2], pre, in_agg)
            e2 = self.expr(l[3], pre, in_agg)
            return pre + [("cmp", l[1], e1, e2)]
        if t == "or":
            return [("or", [self.conj(c, in_agg=in_agg) for c in l[1]])]
        if t == "pat":
            return pre + self.pattern(l, pre)
        if t == "bool":
            if not l[1]:
                self.p.need.add("rr_never")
                return [("atom", "rr_never", [])]
            return []
        raise LowerError(f"cannot lower {t}")

    def neg_group(self, core):
        """!(a, b): a helper relation over the group's outer variables, negated."""
        vs = []
        for l in core:
            lvars(l, vs)
        cols = [v for v in dict.fromkeys(vs) if self.occ.get(v, 0) > 1 or v.startswith("rrt")]
        name = self.p.fresh("neg")
        types = [self.soufflé_var_type(v) for v in cols]
        self.p.extra.append(f".decl {name}({', '.join(f'c{i}: {t}' for i, t in enumerate(types))})")
        self.p.extra.append(f"{name}({', '.join(cols)}) :- {', '.join(pl(x) for x in core)}.")
        return ("not", ("atom", name, [("var", v) for v in cols]))

    def soufflé_var_type(self, v):
        ts = self.vt.get(v, set())
        return "number" if ts & {"node", "number"} else "symbol"

    def atom(self, l, pre, negated, in_agg):
        _, name, args = l
        p = self.p
        if name == "earlier" and len(args) == 2:
            a, b = (self.expr(x, pre) for x in args)
            if negated:
                p.need.add("rr_earlier")
                return [("atom", "rr_earlier", [a, b])]
            d, i, j = (("var", p.fresh(s)) for s in ("d", "i", "j"))
            return [("atom", "docof", [a, d]), ("atom", "docof", [b, d]), ("atom", "visit_index", [a, i]),
                    ("atom", "visit_index", [b, j]), ("cmp", "<", i, j)]
        if name == "within" and len(args) == 2:
            p.need.add("rr_within")
            return [("atom", "rr_within", [self.expr(x, pre) for x in args])]
        if name in SET_AXES and len(args) == 3:
            s = args[2]
            if s[0] == "var":
                key = ("rel", s[1])
            elif s[0] == "str":
                key = ("kind", s[1][1:-1])
                if "," in key[1] or "|" in key[1]:
                    raise LowerError(f"{name}'s set {s[1]} names several kinds in one string: a kind string is one "
                                     f"kind; declare a relation of the kinds and pass its name")
            else:
                raise LowerError(f"{name}'s set must be a relation name or a kind string")
            sid = p.set_id(key)
            p.axes.add((name, sid))
            return [("atom", f"rr_{name}{sid}", [self.expr(x, pre) for x in args[:2]])]
        if name == "parsed" and p.language == "python" and len(args) == 3:
            # served between rounds: rrlab parses the strings the rule asks about (rr_preq) and grafts their trees
            p.parse_requests = True
            if "rr_preq" not in p.decls:
                p.decls["rr_preq"] = [("s", "number"), ("g", "symbol")]
                p.outputs.add("rr_preq")
            a = [self.expr(x, pre) for x in args]
            if a[2][0] == "var":
                self.capture_vars.add(a[2][1])
            self.markers.setdefault("parsed", ("parsed", None))
            return [("atom", "parsed", a)]
        if name == "parsed" and p.language != "vue":
            p.warn("parsed is modelled for Vue and Python files only: other files have no rows")
        if name in ("match", "match_all") and len(args) == 2 and args[0][0] == "str":
            raw = souffle_unescape(args[0][1][1:-1])
            groups = re.findall(r"\(\?P?<([A-Za-z_]\w*)>", raw)
            subj = self.expr(args[1], pre)
            node_subj = self.nodeish(subj)
            spans = {g for g in groups if node_subj and "node" in self.vt.get(g, ())}
            if name == "match_all" or spans:
                k = p.new_match(raw, name, node_subj, groups, spans)
                rel = f"rr_mres{k}"
                self.markers[rel] = (k, subj)
                self.capture_vars |= set(groups)
                cols = [subj, ("wild",)] + [("wild",) if negated else ("var", g) for g in groups]
                return [("atom", rel, cols)]
        if name in ("dfs_first", "dfs_first_memo") and len(args) == (5 if name == "dfs_first" else 6) and \
                all(a[0] == "var" for a in args[3:]):
            # served between rounds: souffle writes the edge, target and roots relations, rrlab searches them
            # (core.rr's order) and answers rr_dfs{k}(r, hit, first)
            rels = [a[1] for a in args[3:]]
            key = (name, *rels)
            k = next((i for i, v in p.dfs.items() if v == key), len(p.dfs))
            if k == len(p.dfs):
                p.dfs[k] = key
                p.decls[f"rr_dfs{k}"] = [("r", "number"), ("hit", "number"), ("first", "number")]
                p.inputs.append(f"rr_dfs{k}")
                p.outputs.update(rels)
            return [("atom", f"rr_dfs{k}", [self.expr(x, pre) for x in args[:3]])]
        if name in ("dfs_first", "dfs_first_memo", "match_all", "string_annotation", "signed_list",
                    "git_rev", "tsconfig", "tsconfig_for", "tsconfig_path",
                    "package_export"):
            p.warn(f"{name} is not modelled: it has no rows")
            if name in ("dfs_first", "dfs_first_memo", "match_all"):
                keep = args[:3] if name.startswith("dfs") else []
                if name == "match_all":
                    p.need.add("rr_never")
                    return [("atom", "rr_never", [])]
                e = f"rr_empty_{name}"
                p.empties[e] = ["number"] * 3
                return [("atom", e, [self.expr(x, pre) for x in keep])]
        if name == "split_dot" and len(args) == 3:
            q, m, x = (self.expr(a, pre) for a in args)
            if q[0] != "var" or self.occ.get(q[1], 0) > 1:
                return [("cmp", "=", ("fun", "rr_has_dot", [q]), ("num", "1")),
                        ("cmp", "=", m, ("fun", "rr_dot_init", [q])), ("cmp", "=", x, ("fun", "rr_dot_last", [q]))]
            return [("cmp", "=", q, ("fun", "dot", [m, x])), ("cmp", "!=", m, ("str", '""'))]
        if name in TEXT_TESTS and len(args) == 2:
            pat = self.expr(args[0], pre)
            s = self.subject(args[1], pre)
            return [("cmp", "=", ("fun", "rr_" + name, [pat, s]), ("num", "1"))]
        if name == "match" and len(args) == 2:
            rx = self.expr(args[0], pre)
            s = self.subject(args[1], pre)
            core = [("cmp", "=", ("fun", "rr_match", [rx, s]), ("num", "1"))]
            if rx[0] == "str" and not negated:
                raw = souffle_unescape(rx[1][1:-1])
                for g in re.findall(r"\(\?P?<([A-Za-z_]\w*)>", raw):
                    k = group_index(raw, g)
                    core.append(("cmp", "=", ("fun", "rr_capok", [rx, s, ("num", str(k))]), ("num", "1")))
                    core.append(("cmp", "=", ("var", g), ("fun", "rr_cap", [rx, s, ("num", str(k))])))
            return core
        if name in p.ordered and name in self.heads and in_agg:
            return [("atom", name + "__prev", [self.expr(x, pre) for x in args])]
        if name == "static_value" or (name == "visit_index" and p.language == "python") or name == "syntax_error":
            p.warn(APPROX[name])
        return [("atom", name, [self.expr(x, pre, in_agg) for x in args])]

    def expr(self, e, pre, in_agg=False):
        t = e[0]
        if t in ("call", "fun"):
            stubs = ("tree_digest", "unparse", "nfkc") + (() if os.path.exists("/opt/homebrew/opt/libsodium/include/sodium.h")
                                                          or os.path.exists("/usr/local/opt/libsodium/include/sodium.h")
                                                          or os.path.exists("/usr/include/sodium.h") else ("ed25519_ok",))
            if t == "fun" and e[1] in stubs:
                self.p.warn(f"@{e[1]} is a stub in rrlab")
            return (t, e[1], [self.expr(a, pre, in_agg) for a in e[2]])
        if t == "bin":
            return ("bin", e[1], self.expr(e[2], pre, in_agg), self.expr(e[3], pre, in_agg))
        if t == "neg":
            return ("neg", self.expr(e[1], pre, in_agg))
        if t == "agg":
            if e[1] in ("min", "max") and e[2] is not None and e[2][0] == "var" and self.etype(e[2]) == "symbol":
                return self.symbol_extreme(e, pre)
            body = self.conj(e[3], in_agg=True)
            tgt = self.expr(e[2], body, True) if e[2] is not None else None
            return ("agg", e[1], tgt, body)
        if t == "join":
            return self.join(e, pre)
        return e

    def join(self, e, pre):
        _, x, keys, sep, body = e
        p = self.p
        inner = []
        for l in body:
            lvars(l, inner)
        evars(x, inner)
        for k in keys:
            evars(k, inner)
        # free: variables of the body that the clause also uses outside it
        local = {}
        for v in inner:
            local[v] = local.get(v, 0) + 1
        free = [v for v in dict.fromkeys(inner) if self.occ.get(v, 0) > local[v]]
        n = p.fresh("join")
        lb = self.conj(body, in_agg=True)
        cols = [("var", v) for v in free]
        kx = [self.expr(k, lb, True) for k in keys] + [self.expr(x, lb, True)]
        ktypes = [("number" if self.etype(k) in ("node", "number") else "symbol") for k in kx]
        ftypes = [self.soufflé_var_type(v) for v in free]
        F = ", ".join(free)
        Fc = (F + ", ") if free else ""
        row, rank, acc, res, anyr = (f"{n}_{s}" for s in ("row", "rank", "acc", "res", "any"))
        decl = lambda r, ts: p.extra.append(f".decl {r}({', '.join(f'c{i}: {t}' for i, t in enumerate(ts))})")
        decl(row, ftypes + ktypes)
        decl(rank, ftypes + ["number", "symbol"])
        decl(acc, ftypes + ["number", "symbol"])
        decl(res, ftypes + ["symbol"])
        decl(anyr, ftypes)
        p.extra.append(f"{row}({Fc}{', '.join(pe(k) for k in kx)}) :- {pc(lb) if lb else 'true'}.")
        ks = [f"rrj_k{i}" for i in range(len(kx))]
        counts = []
        for i in range(len(kx)):
            other = [f"rrj_o{j}" for j in range(len(kx))]
            cond = [f"{other[j]} = {ks[j]}" for j in range(i)] + [f"{other[i]} < {ks[i]}"]
            counts.append(f"rrj_c{i} = count : {{ {row}({Fc}{', '.join(other)}), {', '.join(cond)} }}")
        p.extra.append(f"{rank}({Fc}{' + '.join(f'rrj_c{i}' for i in range(len(kx)))}, {ks[-1]}) :- "
                       f"{row}({Fc}{', '.join(ks)}), {', '.join(counts)}.")
        p.extra.append(f"{acc}({Fc}0, rrj_x) :- {rank}({Fc}0, rrj_x).")
        p.extra.append(f'{acc}({Fc}rrj_r, cat(cat(rrj_s, {sep}), rrj_x)) :- {rank}({Fc}rrj_r, rrj_x), rrj_r > 0, '
                       f'{acc}({Fc}rrj_r0, rrj_s), rrj_r0 = rrj_r - 1.')
        p.extra.append(f"{res}({Fc}rrj_s) :- {acc}({Fc}rrj_r, rrj_s), "
                       f"rrj_n = count : {{ {row}({Fc}{', '.join('_' for _ in kx)}) }}, rrj_r = rrj_n - 1.")
        p.extra.append(f"{anyr}({F}) :- {row}({Fc}{', '.join('_' for _ in kx)}).")
        out = ("var", p.fresh("s"))
        fv = [("var", v) for v in free]
        pre.append(("or", [[("atom", res, fv + [out])], [("not", ("atom", anyr, fv)), ("cmp", "=", out, ("str", '""'))]]))
        return out

    def symbol_extreme(self, e, pre):
        """`t = min x : { … }` over symbols, which the engine orders by their bytes and Soufflé's min refuses: the
        rows' x values, then the one no other row's x is below (min) or above (max). No rows: no value, as min."""
        _, kind, x, body = e
        p = self.p
        inner = []
        for l in body:
            lvars(l, inner)
        evars(x, inner)
        local = {}
        for v in inner:
            local[v] = local.get(v, 0) + 1
        free = [v for v in dict.fromkeys(inner) if self.occ.get(v, 0) > local[v]]
        n = p.fresh("ext")
        lb = self.conj(body, in_agg=True)
        xv = self.expr(x, lb, True)
        ftypes = [self.soufflé_var_type(v) for v in free]
        row, beat = f"{n}_row", f"{n}_beaten"
        p.extra.append(f".decl {row}({', '.join(f'c{i}: {t}' for i, t in enumerate(ftypes + ['symbol']))})")
        p.extra.append(f".decl {beat}({', '.join(f'c{i}: {t}' for i, t in enumerate(ftypes + ['symbol']))})")
        Fc = ", ".join(free) + (", " if free else "")
        p.extra.append(f"{row}({Fc}{pe(xv)}) :- {pc(lb) if lb else 'true'}.")
        op = "<" if kind == "min" else ">"
        p.extra.append(f"{beat}({Fc}rrx_a) :- {row}({Fc}rrx_a), {row}({Fc}rrx_b), rrx_b {op} rrx_a.")
        out = ("var", p.fresh("m"))
        fv = [("var", v) for v in free]
        pre.append(("atom", row, fv + [out]))
        pre.append(("not", ("atom", beat, fv + [out])))
        return out

    def pattern(self, l, pre, negated=False):
        _, e, tick, at = l
        if self.p.patterns is None:
            raise LowerError("a pattern atom needs the pattern compiler")
        root = self.expr(e, pre)
        lang = self.p.language or "typescript"
        name, vars_ = self.p.patterns(self.p, lang, tick, at)
        args = [root] + [("wild",) if negated else ("var", v) for v in vars_]
        return [("atom", name, args)]


# Soufflé keywords and built-in names, which a declaration cannot use as a column name
SOUFFLE_WORDS = {"as", "nil", "true", "false", "min", "max", "sum", "count", "mean", "range", "cat", "ord", "strlen",
                 "substr", "contains", "match", "to_number", "to_string", "to_float", "to_unsigned", "band", "bor",
                 "bxor", "bnot", "bshl", "bshr", "bshru", "land", "lor", "lxor", "lnot", "autoinc", "choice", "brie",
                 "btree", "eqrel", "inline", "magic", "input", "output", "printsize", "overridable"}


def col_name(a):
    return a + "_" if a in SOUFFLE_WORDS else a


def named_args(l):
    """An atom's arguments that are values: a set axis's last argument names a relation."""
    return l[2][:2] if l[1] in SET_AXES and len(l[2]) == 3 else l[2]


def rename_apart(heads, body):
    """Aggregate locals renamed apart, as Soufflé scopes them: a variable of an aggregate's (or join's) body is the
    clause's when the clause uses it outside, else the aggregate's own, so `count : { a(x) }, count : { b(x) }`
    has two x. Each occurrence takes the outermost enclosing scope that uses the variable directly; inner scopes
    get a name of their own. Variables named inside a pattern's backticks keep their names."""
    occ = collections.defaultdict(set)     # var -> scope paths using it directly
    ticks = set()
    ids = iter(range(1, 1 << 30))

    def ev(e, path, walk):
        t = e[0]
        if t == "var":
            walk(e, path)
        elif t == "tick":
            ticks.update(re.findall(r"\$\$?\$?([A-Za-z_]\w*)", e[1]))
        elif t in ("call", "fun"):
            for a in e[2]:
                ev(a, path, walk)
        elif t == "bin":
            ev(e[2], path, walk); ev(e[3], path, walk)
        elif t == "neg":
            ev(e[1], path, walk)
        elif t in ("agg", "join"):
            inner = path + (id(e),)
            parts = ([e[2]] if e[2] is not None else []) if t == "agg" else [e[1]] + list(e[2])
            for x in parts:
                ev(x, inner, walk)
            for l in (e[3] if t == "agg" else e[4]):
                lv(l, inner, walk)

    def lv(l, path, walk):
        t = l[0]
        if t == "atom":
            for a in named_args(l):
                ev(a, path, walk)
        elif t == "not":
            lv(l[1], path, walk)
        elif t == "cmp":
            ev(l[2], path, walk); ev(l[3], path, walk)
        elif t == "or":
            for c in l[1]:
                for x in c:
                    lv(x, path, walk)
        elif t == "pat":
            ev(l[1], path, walk)
            ticks.update(re.findall(r"\$\$?\$?([A-Za-z_]\w*)", l[2]))

    def note(e, path):
        occ[e[1]].add(path)
    for _, _, args in heads:
        for a in args:
            ev(a, (), note)
    for l in body:
        lv(l, (), note)
    if not any(len(p) for ps in occ.values() for p in ps):
        return heads, body
    names = {}

    def name(v, path):
        if v in ticks:
            return v
        for k in range(len(path) + 1):
            if path[:k] in occ[v]:
                q = path[:k]
                break
        if not q:
            return v
        if (v, q) not in names:
            names[(v, q)] = f"{v}_rr{next(ids)}"
        return names[(v, q)]

    def re_e(e, path):
        t = e[0]
        if t == "var":
            return ("var", name(e[1], path))
        if t in ("call", "fun"):
            return (t, e[1], [re_e(a, path) for a in e[2]])
        if t == "bin":
            return ("bin", e[1], re_e(e[2], path), re_e(e[3], path))
        if t == "neg":
            return ("neg", re_e(e[1], path))
        if t == "agg":
            inner = path + (id(e),)
            return ("agg", e[1], re_e(e[2], inner) if e[2] is not None else None, [re_l(l, inner) for l in e[3]])
        if t == "join":
            inner = path + (id(e),)
            return ("join", re_e(e[1], inner), [re_e(k, inner) for k in e[2]], e[3], [re_l(l, inner) for l in e[4]])
        return e

    def re_l(l, path):
        t = l[0]
        if t == "atom":
            keep = len(named_args(l))
            return ("atom", l[1], [re_e(a, path) for a in l[2][:keep]] + list(l[2][keep:]))
        if t == "not":
            return ("not", re_l(l[1], path))
        if t == "cmp":
            return ("cmp", l[1], re_e(l[2], path), re_e(l[3], path))
        if t == "or":
            return ("or", [[re_l(x, path) for x in c] for c in l[1]])
        if t == "pat":
            return ("pat", re_e(l[1], path)) + tuple(l[2:])
        return l
    return ([(h[0], h[1], [re_e(a, ()) for a in h[2]]) for h in heads], [re_l(l, ()) for l in body])


def free_vars(e, out):
    """The variables of an expression, less those local to its aggregates."""
    if e[0] == "var":
        out.add(e[1])
    elif e[0] in ("call", "fun"):
        for a in e[2]:
            free_vars(a, out)
    elif e[0] == "bin":
        free_vars(e[2], out); free_vars(e[3], out)
    elif e[0] == "neg":
        free_vars(e[1], out)
    elif e[0] == "agg" and e[2] is not None and e[2][0] == "var":
        pass
    return out


def grounded(lits):
    """The literals whose variables the rest still bind, once some are taken away: positive atoms bind theirs,
    `v = e` binds v when e's variables are bound; anything else must find its variables bound."""
    lits = list(lits)
    while True:
        bound = set()
        for l in lits:
            if l[0] == "atom":
                bound |= {a[1] for a in l[2] if a[0] == "var"}
        changed = True
        while changed:
            changed = False
            for l in lits:
                if l[0] == "cmp" and l[1] == "=":
                    for a, b in ((l[2], l[3]), (l[3], l[2])):
                        if a[0] == "var" and a[1] not in bound and free_vars(b, set()) <= bound:
                            bound.add(a[1]); changed = True
        keep = []
        for l in lits:
            if l[0] == "atom":
                need = set().union(*[free_vars(a, set()) for a in l[2] if a[0] != "var"]) if l[2] else set()
            elif l[0] == "cmp":
                need = free_vars(l[2], set()) | free_vars(l[3], set())
            else:
                need = set(lvars(l, []))
            if need <= bound:
                keep.append(l)
        if len(keep) == len(lits):
            return keep
        lits = keep


def aggs_of(e):
    """The aggregates in an expression."""
    if e[0] == "agg":
        return [e]
    out = []
    if e[0] in ("call", "fun"):
        for a in e[2]:
            out += aggs_of(a)
    elif e[0] == "bin":
        out += aggs_of(e[2]) + aggs_of(e[3])
    elif e[0] == "neg":
        out += aggs_of(e[1])
    return out


def negate(l):
    if l[0] == "cmp":
        inv = {"=": "!=", "!=": "=", "<": ">=", ">=": "<", ">": "<=", "<=": ">"}
        if l[3] == ("num", "1") and l[2][0] == "fun" and l[2][1].startswith("rr_"):
            return ("cmp", "=", l[2], ("num", "0"))
        return ("cmp", inv[l[1]], l[2], l[3])
    if l[0] == "not":
        return l[1]
    return ("not", l)
