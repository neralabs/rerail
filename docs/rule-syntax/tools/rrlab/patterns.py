"""Compile n ~ `pattern` atoms to structural Soufflé rules (README, "Patterns").

The pattern is parsed with the policy language's tree-sitter grammar (rrlab-facts tree). Each node of the pattern
becomes kind / field / child / op constraints on a fresh variable; a named leaf compares its text. Metavariables:
  $X      binds the Datalog variable X to the node there; a second $X compares source text with the first
  $_      any node, binds nothing
  $$$X    any run of nodes in a list, binds nothing (lists are for fix templates); $$$ alone the same
  at "k"  the pattern's node of kind k is the match (ast-grep's selector), not the pattern's root
Outside JavaScript-family grammars `$` is not an identifier character, so metavariables are written µX before
parsing, as ast-grep does.
Not covered (reported): metavariables inside strings or as operators, more than one $$$ in one list, patterns
that do not parse.
"""
import json, os, re, subprocess

PUNCT = {"(", ")", "[", "]", "{", "}", ",", ";", ":", ".", '"', "'", "`"}
JS = {"javascript", "typescript", "tsx"}
GRAMMAR = {"typescript": "typescript", "javascript": "javascript", "tsx": "tsx", "python": "python", "rust": "rust",
           "php": "php", "sql": "sql", "vue": "vue"}


class Compiler:
    def __init__(self, facts_bin):
        self.bin = facts_bin
        self.cache = {}

    def tree(self, lang, src):
        r = subprocess.run([self.bin, "tree", GRAMMAR.get(lang, lang)], input=src.encode(), capture_output=True)
        if r.returncode:
            raise RuntimeError(r.stderr.decode())
        return json.loads(r.stdout)

    def __call__(self, prog, lang, tick, at):
        key = (lang, tick, at)
        if key in self.cache:
            return self.cache[key]
        fence = len(tick) - len(tick.lstrip("`"))
        src = tick[fence:-fence]
        sigil = "$" if lang in JS else "µ"
        code = src if sigil == "$" else src.replace("$", "µ")
        if lang == "php" and not code.lstrip().startswith("<?"):
            code = "<?php " + code
        t = self.tree(lang, code)
        if has_error(t) and lang in WRAP:
            # a snippet that is no whole file (a Rust expression, PHP without <?php): parse it inside a wrapper
            pre, post = WRAP[lang]
            w = self.tree(lang, pre + code + post)
            if not has_error(w):
                t = w
        notes = []
        if has_error(t):
            notes.append("does not parse cleanly")
        meta = re.compile(r"^" + re.escape(sigil) + r"(\$\$|" + re.escape(sigil * 2) + r")?([A-Z_][A-Z0-9_]*)?$")

        def strip(n):
            return n["text"].strip().rstrip(";").strip()

        def is_meta(n):
            m = meta.match(strip(n))
            return n["named"] and m is not None and (m.group(2) is not None or m.group(1))

        def meta_of(n):
            m = meta.match(strip(n))
            return ("list" if m.group(1) else "one", m.group(2) or "_")

        def named(n):
            return [c for c in n["children"] if c["named"] and not c["extra"]]

        # the match target: a selected kind, else the deepest node spanning the whole pattern
        root = t
        if at:
            found = find_kind(t, at)
            if found is None:
                raise RuntimeError(f"pattern {src!r} has no node of kind {at}")
            root = found
        else:
            whole = re.sub(r"^<\?php\s+", "", code.strip()).rstrip(";").strip()
            root = find_text(t, whole, strip) or t
            while True:
                ch = named(root)
                if len(ch) == 1 and strip(ch[0]) == whole and not is_meta(root):
                    root = ch[0]
                else:
                    break
        lits, bound, seen = [], [], {}
        counter = [0]

        def fresh():
            counter[0] += 1
            return f"rrp{counter[0]}"

        def bind(n, v):
            kind, name = meta_of(n)
            if kind == "list" or name == "_":
                return
            if name in seen:
                a, b = fresh(), fresh()
                lits.append(f"text({v}, {a}), text({seen[name]}, {b}), {a} = {b}")
            else:
                seen[name] = v
                bound.append(name)

        def emit(n, v):
            if is_meta(n):
                bind(n, v)
                lits.append(f"kind({v}, _)")
                return
            lits.append(f'kind({v}, "{n["kind"]}")')
            ch = named(n)
            if not n["children"]:
                if sigil in n["text"]:
                    notes.append(f"metavariable inside a token: {n['text']!r}")
                lits.append(f"text({v}, {q(n['text'])})")
                return
            for c in n["children"]:
                if not c["named"] and c["kind"] not in PUNCT and c["text"].strip():
                    lits.append(f'op({v}, {q(c["kind"])})')
            lists = [i for i, c in enumerate(ch) if is_meta(c) and meta_of(c)[0] == "list"]
            if len(lists) > 1:
                notes.append(f"{len(lists)} $$$ lists in one {n['kind']}: positions after the first are loose")
            total = None
            if lists and any(i > lists[0] for i in range(len(ch))):
                total = fresh()
                lits.append(f"{total} = count : {{ child({v}, _, _) }}")
            elif not lists:
                # exactly these named children, as ast-grep matches (comments in the target are not counted)
                lits.append(f'count : {{ child({v}, rrq, _), !kind(rrq, "comment") }} = {len(ch)}')
            for i, c in enumerate(ch):
                if is_meta(c) and meta_of(c)[0] == "list":
                    continue
                cv = seen_or_fresh(c)
                if lists and i > lists[0]:
                    if len(lists) > 1:
                        pos = None
                    else:
                        pos = f"{total} - {len(ch) - i}"
                else:
                    pos = str(i)
                if c["field"] is not None:
                    lits.append(f'field({v}, "{c["field"]}", {cv})')
                if pos is not None:
                    lits.append(f"child({v}, {cv}, {pos})")
                elif c["field"] is None:
                    lits.append(f"child({v}, {cv}, _)")
                emit(c, cv)

        def seen_or_fresh(c):
            if is_meta(c):
                kind, name = meta_of(c)
                if kind == "one" and name != "_" and name not in seen:
                    return name
            return fresh()

        if is_meta(root):
            rv = fresh()
            bind(root, rv)
            lits.append(f"kind({rv}, _)")
            if bound and bound[0] == meta_of(root)[1]:
                lits.append(f"{bound[0]} = {rv}")
        else:
            rv = fresh()
            emit(root, rv)
        # a variable that is a metavariable's own name is bound by the child() literal that introduced it
        prog.n += 1
        name = f"rr_pat{prog.n}"
        cols = ["root"] + bound
        prog.extra.append(f".decl {name}({', '.join(f'{c}: number' for c in cols)})  // {src}")
        prog.extra.append(f"{name}({', '.join([rv] + bound)}) :- {', '.join(dict.fromkeys(lits))}.")
        prog.pattern_report.append((src, at, "ok" if not notes else "partial: " + "; ".join(notes)))
        self.cache[key] = (name, bound)
        return name, bound


WRAP = {"rust": ("fn rrlab_pattern() { ", " }")}


def find_text(t, whole, strip):
    """The outermost named node whose text is the whole pattern."""
    if t["named"] and strip(t) == whole:
        return t
    for c in t["children"]:
        f = find_text(c, whole, strip)
        if f:
            return f
    return None


def q(s):
    """A Soufflé string literal."""
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t") + '"'


def has_error(t):
    return t.get("error") or any(has_error(c) for c in t["children"])


def find_kind(t, k):
    if t["kind"] == k:
        return t
    for c in t["children"]:
        f = find_kind(c, k)
        if f:
            return f
    return None
