"""Rewrite .rr text from the first draft's syntax into Soufflé form.

First draft            Soufflé form
  $x                   x                       (`$` stays only inside backtick patterns and templates)
  .use a/b             #include "a/b.rr"
  .set s = "a" | "b"   .decl s(v: symbol)  s("a"). s("b").
  .kindset k = …       .decl k(kind: symbol)  k("…").
  .native r(…)         .decl r(…) and an `.input` list per section
  .native edit r(…)    .edit r(…)              (built in: no rules)
  sym / int / set      symbol / number / relation
  $x in s              s(x)
  $x in {"a", "b"}     (x = "a" ; x = "b")
  !($x in {"a", "b"})  x != "a", x != "b"
  e =~ "re"            match("re", e)          (whole-string: `^`/`$` dropped, `.*` added where unanchored)
  some { … }           count : { … } > 0
  !some { … }, !( … )  count : { … } = 0
  count { … }          count : { … }
  @concat / @len       cat / strlen            (Soufflé built-ins)
  @cat                 @dot                    (a.b; not Soufflé's cat, which is plain concatenation)

Used once on the hand-written files and on every translation the page shows.
"""
import collections, re

IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


# ---- regexes: search semantics -> Soufflé's whole-string `match` -------------------------------------------

def _top_alts(r):
    """Split r at top-level `|` (outside groups and classes)."""
    out, depth, cls, i, start = [], 0, False, 0, 0
    while i < len(r):
        ch = r[i]
        if ch == "\\":
            i += 2
            continue
        if cls:
            if ch == "]":
                cls = False
        elif ch == "[":
            cls = True
            if r[i + 1:i + 2] == "]":
                i += 1
        elif ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        elif ch == "|" and depth == 0:
            out.append(r[start:i])
            start = i + 1
        i += 1
    out.append(r[start:])
    return out


def _ends_anchor(a):
    if not a.endswith("$"):
        return False
    return (len(a[:-1]) - len(a[:-1].rstrip("\\"))) % 2 == 0


def whole(r):
    """A regex for whole-string matching equivalent to searching for r."""
    flags = ""
    m = re.match(r"\(\?[a-zA-Z]+\)", r)
    if m:
        flags, r = m.group(0), r[m.end():]
    multiline = "m" in flags
    # Text that may span lines needs a wrapper that crosses newlines.
    any_ = "(?s:.*)" if multiline or "\\n" in r or "\\r" in r else ".*"

    def starts(a):
        return (a.startswith("^") and not multiline) or a.startswith("\\A")

    def strip_start(a):
        return a[2:] if a.startswith("\\A") else a[1:]

    def ends(a):
        if a.endswith("\\z") or a.endswith("\\Z"):
            return (len(a) - 2 - len(a[:-2].rstrip("\\"))) % 2 == 0
        return not multiline and _ends_anchor(a)

    def strip_end(a):
        return a[:-2] if a.endswith("\\z") or a.endswith("\\Z") else a[:-1]

    alts = _top_alts(r)
    if len(alts) == 1:
        a = alts[0]
        left, right = starts(a), ends(a)
        if left:
            a = strip_start(a)
        if right:
            a = strip_end(a)
        if not left and (a.startswith(".*") or a.startswith("(?s:.*)")):
            left = True
        if not right and a.endswith(".*") and not a.endswith("\\.*"):
            right = True
        return flags + ("" if left else any_) + a + ("" if right else any_)
    left = all(starts(a) for a in alts)
    right = all(ends(a) for a in alts)
    if left:
        alts = [strip_start(a) for a in alts]
    if right:
        alts = [strip_end(a) for a in alts]
    return flags + ("" if left else any_) + "(?:" + "|".join(alts) + ")" + ("" if right else any_)


# ---- scanning --------------------------------------------------------------------------------------------

def segments(text):
    """Yield (kind, s): kind is code, str, tick or comment."""
    i, n, buf = 0, len(text), []
    while i < n:
        ch = text[i]
        if ch == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            if buf:
                yield "code", "".join(buf); buf = []
            yield "str", text[i:j + 1]
            i = j + 1
        elif ch == "`":
            k = i
            while k < n and text[k] == "`":
                k += 1
            fence = text[i:k]
            j = text.find(fence, k)
            j = n if j < 0 else j
            if buf:
                yield "code", "".join(buf); buf = []
            yield "tick", text[i:j + len(fence)]
            i = j + len(fence)
        elif text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            if buf:
                yield "code", "".join(buf); buf = []
            yield "comment", text[i:j]
            i = j
        else:
            buf.append(ch)
            i += 1
    if buf:
        yield "code", "".join(buf)


def comment_vars(c):
    """In a comment: `$x` (a lowercase Datalog variable) -> x; backtick spans and $UPPER stay."""
    parts = re.split(r"(`[^`]*`)", c)
    return "".join(p if p.startswith("`") else re.sub(r"\$(?=[a-z_][a-z0-9_]*\b)", "", p) for p in parts)


# ---- body rewriting on a token list ----------------------------------------------------------------------

TOK = re.compile(r'\s+|"(?:[^"\\]|\\.)*"|`+|//[^\n]*|=~|:-|=>|!=|<=|>=|\$\$\$[A-Za-z_]\w*|\$[A-Za-z_]\w*|@?[A-Za-z_]\w*|\d+|.', re.S)


def tokens(text):
    out, i = [], 0
    for kind, s in segments(text):
        if kind == "code":
            out += TOK.findall(s)
        else:
            out.append(s)
    return out


def _skip_ws(t, i, step=1):
    while 0 <= i < len(t) and t[i].strip() == "":
        i += step
    return i


def _match_close(t, i, op, cl):
    depth = 0
    for j in range(i, len(t)):
        if t[j] == op:
            depth += 1
        elif t[j] == cl:
            depth -= 1
            if depth == 0:
                return j
    raise ValueError("unbalanced " + op)


def _set_items(t, i):
    """t[i] == '{': return (values, index of '}')."""
    j = _match_close(t, i, "{", "}")
    vals = [x for x in t[i + 1:j] if x.strip() and x != ","]
    return vals, j


def rewrite(t):
    out, i = [], 0
    while i < len(t):
        x = t[i]
        # !( … ) forms
        if x == "!" and _skip_ws(t, i + 1) < len(t) and t[_skip_ws(t, i + 1)] == "(":
            p = _skip_ws(t, i + 1)
            q = _match_close(t, p, "(", ")")
            inner = t[p + 1:q]
            core = [y for y in inner if y.strip()]
            # !($x in set) / !($x in {…})
            if len(core) >= 3 and core[0].startswith("$") and core[1] == "in":
                v = core[0][1:]
                if core[2] == "{":
                    vals = [y for y in core[3:-1] if y != ","]
                    out.append(", ".join(f"{v} != {y}" for y in vals))
                else:
                    out.append(f"!{core[2]}({v})")
                i = q + 1
                continue
            # !(e =~ "re")
            if len(core) == 3 and core[1] == "=~":
                out.append(f'!match("{whole(core[2][1:-1])}", {rewrite([core[0]])[0]})')
                i = q + 1
                continue
            out.append("count : { " + "".join(rewrite(inner)).strip() + " } = 0")
            i = q + 1
            continue
        # !some { … } / some { … } / count { … }
        if x in ("some", "count") or (x == "!" and i + 1 < len(t) and t[_skip_ws(t, i + 1)] == "some"):
            neg = x == "!"
            k = _skip_ws(t, i + 1) if neg else i
            b = _skip_ws(t, k + 1)
            if b < len(t) and t[b] == "{" and t[k] in ("some", "count"):
                e = _match_close(t, b, "{", "}")
                body = "".join(rewrite(t[b + 1:e])).strip()
                if t[k] == "count":
                    out.append("count : { " + body + " }")
                else:
                    out.append("count : { " + body + " } " + ("= 0" if neg else "> 0"))
                i = e + 1
                continue
        # $x in set / $x in {…}
        if x.startswith("$") and not x.startswith("$$$"):
            n1 = _skip_ws(t, i + 1)
            if n1 < len(t) and t[n1] == "in":
                n2 = _skip_ws(t, n1 + 1)
                v = x[1:]
                if t[n2] == "{":
                    vals, e = _set_items(t, n2)
                    out.append(f"{v} = {vals[0]}" if len(vals) == 1 else "(" + " ; ".join(f"{v} = {y}" for y in vals) + ")")
                    i = e + 1
                else:
                    out.append(f"{t[n2]}({v})")
                    i = n2 + 1
                continue
            if n1 < len(t) and t[n1] == "=~":
                n2 = _skip_ws(t, n1 + 1)
                out.append(f'match("{whole(t[n2][1:-1])}", {v_of(x)})')
                i = n2 + 1
                continue
            out.append(v_of(x))
            i += 1
            continue
        if x == "@concat":
            out.append("cat"); i += 1; continue
        if x == "@len":
            out.append("strlen"); i += 1; continue
        if x == "@cat":
            out.append("@dot"); i += 1; continue
        out.append(x)
        i += 1
    return out


def v_of(x):
    return x[1:] if x.startswith("$") and not x.startswith("$$$") else x


# ---- line-level directives -------------------------------------------------------------------------------

TYPES = [(re.compile(r"(\w+\s*:\s*)sym\b"), r"\1symbol"), (re.compile(r"(\w+\s*:\s*)int\b"), r"\1number"),
         (re.compile(r"(\w+\s*:\s*)set\b"), r"\1relation")]


def types(s):
    for p, r in TYPES:
        s = p.sub(r, s)
    return s


def facts(name, vals, indent=""):
    lines, cur = [], indent
    for v in vals:
        f = f"{name}({v})."
        if len(cur) + len(f) + 1 > 88 and cur.strip():
            lines.append(cur.rstrip())
            cur = indent
        cur += f + " "
    if cur.strip():
        lines.append(cur.rstrip())
    return lines


def directives(text):
    """.use, .set/.kindset, .native, types. Returns new text."""
    src = text.split("\n")
    out, i, pending_inputs = [], 0, []

    def flush():
        if pending_inputs:
            out.append(".input " + ", ".join(dict.fromkeys(pending_inputs)))
            pending_inputs.clear()

    while i < len(src):
        line = src[i]
        if re.match(r"// ----", line) or (line.strip() == "" and pending_inputs and i + 1 < len(src)
                                           and src[i + 1].startswith("// ----")):
            flush()
        m = re.match(r"\.use\s+(\S+)(.*)$", line)
        if m:
            out.append(f'#include "{m.group(1)}.rr"{m.group(2)}')
            i += 1
            continue
        m = re.match(r"\.(set|kindset)\s+(\w+)\s*=\s*(.*)$", line)
        if m:
            kind, name, rest = m.groups()
            body, comment = _split_comment(rest)
            comment = comment.strip()
            while i + 1 < len(src) and re.match(r"\s+\|", src[i + 1]):
                i += 1
                b2, c2 = _split_comment(src[i])
                body += " " + b2.strip()
                comment = comment or c2.strip()
            vals = [v.strip() for v in body.split("|") if v.strip()]
            col = "kind" if kind == "kindset" else "v"
            out.append(f".decl {name}({col}: symbol)" + (f"  {comment}" if comment else ""))
            out += facts(name, vals)
            i += 1
            continue
        m = re.match(r"\.native\s+edit\s+(.*)$", line)
        if m:
            out.append(types(".edit " + m.group(1)) + ("" if "//" in m.group(1) else "   // built in"))
            i += 1
            continue
        m = re.match(r"\.native\s+(\w+)(.*)$", line)
        if m:
            out.append(types(".decl " + m.group(1) + m.group(2)))
            pending_inputs.append(m.group(1))
            i += 1
            continue
        if re.match(r"\.(decl|edit|violation|input)\b", line):
            code, comment = _split_comment(line)
            line = types(line[:len(line) - len(comment)]) + comment if comment else types(line)
        out.append(line)
        i += 1
    flush()
    return "\n".join(out)


def _split_comment(s):
    pos = None
    for kind, seg in _offsets(s):
        if kind == "comment":
            pos = seg
            break
    if pos is None:
        return s, ""
    return s[:pos], s[pos:]


def _offsets(s):
    off = 0
    for kind, seg in segments(s):
        yield kind, off
        off += len(seg)


def convert(text):
    text = directives(text)
    t = tokens(text)
    t = [comment_vars(x) if x.startswith("//") else x for x in t]
    return "".join(rewrite(t))


if __name__ == "__main__":
    import sys
    for path in sys.argv[1:]:
        src = open(path).read()
        open(path, "w").write(convert(src))


# ---- groundedness check ----------------------------------------------------------------------------------
# Soufflé rejects a rule with a variable that no positive atom binds. The same check, with this language's
# additions: a pattern `n ~ `…`` binds n and its $X metavariables (as X); match/match_all bind their named groups;
# a fix relation's head and an .edit's head arrive bound; a stop-set argument names a relation, not a variable.

BUILTIN_FN = {"cat", "strlen", "to_number", "to_string", "contains", "substr", "ord", "min", "max", "sum",
              "count", "mean", "join"}
# Built-ins that take relation names as arguments, and at which argument positions.
RELATION_ARG = {"within_stop": (2,), "nearest": (2,), "precedes_stop": (2,), "inside": (2,), "has": (2,),
                "dfs_first": (3, 4), "dfs_first_memo": (3, 4, 5)}
# The text tests (ALIGNMENT.md): built-in constraints like Soufflé's match and contains, pattern first.
TEXT_TESTS = {"glob", "iglob", "prefix", "iprefix", "suffix", "isuffix", "contains", "icontains", "word", "iword"}
STMT_TOK = re.compile(r'"(?:[^"\\]|\\.)*"|`+|//[^\n]*|:-|=>|!=|<=|>=|\d+(?:\.\d+)?(?:[eE][-+]?\d+)?|[A-Za-z_][A-Za-z0-9_]*|@[A-Za-z_]\w*|\d+(?:\.\d+)?(?:[eE][-+]?\d+)?|\S', re.S)


def _lex(s):
    out, i = [], 0
    for kind, seg in segments(s):
        if kind == "comment":
            continue
        if kind in ("str", "tick"):
            out.append(seg)
        else:
            out += STMT_TOK.findall(seg)
    return _drop_json(out)


def _drop_json(toks):
    """The JSON object argument of ast(…) and text_rule(…) is data, not variables."""
    out, i = [], 0
    while i < len(toks):
        if toks[i] in ("ast", "text_rule") and i + 1 < len(toks) and toks[i + 1] == "(":
            j = i + 2
            while j < len(toks) and toks[j] != "{" and toks[j] != ")":
                j += 1
            if j < len(toks) and toks[j] == "{":
                d, k = 0, j
                while k < len(toks):
                    d += toks[k] == "{"
                    d -= toks[k] == "}"
                    if d == 0:
                        break
                    k += 1
                out += toks[i:j] + ['"json"']
                i = k + 1
                continue
        out.append(toks[i])
        i += 1
    return out


def statements(text):
    """(line, text) per rule or fact; directives (with their indented continuation lines) are skipped."""
    lines = text.split("\n")
    out, buf, start, i = [], [], 0, 0
    while i < len(lines):
        ln = lines[i]
        bare = ln.split("//")[0] if not buf else ln
        if not buf and (ln.startswith(".") or ln.startswith("#")):
            i += 1
            while i < len(lines) and lines[i][:1] in (" ", "\t") and lines[i].strip():
                i += 1
            continue
        if not buf and not ln.strip() or (not buf and ln.lstrip().startswith("//")):
            i += 1
            continue
        if not buf:
            start = i + 1
        buf.append(ln)
        joined = "\n".join(buf)
        parts = _split_stmts(joined)
        if parts is not None:
            for p in parts:
                if p.strip():
                    out.append((start, p.strip()))
            buf = []
        i += 1
    return out


def _split_stmts(s):
    """Split complete statements ending in `.`; None if the last one is unfinished."""
    parts, depth, cur, toks = [], 0, [], []
    for kind, seg in segments(s):
        if kind == "comment":
            continue
        if kind in ("str", "tick"):
            cur.append(seg)
            continue
        for ch in seg:
            if ch in "({[":
                depth += 1
            elif ch in ")}]":
                depth -= 1
            if ch == "." and depth == 0:
                parts.append("".join(cur))
                cur = []
                continue
            cur.append(ch)
    rest = "".join(cur)
    if rest.strip():
        return None
    return parts


def _split_top(toks, sep):
    out, cur, d = [], [], 0
    for t in toks:
        if t in ("(", "{", "["):
            d += 1
        elif t in (")", "}", "]"):
            d -= 1
        if t == sep and d == 0:
            out.append(cur)
            cur = []
        else:
            cur.append(t)
    out.append(cur)
    return out


def _is_var(t):
    return bool(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", t)) and t != "_" and t not in ("must", "may") \
        and t not in BITWISE


# Soufflé's word operators: `x band 1` is an expression, not two variables.
BITWISE = {"band", "bor", "bxor", "bnot", "bshl", "bshr", "bshru", "land", "lor", "lxor", "lnot"}


def _vars(toks):
    """Variables in a term/argument list: identifiers not followed by `(`."""
    vs, i = set(), 0
    while i < len(toks):
        t = toks[i]
        nxt = toks[i + 1] if i + 1 < len(toks) else ""
        if t.startswith("`"):
            vs |= {m for m in re.findall(r"\$\$?\$?([A-Za-z_]\w*)", t)}
        elif _is_var(t) and nxt != "(" and not t.startswith("@"):
            vs.add(t)
        i += 1
    return vs


def _atom_args(toks):
    """name(args) -> (name, [arg token lists])."""
    name = toks[0]
    inner = toks[2:-1]
    return name, [a for a in _split_top(inner, ",")] if inner else []


def check_rule(stmt, bound_heads=(), bound_vars=()):
    """Unbound variables of one rule, as a sorted list (empty when grounded)."""
    toks = _lex(stmt)
    if toks and toks[-1] == ".":
        toks = toks[:-1]
    if toks and toks[0] in ("must", "may"):
        toks = toks[1:]
    if ":-" in toks:
        k = toks.index(":-")
        head, rest = toks[:k], toks[k + 1:]
    elif "=>" in toks:
        k = toks.index("=>")
        head, rest = toks[:k], ["=>"] + toks[k + 1:]
    else:
        head, rest = toks, []
    edits = []
    if "=>" in rest:
        k = rest.index("=>")
        rest, edits = rest[:k], rest[k + 1:]
    name = head[0] if head else ""
    bound = set()
    if name in bound_heads:
        bound |= _vars(head[1:])
    bound |= set(bound_vars)
    branches = _split_top(rest, ";")
    if len(branches) > 1:   # a top-level `;` binds looser than `,`: each branch is a rule of its own
        tail = ["=>"] + edits if edits else []
        return sorted(set().union(*(check_rule(" ".join(head + [":-"] + b + tail), bound_heads, bound_vars)
                                    for b in branches)))
    lits = [l for l in _split_top(rest, ",") if l]
    need = set()
    # fixpoint over literals
    pending = list(lits)
    changed = True
    while changed:
        changed = False
        for l in list(pending):
            b, n, done = _literal(l, bound)
            if done:
                pending.remove(l)
                bound |= b
                need |= n
                changed = True
    for l in pending:   # could not be satisfied: everything it needs counts as unbound
        b, n, _ = _literal(l, bound, force=True)
        need |= n
    need |= _vars(_drop_aggregates(head[1:]))
    need |= _vars(edits)
    return sorted(v for v in need if v not in bound)


def _drop_aggregates(toks):
    """Head tokens without aggregates: `count : { … }` (and min/max/sum/join/mean x : { … }) is a value whose body
    binds its own variables, so only the tokens outside it can need a binding."""
    out, i = [], 0
    while i < len(toks):
        if toks[i] in ("count", "min", "max", "sum", "join", "mean") and "{" in toks[i:]:
            j = toks.index("{", i)
            if ":" in toks[i:j]:
                depth = 0
                while j < len(toks):
                    depth += toks[j] == "{"
                    depth -= toks[j] == "}"
                    j += 1
                    if depth == 0:
                        break
                i = j
                continue
        out.append(toks[i])
        i += 1
    return out


def _literal(l, bound, force=False):
    """(binds, needs, ready) for one body literal."""
    if l[0] == "(" and l[-1] == ")" and ";" in l:     # disjunction
        branches = [b for b in _split_top(l[1:-1], ";")]
        sets, needs = [], set()
        for br in branches:
            inner = set(bound)
            for lit in [x for x in _split_top(br, ",") if x]:
                b, n, _ = _literal(lit, inner, force=True)
                inner |= b
                needs |= n - inner
            sets.append(inner - bound)
        binds = set.intersection(*sets) if sets else set()
        return binds, needs, True
    if l[0] == "!":
        rest = l[1:]
        vs = _vars(_strip_relation_args(rest))
        if rest and rest[0] in ("match", "match_all"):
            vs = _vars(rest[3:])
        ready = force or vs <= bound
        return set(), vs, ready
    if "~" in l and any(t.startswith("`") for t in l):    # pattern
        k = l.index("~")
        return _vars(l[:k]) | _vars(l[k + 1:]), set(), True
    if l[0] in TEXT_TESTS and len(l) > 2 and l[1] == "(":    # glob(p, s) …: tests, binding nothing
        vs = _vars(l)
        return set(), vs, force or vs <= bound
    if l[0] in ("match", "match_all") and len(l) > 2:
        args = _split_top(l[2:-1], ",")
        groups = set(re.findall(r"\(\?P?<([A-Za-z_]\w*)>", l[2]))
        subj = _vars(args[1]) if len(args) > 1 else set()
        ready = force or subj <= bound
        return groups, subj, ready
    if "count" in l or "min" in l or "max" in l or "sum" in l or "join" in l:
        # v = agg … : { … }   or   agg : { … } op value
        if ":" in l and "{" in l:
            a = l.index("{")
            e = len(l) - 1 - l[::-1].index("}")
            inner = l[a + 1:e]
            outside = l[:a] + l[e + 1:]
            inner_vars = _inner_needs(inner, bound)
            if len(outside) >= 2 and outside[1] == "=" and _is_var(outside[0]):
                ready = force or inner_vars <= bound
                return {outside[0]}, inner_vars, ready
            vs = _vars([t for t in outside if t not in ("count", "min", "max", "sum", "join", ":")]) | inner_vars
            # `min k : { … }`: k is local
            agg_local = {outside[1]} if outside and outside[0] in ("min", "max", "sum") and len(outside) > 1 else set()
            vs -= agg_local
            return set(), vs, force or vs <= bound
    for op in ("!=", "<=", ">=", "<", ">"):
        if op in l:
            vs = _vars(l)
            return set(), vs, force or vs <= bound
    if "=" in l:
        k = l.index("=")
        left, right = l[:k], l[k + 1:]
        lv, rv = _vars(left), _vars(right)
        if len(left) == 1 and _is_var(left[0]) and rv <= bound:
            return {left[0]}, rv, True
        if len(right) == 1 and _is_var(right[0]) and lv <= bound:
            return {right[0]}, lv, True
        return set(), lv | rv, force or (lv | rv) <= bound
    # positive atom
    if len(l) >= 3 and l[1] == "(":
        return _vars(_strip_relation_args(l)), set(), True
    return set(), _vars(l), True


def _inner_needs(inner, bound):
    """Variables an aggregate body needs from outside: those it uses in negations/comparisons but never binds."""
    lits = [x for x in _split_top(inner, ",") if x]
    b = set(bound)
    needs = set()
    pend = list(lits)
    changed = True
    while changed:
        changed = False
        for lit in list(pend):
            bb, nn, done = _literal(lit, b)
            if done:
                pend.remove(lit)
                b |= bb
                needs |= nn
                changed = True
    for lit in pend:
        _, nn, _ = _literal(lit, b, force=True)
        needs |= nn
    return {v for v in needs if v not in (b - bound)}


def _strip_relation_args(l):
    if len(l) >= 3 and l[0] in RELATION_ARG and l[1] == "(":
        name, args = _atom_args(l)
        drop = {k for k in RELATION_ARG[name] if k < len(args) and len(args[k]) == 1}
        args = [a for j, a in enumerate(args) if j not in drop]
        out = [name, "("]
        for j, a in enumerate(args):
            if j:
                out.append(",")
            out += a
        return out + [")"]
    return l


def fix_relations(text):
    names = set(re.findall(r"^\s+fix\s+(?:safe\s+)?(\w+)", text, re.M))
    names |= set(re.findall(r"^\.edit\s+(\w+)", text, re.M))
    return names


def check_text(text, extra_bound=()):
    heads = fix_relations(text) | set(extra_bound)
    problems = []
    for line, st in statements(text):
        try:
            bad = check_rule(st, heads)
        except Exception as e:      # an unparsable statement is a problem too
            bad = [f"<parse: {e}>"]
        if bad:
            problems.append((line, bad, st))
    return problems


# ---- stratification ------------------------------------------------------------------------------------------------
# Soufflé requires that no relation depends on itself through `!` or an aggregate. Addition 6 relaxes that for
# relations declared `ordered by earlier`: such a use is allowed when its first column is tied to the head's first
# column by earlier(m, n) in the same scope (the body, or the aggregate's body).

AGG = ("count", "min", "max", "sum", "join", "mean")


def ordered_decls(text):
    """{relation: order} for `.decl r(…) ordered by earlier`."""
    return {m.group(1): m.group(2) for m in
            re.finditer(r"^\.decl\s+(\w+)\(.*\)\s+ordered by (earlier)\b", text, re.M)}


def _first_arg(atom):
    """First argument token of `r ( a , … )`, or None."""
    if len(atom) > 2 and atom[1] == "(":
        return atom[2]
    return None


def _tied(scope, order, m, n):
    for i in range(len(scope) - 5):
        if scope[i] == order and scope[i + 1:i + 6] == ["(", m, ",", n, ")"]:
            return True
    return False


def rule_deps(stmt):
    """(head, head_first_arg, [(dep, negative, dep_first_arg, scope_tokens)]) for one rule; None for facts."""
    toks = _lex(stmt)
    if toks and toks[-1] == ".":
        toks = toks[:-1]
    if toks and toks[0] in ("must", "may"):
        toks = toks[1:]
    if not toks or ":-" not in toks:
        return None
    k = toks.index(":-")
    head, rest = toks[:k], toks[k + 1:]
    if "=>" in rest:
        rest = rest[:rest.index("=>")]
    deps = []

    def walk(lits, neg, scope):
        for l in lits:
            if not l:
                continue
            if l[0] == "(" and l[-1] == ")" and ";" in l:
                for br in _split_top(l[1:-1], ";"):
                    walk([x for x in _split_top(br, ",") if x], neg, scope)
                continue
            if l[0] == "!":
                walk([l[1:]], True, scope)
                continue
            if any(t in AGG for t in l) and "{" in l and ":" in l:
                a = l.index("{")
                e = len(l) - 1 - l[::-1].index("}")
                inner = l[a + 1:e]
                walk([x for x in _split_top(inner, ",") if x], True, inner)
                continue
            if len(l) > 1 and l[1] == "(" and _is_var(l[0]):
                deps.append((l[0], neg, _first_arg(l), scope))
                if l[0] in RELATION_ARG:
                    # A relation passed to a built-in is read whole, like a negation: adding a stop node or
                    # an edge can remove answers. It must be complete first, so it is a negative dependency.
                    _, args = _atom_args(l)
                    for k in RELATION_ARG[l[0]]:
                        if k < len(args) and len(args[k]) == 1 and _is_var(args[k][0]):
                            deps.append((args[k][0], True, None, scope))

    walk([x for x in _split_top(rest, ",") if x], False, rest)
    return head[0], _first_arg(head), deps


def check_strata(texts):
    """Problems in one program (a list of file texts): negative cycles not allowed by `ordered by`."""
    ordered, rules = {}, []
    for t in texts:
        ordered.update(ordered_decls(t))
        for line, st in statements(t):
            r = rule_deps(st)
            if r:
                rules.append((line, st, r))
    idb = {r[0] for _, _, r in rules}
    graph = collections.defaultdict(set)
    for _, _, (h, _, deps) in rules:
        for d, _, _, _ in deps:
            if d in idb:
                graph[h].add(d)
    comp = _scc(idb, graph)
    problems = []
    for line, st, (h, n, deps) in rules:
        for d, neg, m, scope in deps:
            if not neg or d not in idb or comp[d] != comp[h]:
                continue
            o = ordered.get(h)
            if o and ordered.get(d) == o and m and n and _tied(scope, o, m, n):
                continue
            problems.append((line, h, d, st))
    return problems


def _scc(nodes, graph):
    """Tarjan's strongly connected components, iterative: {node: component id}."""
    index, low, comp, stack, on, out = {}, {}, {}, [], set(), {}
    counter = [0]
    for root in sorted(nodes):
        if root in index:
            continue
        work = [(root, iter(sorted(graph.get(root, ()))))]
        index[root] = low[root] = counter[0]; counter[0] += 1
        stack.append(root); on.add(root)
        while work:
            v, it = work[-1]
            nxt = next(it, None)
            if nxt is not None:
                if nxt not in index:
                    index[nxt] = low[nxt] = counter[0]; counter[0] += 1
                    stack.append(nxt); on.add(nxt)
                    work.append((nxt, iter(sorted(graph.get(nxt, ())))))
                elif nxt in on:
                    low[v] = min(low[v], index[nxt])
                continue
            work.pop()
            if work:
                low[work[-1][0]] = min(low[work[-1][0]], low[v])
            if low[v] == index[v]:
                while True:
                    w = stack.pop(); on.discard(w); out[w] = v
                    if w == v:
                        break
    return out

