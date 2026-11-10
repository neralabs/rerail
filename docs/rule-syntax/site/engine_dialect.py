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
        for h in re.finditer(r"^" + rel + r"\(", text, re.M):
            end = head_span(text, h.end() - 1)
            args = split_args(text[h.end():end - 1])
            if len(args) == len(names):
                args[0], args[k] = args[k], args[0]
            out.append(text[pos:h.end()] + ", ".join(args) + ")"); pos = end
        out.append(text[pos:]); text = "".join(out)
    return text

# Helper relations a rule declares are its own: prefixed with the rule's code, so no two programs (nor a program and
# a library) share one. Every mention outside string and pattern literals is renamed: `rel(`, and `rel` passed as a
# kind set. `.violation` relations keep their names (they are the findings).
LIT = re.compile(r'"(?:[^"\\]|\\.)*"|`[^`]*`|//[^\n]*')
def rename_outside_literals(text, names, tag):
    pat = re.compile(r"(?<![\w$])(" + "|".join(sorted(map(re.escape, names), key=len, reverse=True)) + r")(?!\w)")
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
    msg = re.sub(r"\{([^{}]*)\}", one, m.group(4))
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
            text += f"\n// main's file condition {json.dumps(fp)}\n{decl}{rel}({args}) :- {raw}({args}), node(rr_d, v0, _, _, _), doc(rr_d, rr_p, _), {', '.join(conds)}.\n"
    return text

# A pattern that is one metavariable, `n ~ \`$X\``, matches every node and names it: `X = n`.
def lone_metavariables(text, base):
    text = re.sub(r"(?<![\w$])([a-z][a-z0-9_]*) ~ `\$_`", r"\1 = \1", text)
    text = re.sub(r"(?<![\w$])([a-z][a-z0-9_]*) ~ `\$([A-Z][A-Z0-9_]*)`", r"\2 = \1", text)
    return text

# Main compares metavariables within one file. A body that joins two nodes by equal text, `text(A, t), text(B, t)`,
# gets `@doc(A) = @doc(B)`: true in main, and it keeps the relation in the file-local part.
def same_document_joins(text, base):
    def same_doc(m):
        body = m.group(0)
        uses = re.findall(r"(?<![\w$])text\((\w+), (\w+)\)", body)
        by_t = {}
        for node, t in uses: by_t.setdefault(t, []).append(node)
        extra = [f"@doc({ns[0]}) = @doc({x})" for t, ns in by_t.items() for x in ns[1:] if x != ns[0]]
        return body[:-1] + (", " + ", ".join(extra) if extra else "") + "."
    text = re.sub(r"(?ms)^[A-Za-z_]\w*\([^\n]*?\) :-.*?\.(?=\s*$)", same_doc, text)
    return text

def engine_text(text, base):
    for p in (anchor_captures, own_helpers, message_braces, lower_options, lone_metavariables,
              same_document_joins):
        text = p(text, base)
    return text
