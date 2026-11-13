"""The engine's dialect of the translation: a mechanical post-pass over translate.py's display text.

translate.py writes the display dialect of the syntax page; the engine's reader takes a slightly different one.
`engine_text(text, base)` rewrites one translated rule (base: the policy id) into what `rerail-engine logic` reads.
"""
import json
import os
import re


def split_args(s):
    out, depth, cur, q = [], 0, "", None
    for ch in s:
        if q:
            cur += ch
            if ch == q: q = None
            continue
        if ch in "\"`": q = ch
        elif ch in "([{": depth += 1
        elif ch in ")]}": depth -= 1
        if ch == "," and depth == 0:
            out.append(cur.strip()); cur = ""
        else:
            cur += ch
    if cur.strip(): out.append(cur.strip())
    return out

def head_span(text, start):
    """Index after the head's closing paren, from the '(' at start."""
    depth, q = 0, None
    for i in range(start, len(text)):
        ch = text[i]
        if q:
            if ch == q: q = None
            continue
        if ch in "\"`": q = ch
        elif ch == "(": depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0: return i + 1
    raise ValueError("unbalanced head")

# `anchor x` (a capture): the finding is at x, so x takes the `at` column and the witness n takes x's column.
def anchor_captures(text, base):
    for m in list(re.finditer(r"^\.violation (\w+)\(([^)]*)\)(.*)\n    anchor (\w+)\n", text, re.M)):
        rel, cols, rest, anchor = m.groups()
        names = [c.split(":")[0].strip() for c in cols.split(",")]
        if anchor not in names:
            continue
        k = names.index(anchor)
        new_cols = ", ".join(f"{n}: node" if i in (0, k) else c.strip() for i, (n, c) in enumerate(zip(names, cols.split(","))))
        text = text.replace(m.group(0), f".violation {rel}({new_cols}){rest}\n", 1)
        # every head of rel at a line start: swap argument 0 and k
        out, pos = [], 0
        # Main reports at the matched node when the anchor capture is unset: where the body may leave it unset,
        # the location is the capture when it is a node, else the node.
        for h in re.finditer(r"^" + rel + r"\(", text, re.M):
            end = head_span(text, h.end() - 1)
            args = split_args(text[h.end():end - 1])
            tail = ""
            if len(args) == len(names):
                stop = rule_end(text, end)
                if re.search(r"\bunset\(" + re.escape(args[k]) + r"\)", text[end:stop]):
                    x, at = args[k], args[0]
                    args[0], args[k] = "rr_at", x
                    tail = (text[end:stop], f",\n    (unset({x}), rr_at = {at} ; !unset({x}), rr_at = {x})")
                else:
                    args[0], args[k] = args[k], args[0]
            out.append(text[pos:h.end()] + ", ".join(args) + ")")
            if tail:
                out.append(tail[0] + tail[1]); pos = stop
            else:
                pos = end
        out.append(text[pos:]); text = "".join(out)
    return text


def rule_end(text, start):
    """Index of the `.` that ends the rule whose body starts at start (outside strings, ticks and braces)."""
    q, depth, esc = None, 0, False
    for i in range(start, len(text)):
        ch = text[i]
        if q:
            if esc:
                esc = False
            elif ch == "\\" and q == '"':
                esc = True
            elif ch == q:
                q = None
            continue
        if ch in "\"`": q = ch
        elif ch in "({[": depth += 1
        elif ch in ")}]": depth -= 1
        elif ch == "." and depth == 0 and (i + 1 == len(text) or text[i + 1] in "\n "):
            return i
    return len(text)

# Helper relations a rule declares are its own: prefixed with the rule's code, so no two programs (nor a program and
# a library) share one. Every mention outside string and pattern literals is renamed: `rel(`, and `rel` passed as a
# kind set. `.violation` relations keep their names (they are the findings).
# Directive names (`.scope`, `.policy`, `.decl`, …) at a line start are never renamed either.
LIT = re.compile(r'"(?:[^"\\]|\\.)*"|`[^`]*`|//[^\n]*|(?m:^\.\w+)')
def rename_outside_literals(text, names, tag):
    pat = re.compile(r"(?<![\w$])(?<!: )(" + "|".join(sorted(map(re.escape, names), key=len, reverse=True)) + r")(?!\w)")
    out, pos = [], 0
    for m in LIT.finditer(text):
        out.append(pat.sub(lambda x: tag + "_" + x.group(1), text[pos:m.start()])); out.append(m.group(0)); pos = m.end()
    out.append(pat.sub(lambda x: tag + "_" + x.group(1), text[pos:]))
    return "".join(out)
def own_helpers(text, base):
    tag = re.sub(r"[^a-z0-9]+", "_", base.split("-")[1])
    local = set(re.findall(r"^\.decl (\w+)\(", text, re.M))
    KEYWORDS = {"suppress", "fix", "code", "tag", "options", "anchor", "each", "at", "in", "over", "count", "sum",
                "min", "max", "mean", "safe", "noqa", "nosec", "ruff"}
    plain, keyword = local - KEYWORDS, local & KEYWORDS
    if plain:
        text = rename_outside_literals(text, plain, tag)
    for k in keyword:
        text = re.sub(r"(?<![\w$])" + k + r"(?=\()", tag + "_" + k, text)
    return text

# A message's `{x}` names a column (or `{x.line}`, `{x.name}`); any other brace is the text's own: `{{`, `}}`.
def escape_message(m):
    cols = [c.split(":")[0].strip() for c in m.group(2).split(",")]
    def one(b):
        key = b.group(1).split(".")[0]
        return b.group(0) if key in cols else "{{" + b.group(1) + "}}"
    # a column reference stays; every other brace, nested ones included, is doubled
    msg = re.sub(r"\{([^{}]*)\}|([{}])", lambda b: one(b) if b.group(2) is None else b.group(2) * 2, m.group(4))
    return f".violation {m.group(1)}({m.group(2)}){m.group(3)}{msg}"
def message_braces(text, base):
    text = re.sub(r'^\.violation (\w+)\(([^)]*)\)((?: code "(?:[^"\\]|\\.)*")? )("(?:[^"\\]|\\.)*")', escape_message, text, flags=re.M)
    return text

# `options` (main's clause conditions), which a `logic` run does not read, as rules:
#   when.pythonVersion: decided against main's default, Python 3.10 (scope/options.rs DEFAULT_PYTHON_MINOR):
#     a violation whose bounds exclude it gets no rules;
#   file.path.regex / notRegex: the rules derive REL_raw, and REL is REL_raw on the documents whose path matches.
DEFAULT = (3, 10)
def ver(s): return tuple(int(x) for x in s.split("."))
def lower_options(text, base):
    for m in list(re.finditer(r"^\.violation (\w+)\(([^)]*)\)[^\n]*\n((?:    [^\n]*\n)*?)    options (\{[^\n]*\})\n", text, re.M)):
        rel, cols, opts = m.group(1), m.group(2), json.loads(m.group(4))
        text = text.replace("    options " + m.group(4) + "\n", "", 1)
        v = (opts.get("when") or {}).get("pythonVersion") or {}
        off = ("min" in v and ver(v["min"]) > DEFAULT) or ("max" in v and ver(v["max"]) < DEFAULT)
        fp = ((opts.get("file") or {}).get("path")) or {}
        if not off and not fp:
            continue
        raw = rel + "_raw"
        text = re.sub(r"^" + rel + r"\(", raw + "(", text, flags=re.M)
        n = len(cols.split(","))
        args = ", ".join(f"v{i}" for i in range(n))
        decl = f".decl {raw}({cols})\n"
        if off:
            text += f"\n// pythonVersion {json.dumps(v)} excludes Python 3.10, main's default: no findings.\n{decl}"
        else:
            conds = []
            if "regex" in fp: conds.append(f'match({json.dumps(fp["regex"])}, rr_p)')
            if "notRegex" in fp: conds.append(f'!match({json.dumps(fp["notRegex"])}, rr_p)')
            text += f"\n// main's file condition {json.dumps(fp)}\n{decl}{rel}({args}) :- {raw}({args}), rr_d = @doc(v0), doc(rr_d, rr_p, _), {', '.join(conds)}.\n"
    return text

# A pattern that is one metavariable, `n ~ \`$X\``, matches every node and names it: `X = n`.
def lone_metavariables(text, base):
    # The pattern grounded the node; `node(…)` keeps it grounded where nothing else in the body does.
    text = re.sub(r"(?<![\w$])([a-z][a-z0-9_]*) ~ `\$_`", r"node(_, \1, _, _, _)", text)
    text = re.sub(r"(?<![\w$])([a-z][a-z0-9_]*) ~ `\$([A-Z][A-Z0-9_]*)`", r"node(_, \1, _, _, _), \2 = \1", text)
    return text

# Main compares metavariables within one file. A body that joins two nodes by equal text, `text(A, t), text(B, t)`,
# gets `@doc(A) = @doc(B)`: true in main, and it keeps the relation in the file-local part.
# A pair with a node local to an aggregate `{ … }` gets its constraint inside that aggregate, where the node is bound.
TEXT_USE = re.compile(r"(?<![\w$])text\((\w+), (\w+)\)")


def aggregate_spans(body):
    """Scopes a variable can be local to, as (start, end): each `{ … }` aggregate body, and each branch of a
    `( a ; b )` disjunction (end: the `;` or `)` after it). Outside string and pattern literals."""
    spans, stack, q = [], [], None
    for i, ch in enumerate(body):
        if q:
            if ch == q and body[i - 1] != "\\":
                q = None
            continue
        if ch in "\"`":
            q = ch
        elif ch in "{(":
            stack.append([ch, i, i, False])     # kind, open, branch start, has `;`
        elif ch == ";" and stack and stack[-1][0] == "(":
            spans.append((stack[-1][2], i))
            stack[-1][2], stack[-1][3] = i, True
        elif ch == "}" and stack:
            spans.append((stack.pop()[1], i))
        elif ch == ")" and stack:
            kind, start, branch, disj = stack.pop()
            if disj:
                spans.append((branch, i))
    return spans


def same_document_joins(text, base):
    def same_doc(m):
        body = m.group(0)
        if "parsed(" in body:   # a node of a parsed string lives in the string's own (guest) document
            return body
        spans = aggregate_spans(body)

        def scope(pos):     # the innermost aggregate holding pos, or None for the rule body
            inner = [(s, e) for s, e in spans if s < pos < e]
            return min(inner, key=lambda x: x[1] - x[0]) if inner else None
        by_t = {}
        # a variable bound to a string (`X = @replace_all(…)`, `X = "…"`) is not a node: it has no document
        string = lambda v: re.search(r"(?<![\w$])" + re.escape(v) + r' = [@"]', body)
        for u in TEXT_USE.finditer(body):
            if string(u.group(1)):
                continue
            by_t.setdefault(u.group(2), []).append((u.group(1), scope(u.start())))
        # one text variable's uses lie in nested scopes (it is bound in the outermost one): each pair's constraint
        # goes where the inner of its two nodes is bound
        adds = {}
        for t, uses in by_t.items():
            first = uses[0][0]
            for node, sc in uses[1:]:
                if node == first:
                    continue
                s0 = uses[0][1]
                deeper = sc if sc is not None and (s0 is None or sc[1] - sc[0] < s0[1] - s0[0]) else s0
                c = f"@doc({first}) = @doc({node})"
                if c not in adds.setdefault(deeper, []):
                    adds[deeper].append(c)
        for sc in sorted((k for k in adds if k is not None), key=lambda x: -x[1]):
            body = body[:sc[1]].rstrip() + ", " + ", ".join(adds[sc]) + " " + body[sc[1]:]
        extra = adds.get(None, [])
        return body[:-1] + (", " + ", ".join(extra) if extra else "") + "."
    text = re.sub(r"(?ms)^[A-Za-z_]\w*\([^\n]*?\) :-.*?\.(?=\s*$)", same_doc, text)
    return text

# `anchor file.start`: main reports at the file's start (line 1). The reader has no such anchor; the finding is at
# the witness, which for such a rule is the module (it starts the file).
def file_anchor(text, base):
    return text.replace("    anchor file.start\n", "    // anchor file.start: at the witness, the module, which starts the file\n")


# ast-grep leaves a metavariable empty where the match does not bind it; translate.py writes that `unset(X)` before
# its simplify pass. Inlining and demand rewrites after it can leave a rule where X is bound in some branches of a
# disjunction only, or (a column the relation passes out) nowhere in the body: the engine refuses it as ungrounded.
# Checked again on the final text: a branch that does not bind X gets `unset(X)`; a head column the body never
# names gets `unset(X)` at the end.
def unset_unbound(text, base):
    from souffle import check_rule
    from translate import _split_top_text, head_span_ok
    binds = lambda body, x: x not in check_rule(f"rr_probe({x}) :- {body}.")
    out, pos = [], 0
    for m in re.finditer(r"(?m)^([A-Za-z_]\w*)\(", text):
        if m.start() < pos:
            continue
        try:
            end = head_span(text, m.end() - 1)
        except ValueError:
            continue
        if not text.startswith(" :- ", end):
            continue
        stop = rule_end(text, end)
        rule = text[m.start():stop + 1]
        try:
            bad = [x for x in check_rule(rule) if re.fullmatch(r"[A-Z][A-Z0-9_]*", x)]
        except Exception:
            bad = []
        if not bad:
            continue
        head, body = text[m.start():end], text[end + 4:stop]
        lits = _split_top_text(body, ",")
        changed = False
        for x in bad:
            done = False
            for i, l in enumerate(lits):
                t = l.strip()
                if not (t.startswith("(") and t.endswith(")") and head_span_ok(t)):
                    continue
                branches = _split_top_text(t[1:-1], ";")
                if len(branches) < 2:
                    continue
                ok = [binds(b, x) for b in branches]
                if any(ok) and not all(ok):
                    lits[i] = " " + "(" + " ; ".join(b.strip() if k else f"{b.strip()}, unset({x})"
                                                   for b, k in zip(branches, ok)) + ")"
                    done = changed = True
                    break
            if not done and not re.search(r"(?<![\w$])" + x + r"\b", re.sub(r'"(?:[^"\\]|\\.)*"', "", body)):
                lits.append(f" unset({x})")
                changed = True
        if changed:
            out.append(text[pos:m.start()] + head + " :- " + ",".join(lits).strip() + ".")
            pos = stop + 1
    out.append(text[pos:])
    return "".join(out)


# A JSON `compare` of two metavariables is written `t = A, t = B` (t a fresh variable). Where A or B is computed
# later in the rule (a `transform`'s literals end it), t is bound by neither where it stands, and the reader
# refuses t as ungrounded: `A = B` says the same and needs only that both be bound somewhere in the rule.
def compare_pairs(text, base):
    pat = re.compile(r"(?<![\w$])([a-z]\w*) = ([A-Z][A-Z0-9_]*), \1 = ([A-Z][A-Z0-9_]*)(?!\w)")

    def rule(r):    # one rule (its text up to the `.` that ends it): t used only in the pair
        body = r.group(0)
        def one(m):
            uses = len(re.findall(r"(?<![\w$])" + re.escape(m.group(1)) + r"(?!\w)", body))
            return f"{m.group(2)} = {m.group(3)}" if uses == 2 else m.group(0)
        return pat.sub(one, body)
    return re.sub(r"(?ms)^[A-Za-z_]\w*\([^\n]*?\) :-.*?\.(?=\s*$)", rule, text)


# `not: {compare: {left: X, right: X}}`, main's test that metavariable X is unbound (a compare of an unbound
# metavariable is false), translates to `count : { text(X, t), text(X, t) } = 0`; where the caller passes X in,
# the reader refuses X as ungrounded. It says X is unset: `unset(X)`.
def unbound_tests(text, base):
    return re.sub(r"count : \{ text\(([A-Z][A-Z0-9_]*), (\w+)\), text\(\1, \2\) \} = 0", r"unset(\1)", text)


# An axis with a stop set inside an aggregate, `count : { within_stop(c, p, S), … }`: the engine's lowering of
# the aggregate's context gives the set's name a head column, which the native engine declines (a strict run
# aborts). The axis over that set becomes a relation of its own, read inside the aggregate.
def stop_axes_in_aggregates(text, base):
    made = {}
    def inner(m):
        def one(a):
            axis, x, y, rel = a.groups()
            name = f"{rel}_{axis.split('_')[0]}"
            made[name] = (axis, rel)
            return f"{name}({x}, {y})"
        return re.sub(r"(?<![\w$])(within_stop|precedes_stop)\((\w+), (\w+), ([a-z]\w*)\)", one, m.group(0))
    out, pos = [], 0
    for m in re.finditer(r"(?:count|sum|min|max|mean|join)(?: \w+)? : \{", text):
        if m.start() < pos:
            continue
        depth, i = 0, m.end() - 1
        while i < len(text):
            depth += text[i] == "{"
            depth -= text[i] == "}"
            if depth == 0:
                break
            i += 1
        out.append(text[pos:m.start()])
        out.append(inner(re.match(r"(?s).*", text[m.start():i + 1])))
        pos = i + 1
    out.append(text[pos:])
    text = "".join(out)
    for name, (axis, rel) in made.items():
        cols = "n: node, a: node" if axis == "within_stop" else "a: node, b: node"
        text += (f"\n// {name}: {axis} over the stop set {rel}, read inside an aggregate\n"
                 f".decl {name}({cols})\n{name}(x, y) :- {axis}(x, y, {rel}).\n")
    return text


def engine_text(text, base):
    for p in (stop_axes_in_aggregates, anchor_captures, file_anchor, own_helpers, message_braces, lower_options, lone_metavariables,
              same_document_joins, compare_pairs, unbound_tests, unset_unbound):
        text = p(text, base)
    return text
