#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Port a data-layer .rr program (main file + its part files, over prelude.rr) to programs over core.rr.

usage: data_layer_rr_port.py <programs dir> <main stem> <out dir>

Each file stays a program of its own (the parts' relations are their own), each with the main file's `.policy`
line (so the corpus includes core.rr and the matchers' `$option`s resolve) and a shim for what prelude.rr defines
that core.rr does not. The rewrites (decision 0107):
- prelude's `doc(d, path)` is core's `doc(d, path, module)`;
- `@within(n, a) = 1` (`= 0`) is core's `within(n, a)` (`!within`); `@depth(a)` is `ast_depth(a, k)`, a fresh `k`;
- `@rx_matches(s, re) = 1` (`= 0`, `!= 1`) is the text test `@rx(s, re) = 1` (`= 0`);
- SQLite's functions are the catalogue's `sql_` forms (`@substr` 1-based, `@instr`, `@trim`, …); a `@concat` or
  `@sub` of more than two is nested.
"""
import re, sys, pathlib
src, stem, out = pathlib.Path(sys.argv[1]), sys.argv[2], pathlib.Path(sys.argv[3])
out.mkdir(parents=True, exist_ok=True)
main = src / f"{stem}.rr"
files = [main] + sorted(f for f in src.glob(f"{stem}.*.rr") if "unwidened" not in f.name)
policy = next(l for l in main.read_text().splitlines() if l.startswith(".policy"))
shim = """
// prelude.rr's relations core.rr does not hold, over core's (the data-layer port).
.decl start(n: node, s: number)
start(n, s) :- node(_, n, _, s, _).
// The tree builtins @class_path and @class_prefix as relations (decision 0107): the `name` texts of a class and its
// class ancestors, outermost first, joined with "."; a node's enclosing classes' path and ".", or "".
.decl dl_class_kind(k: symbol)
dl_class_kind("class_definition").
.decl dl_in_class(n: node)
dl_in_class(n) :- nearest(n, _, dl_class_kind).
.decl dl_class_path(c: node, s: symbol)
dl_class_path(c, t) :- kind(c, "class_definition"), field(c, "name", m), text(m, t), !dl_in_class(c).
dl_class_path(c, s) :- kind(c, "class_definition"), field(c, "name", m), text(m, t), nearest(c, a, dl_class_kind),
    dl_class_path(a, p), s = @concat(@concat(p, "."), t).
.decl dl_class_prefix(n: node, s: symbol)
dl_class_prefix(n, s) :- nearest(n, c, dl_class_kind), dl_class_path(c, p), s = @concat(p, ".").
dl_class_prefix(n, "") :- kind(n, "function_definition"), !dl_in_class(n).
// The tree builtin @qualname as a relation: a frame's (function or class) qualified name, its parent frame's, then
// ".<locals>." where the frame is a function under a function frame, else ".", then its `name` text (the data-layer
// oracle's `contexts`); d counts the function frames, the frame's own included.
.decl dl_frame_kind(k: symbol)
dl_frame_kind("function_definition").
dl_frame_kind("class_definition").
.decl dl_frame(f: node, t: symbol, fn: number)
dl_frame(f, t, 1) :- kind(f, "function_definition"), field(f, "name", m), text(m, t).
dl_frame(f, t, 0) :- kind(f, "class_definition"), field(f, "name", m), text(m, t).
// A frame's parent frame, from the frames: read from the parent, `nearest` would visit every node under it.
.decl dl_frame_parent(f: node, a: node)
dl_frame_parent(f, a) :- dl_frame(f, _, _), nearest(f, a, dl_frame_kind).
.decl dl_in_frame(n: node)
dl_in_frame(n) :- dl_frame_parent(n, _).
.decl dl_qual(f: node, q: symbol, d: number)
dl_qual(f, t, k) :- dl_frame(f, t, k), !dl_in_frame(f).
dl_qual(f, s, d) :- dl_frame(f, t, 1), dl_frame_parent(f, a), dl_qual(a, p, k), k > 0, d = k + 1,
    s = @concat(@concat(p, ".<locals>."), t).
dl_qual(f, s, d) :- dl_frame(f, t, x), dl_frame_parent(f, a), dl_qual(a, p, k), (x = 0; k = 0), d = k + x,
    s = @concat(@concat(p, "."), t).
.decl dl_qualname(f: node, q: symbol)
dl_qualname(f, q) :- dl_qual(f, q, _).
// @qualdepth: the function frames on the frame's chain, its own included.
.decl dl_qualdepth(f: node, d: number)
dl_qualdepth(f, d) :- dl_qual(f, _, d).
"""

def call_end(t, i):
    """The index after the `)` closing the call whose `(` is at i-1."""
    d, q = 1, False
    while d:
        c = t[i]
        if q:
            if c == "\\": i += 1
            elif c == '"': q = False
        elif c == '"': q = True
        elif c == "(": d += 1
        elif c == ")": d -= 1
        i += 1
    return i

def args_of(inner):
    out, d, q, cur = [], 0, False, ""
    i = 0
    while i < len(inner):
        c = inner[i]
        if q:
            cur += c
            if c == "\\": cur += inner[i + 1]; i += 1
            elif c == '"': q = False
        elif c == '"': q = True; cur += c
        elif c in "(": d += 1; cur += c
        elif c in ")": d -= 1; cur += c
        elif c == "," and d == 0: out.append(cur.strip()); cur = ""
        else: cur += c
        i += 1
    if cur.strip(): out.append(cur.strip())
    return out

SQL = {("substr", 3): "sql_substr", ("substr", 2): "sql_substr2", ("substr_bytes", 3): "sql_substr_bytes", ("instr", 2): "sql_instr", ("length", 1): "sql_length",
       ("trim", 1): "sql_trim", ("trim", 2): "sql_trim2", ("ltrim", 1): "sql_ltrim", ("ltrim", 2): "sql_ltrim2",
       ("upper", 1): "sql_upper", ("lower", 1): "sql_lower", ("replace", 3): "sql_replace", ("like", 2): "sql_like",
       ("cast_int", 1): "sql_cast_int"}

def rewrite_calls(t):
    """Renames and nests calls, innermost first."""
    while True:
        changed = False
        for m in re.finditer(r"@([a-z_]+)\(", t):
            name, end = m.group(1), call_end(t, m.end())
            args = args_of(t[m.end():end - 1])
            new = None
            if (name, len(args)) in SQL:
                new = f"@{SQL[(name, len(args))]}({', '.join(args)})"
            elif name in ("concat", "sub", "add") and len(args) > 2:
                acc = f"@{name}({args[0]}, {args[1]})"
                for a in args[2:]:
                    acc = f"@{name}({acc}, {a})"
                new = acc
            if new and new != t[m.start():end]:
                t = t[:m.start()] + new + t[end:]
                changed = True
                break
        if not changed:
            return t

def rewrite_rule(rule, fresh):
    # @within(n, a) = 1 / = 0 / != 1
    def within(m):
        neg = (m.group(3) == "0") != (m.group(2) == "!=")
        return f"{'!' if neg else ''}within({m.group(1)})"
    rule = re.sub(r"@within\(([^()]*)\)\s*(!=|=)\s*([01])\b", within, rule)
    # @rx_matches(s, re) = 1 / = 0 / != 1 / != 0
    out, i = "", 0
    for m in re.finditer(r"@rx_matches\(", rule):
        end = call_end(rule, m.end())
        tail = re.match(r"\s*(!=|=)\s*([01])\b", rule[end:])
        if not tail:
            continue
        holds = (tail.group(2) == "1") != (tail.group(1) == "!=")
        out += rule[i:m.start()] + f"@rx({rule[m.end():end - 1]}) = {1 if holds else 0}"
        i = end + tail.end()
    rule = out + rule[i:]
    # Tree builtins anywhere: a fresh variable from a relation (core's, or the shim's).
    extra = []
    lifted = {"depth": "ast_depth", "line": "line", "class_path": "dl_class_path", "class_prefix": "dl_class_prefix", "qualname": "dl_qualname", "qualdepth": "dl_qualdepth"}
    while (m := re.search(r"@(depth|line|class_path|class_prefix|qualname|qualdepth)\(([^()]*)\)", rule)):
        fresh[0] += 1
        v = f"dl_{m.group(1)}{fresh[0]}"
        extra.append(f"{lifted[m.group(1)]}({m.group(2)}, {v})")
        rule = rule[:m.start()] + v + rule[m.end():]
    # @printf over %s and %d: a @concat of its parts.
    while (m := re.search(r"@printf\(", rule)):
        end = call_end(rule, m.end())
        args = args_of(rule[m.end():end - 1])
        fmt = args[0]
        assert fmt.startswith('"') and fmt.endswith('"'), fmt
        pieces = re.split(r"(%[sd]|%0[1-9][0-9]*d)", fmt[1:-1])
        parts, k = [], 1
        for piece in pieces:
            if piece in ("%s", "%d"):
                parts.append(args[k] if piece == "%s" else f"@rr_to_string({args[k]})"); k += 1
            elif re.fullmatch(r"%0[1-9][0-9]*d", piece):
                # Zero-padded to width w (a non-negative number below 10^w, as the ordinals it formats are): the last
                # w characters of w zeros and the number, as SQLite's substr takes a negative start.
                w = int(piece[2:-1])
                parts.append(f'@sql_substr(@concat("{"0" * w}", @rr_to_string({args[k]})), -{w}, {w})'); k += 1
            elif piece:
                assert "%" not in piece, fmt
                parts.append(f'"{piece}"')
        acc = parts[0] if parts else '""'
        for q in parts[1:]:
            acc = f"@concat({acc}, {q})"
        rule = rule[:m.start()] + acc + rule[end:]
    # @id(n): the node's id as text (an identity in finding keys).
    rule = re.sub(r"@id\(([^()]*)\)", r"@rr_to_string(\1)", rule)
    if extra:
        body = rule.rstrip()
        assert body.endswith("."), body[-80:]
        rule = body[:-1] + ", " + ", ".join(extra) + "." + rule[len(body):]
    return rewrite_calls(rule)

def statements(t):
    """The program's text split after each line ending a statement (`.` at the end, outside a string)."""
    out, cur = [], ""
    for line in t.splitlines(keepends=True):
        cur += line
        s = line.rstrip()
        if s.endswith(".") and not s.lstrip().startswith("//"):
            out.append(cur); cur = ""
    out.append(cur)
    return out

for f in files:
    t = f.read_text()
    # A node's identity text `kind:start:end` (legacy's node_id): kinds are their own domain here, not text, and the
    # identity is no part of a finding's key, so the node's own id stands for it.
    t = t.replace('node_id(n, id) :- nodes(n), node(_, n, k, s, e), id = @printf("%s:%d:%d", k, s, e).',
                  'node_id(n, id) :- nodes(n), id = @rr_to_string(n).')
    t = re.sub(r"\bdoc\(\s*([^,()]+?)\s*,\s*([^,()]+?|\"[^\"]*\")\s*\)", r"doc(\1, \2, _)", t)
    # `in_list("NAME", v)`, one relation over every list, holds kinds in one list and field names in another: its
    # column cannot be both. Each list becomes its own relation, `in_list_NAME(v)`.
    if re.search(r"^\.decl in_list\(list: symbol, v: symbol\)", t, re.M):
        names = sorted(set(re.findall(r'\bin_list\("(\w+)",', t)))
        assert not re.search(r'\bin_list\((?!")(?!list: symbol)', t), "a list named by a variable"
        t = re.sub(r'\bin_list\("(\w+)",\s*', r"in_list_\1(", t)
        t = re.sub(r"^\.decl in_list\(list: symbol, v: symbol\)\n", "".join(f".decl in_list_{n}(v: symbol)\n" for n in names), t, flags=re.M)
    fresh = [0]
    t = "".join(rewrite_rule(s, fresh) if ":-" in s else s for s in statements(t))
    if f != main:
        t = policy + "\n" + t
    (out / f.name).write_text(t + shim)
print(f"{out}: {len(files)} files")
