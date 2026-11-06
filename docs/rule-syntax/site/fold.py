"""Fold families of copy-pasted relations into one relation and a table. Runs after simplify.py's merge and
inline, before metavariable sharing.

The pack JSON repeats one util per module or per API: `m:assert:call`, `m:stream:call`, … are the same
rules with a different module name, and `v:assert` calls `m:assert:esm-whole` where `v:stream` calls
`m:stream:esm-whole`. Translated one by one, each copy is its own relation. Here, relations whose rules are
identical up to string constants and up to which member of another family they call become one relation
with a key column, plus a table with one row per former relation:

    m_call(n, q) :- m_call_case(q, s1), kind(n, "call_expression"), …, glob(s1, v7), …
    m_call_case("assert", "{node:,}assert"). m_call_case("stream", "{node:,}stream"). …

and every call `m_assert_call(x)` becomes `m_call(x, "assert")`.

Meaning: the folded relation's rows for key k are exactly the old relation's rows, because its rules are the
old rules with the table row's strings put back. A call into another family keeps its key (the same key, or
a key column of the table), so recursion and mutual calls fold too. A family is folded only if every member
can be: same number of rules, no member passed as a relation argument (`nearest(n, a, rel)` cannot take a
key), and every call that differs between members lands in one folded family.

Folding can make a program unstratifiable even where each key alone is fine: if `a_x` negates `b_x` and
`b_y` calls `a_y`, the folded `a` negates `b` and `b` calls `a`, and Soufflé stratifies by relation.
Families on a cycle through `!` or an aggregate, in the graph with families collapsed, are left unfolded.
"""

import re

from simplify import GENERATED, _calls, _canon, _locals, _split_protected

MIN_FAMILY = 3


def _library_names():
    """Relation and functor names the libraries declare: a family must not take one (`call`, `inside`, …)."""
    import glob
    import os
    lib = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "lib")
    names = set()
    for f in glob.glob(os.path.join(lib, "**", "*.rr"), recursive=True):
        names |= set(re.findall(r"^\.(?:decl|edit|functor)\s+(\w+)", open(f).read(), re.M))
    return names


RESERVED = _library_names() | {"node", "kind", "field", "child", "text", "match", "glob", "count", "cat", "strlen",
                               "min", "max", "sum", "mean", "range", "as", "true", "false", "join", "by", "sep"}
_PH_S = "\x00S\x00"
_PH_R = "\x00R\x00"


def _template(lit, cands):
    """Lit with each string literal and each call `c(arg)` of a candidate replaced by a placeholder.
    Returns (template, slots) with slots [("S", '"text"') | ("R", name, arg)] in text order."""
    parts = _split_protected(lit)
    out, slots = [], []
    call = re.compile(r"(?<![\w.@$])([a-z_]\w*)\(([^()]*)\)")
    for i, p in enumerate(parts):
        if i % 2:
            # A regex with a named group binds a metavariable, and share_metavariables reads it from the pattern
            # text: it must stay in the rule, not move into the table.
            if p.startswith('"') and not re.search(r"\(\?P?<(?![=!])", p):
                out.append(_PH_S)
                slots.append(("S", p))
            else:
                out.append(p)
            continue

        def rep(m):
            if m.group(1) in cands:
                slots.append(("R", m.group(1), m.group(2)))
                return _PH_R + "(" + m.group(2) + ")"
            return m.group(0)
        out.append(call.sub(rep, p))
    return "".join(out), slots


def _bare_uses(rs, names):
    """Relations passed as an argument somewhere (`within_stop(a, b, stop_9)`): they cannot take a key."""
    bare = set()
    for r in rs.all():
        for l in rs.lits(r):
            code = "".join(p for i, p in enumerate(_split_protected(l)) if i % 2 == 0)
            for m in re.finditer(r"(?<![\w.@$])([a-z_]\w*)\b(?!\s*\()", code):
                if m.group(1) in names:
                    bare.add(m.group(1))
    return bare


def _key_and_name(members):
    toks = [m.split("_") for m in members]
    pre = 0
    while all(len(t) > pre for t in toks) and len({t[pre] for t in toks}) == 1:
        pre += 1
    suf = 0
    while all(len(t) > pre + suf for t in toks) and len({t[-1 - suf] for t in toks}) == 1:
        suf += 1
    keys = {m: "_".join(t[pre:len(t) - suf]) for m, t in zip(members, toks)}
    if any(not k for k in keys.values()) or len(set(keys.values())) != len(members):
        keys = {m: m for m in members}
    name = "_".join(toks[0][:pre] + (toks[0][len(toks[0]) - suf:] if suf else []))
    return keys, name


def fold(rs, decls, notes, allowed=None):
    """Fold in place; returns the number of relations folded away. `allowed`, on a retry, limits which
    relations may join a family (those not dropped for stratification)."""
    defs = rs.defs()
    cands = {n for n, rules in defs.items()
             if n not in rs.protected and n not in rs.facts and all(rs.head(r) == f"{n}(n)" for r in rules)}
    if allowed is not None:
        cands &= allowed
    cands -= _bare_uses(rs, cands)
    cands -= _nested_calls(rs, cands)
    # Signature of each candidate: its rules with strings, candidate calls and local names abstracted.
    info = {}
    for n in sorted(cands):
        rows = []
        for r in defs[n]:
            tl = [_template(l, cands) for l in rs.lits(r)]
            canon = _canon([("n", [t for t, _ in tl])])[0]
            rows.append((canon, [t for t, _ in tl], [s for _, ss in tl for s in ss], r))
        # Rules alike up to their strings may pair with another member's in any order: either way each member's
        # rule set comes back exactly. Sorting by the slots too keeps the output stable.
        rows.sort(key=lambda x: (x[0], repr(x[2])))
        info[n] = rows
    groups = {}
    for n, rows in info.items():
        groups.setdefault(tuple(x[0] for x in rows), []).append(n)
    fams = {}
    for sig, members in groups.items():
        if len(members) >= MIN_FAMILY:
            members.sort()
            keys, name = _key_and_name(members)
            fams[sig] = {"members": members, "keys": keys, "name": name}
    # A family is foldable when each call slot is the same relation in every member, or lands in one family.
    while True:
        fam_of = {m: f for f in fams.values() for m in f["members"]}
        drop = []
        for sig, f in fams.items():
            ms = f["members"]
            slots = [[s for x in info[m] for s in x[2]] for m in ms]
            plan = []
            ok = True
            for j in range(len(slots[0])):
                col = [s[j] for s in slots]
                if col[0][0] == "S":
                    plan.append(("const", col[0][1]) if len({c[1] for c in col}) == 1 else ("col", [c[1] for c in col]))
                    continue
                targets = [c[1] for c in col]
                if len(set(targets)) == 1:
                    plan.append(("const", None))
                    continue
                g = fam_of.get(targets[0])
                if g is None or any(fam_of.get(t) is not g for t in targets):
                    ok = False
                    break
                gkeys = [g["keys"][t] for t in targets]
                if all(gk == f["keys"][m] for gk, m in zip(gkeys, ms)):
                    plan.append(("call_same", g))
                else:
                    plan.append(("call_col", g, [f'"{k}"' for k in gkeys]))
            if ok:
                f["plan"] = plan
            else:
                drop.append(sig)
        if not drop:
            break
        for sig in drop:
            del fams[sig]
    if not fams:
        return 0
    # Stratification: drop families on a cycle through negation once collapsed, then plan again without them.
    node_of = {m: "\x00" + str(id(f)) for f in fams.values() for m in f["members"]}
    bad = _unstratified(rs, defs, node_of)
    if bad:
        keep = {m for f in fams.values() if "\x00" + str(id(f)) not in bad for m in f["members"]}
        if allowed is not None:
            keep &= allowed
        return fold(rs, decls, notes, keep) if keep else 0

    used = set(defs) | set(rs.facts) | set(decls) | RESERVED
    for f in fams.values():
        base = f["name"] or f["members"][0] + "_family"
        while base in used or (GENERATED.match(base) and base not in f["members"]):
            base += "_of"
        if base in f["members"]:
            base += "_of"
        f["name"] = base
        used.add(base)
    fam_of = {m: f for f in fams.values() for m in f["members"]}

    # Rewrite every call of a member, outside the families' own bodies, to the family with its key.
    def rewrite_calls(lit):
        parts = _split_protected(lit)
        for i in range(0, len(parts), 2):
            parts[i] = re.sub(r"(?<![\w.@$])([a-z_]\w*)\(([^()]*)\)",
                              lambda m: (f'{fam_of[m.group(1)]["name"]}({m.group(2)}, "{fam_of[m.group(1)]["keys"][m.group(1)]}")'
                                         if m.group(1) in fam_of else m.group(0)), parts[i])
        return "".join(parts)

    gone = set()
    for f in fams.values():
        ms, plan, name = f["members"], f["plan"], f["name"]
        first = info[ms[0]]
        taken = set()
        for _, tmpl, _, r in first:
            taken |= set(_locals(tmpl)) | {"n"}

        def fresh(base, taken=taken):
            k = 1
            while f"{base}{k}" in taken:
                k += 1
            taken.add(f"{base}{k}")
            return f"{base}{k}"
        key = fresh("q")
        f["key"] = key
        cols, colvar = [], {}   # table columns after the key (values per member), and their variables
        keyvals = tuple(f'"{f["keys"][m]}"' for m in ms)

        def column(vals):
            vals = tuple(vals)
            if vals == keyvals:
                return key
            if vals not in colvar:
                colvar[vals] = fresh("s")
                cols.append(vals)
            return colvar[vals]
        subst = []     # per slot: None (unchanged), a variable (a string column), or ("call", family, key expr)
        for p in plan:
            if p[0] == "const":
                subst.append(None)
            elif p[0] == "col":
                subst.append(column(p[1]))
            elif p[0] == "call_same":
                subst.append(("call", p[1], key))
            else:
                subst.append(("call", p[1], column(p[2])))
        table = f"{name}_case"
        while table in used:
            table += "_of"
        used.add(table)
        rows = [tuple([f'"{f["keys"][m]}"'] + [c[i] for c in cols]) for i, m in enumerate(ms)]
        rs.facts[table] = [tuple(_unq(x) for x in row) for row in rows]
        decls[table] = f".decl {table}(key: symbol{''.join(f', {colvar[c]}: symbol' for c in cols)})"
        jnames = [_json_name(notes.get(m, []), m) for m in ms]
        shown = ", ".join(f"`{j}`" for j in jnames[:3]) + (f" and {len(ms) - 3} more" if len(ms) > 3 else "")
        notes[table] = [f"// {table}: one row per relation folded into {name}: its key, then the strings that differed."]
        decls[name] = f".decl {name}(n: node, key: symbol)"
        notes[name] = [f"// {name}(n, key): {len(ms)} copies of one rule set ({shown}), folded into one: row `key` of",
                       f"// {table} gives the strings each copy had."]
        # The family's rules: the first member's templates with each slot put back.
        si = 0
        new_rules = []
        for _, tmpl, slots0, r in first:
            lits = [f"{table}({', '.join([key] + [colvar[c] for c in cols])})"]
            for t in tmpl:
                out = []
                pieces = re.split(f"({re.escape(_PH_S)}|{re.escape(_PH_R)}\\(([^()]*)\\))", t)
                # re.split with two groups yields [text, whole, arg, text, whole, arg, …]
                k = 0
                while k < len(pieces):
                    out.append(pieces[k])
                    if k + 1 < len(pieces):
                        whole, arg = pieces[k + 1], pieces[k + 2]
                        plan_i, sub = plan[si], subst[si]
                        if whole == _PH_S:
                            out.append(slots0[_slot_index(first, r, si, info, ms[0])][1] if sub is None else sub)
                        else:
                            callee = slots0[_slot_index(first, r, si, info, ms[0])][1]
                            if sub is None:
                                out.append(rewrite_calls(f"{callee}({arg})"))
                            else:
                                out.append(f'{sub[1]["name"]}({arg}, {sub[2]})')
                        si += 1
                    k += 3
                lits.append("".join(out))
            new_rules.append((r, lits))
        f["new"] = new_rules
        gone |= set(ms)

    # Install: the first member's rule objects carry the family's rules; the other members' rules go.
    for f in fams.values():
        for r, lits in f["new"]:
            if isinstance(r, dict):
                r["head"] = f'{f["name"]}(n, {f["key"]})'
            else:
                r[0] = f'{f["name"]}(n, {f["key"]})'
            rs.set_lits(r, lits)
    keep = {id(r) for f in fams.values() for r, _ in f["new"]}
    rs.helper_rules[:] = [h for h in rs.helper_rules if id(h) in keep or rs.head(h).split("(")[0] not in gone]
    rs.entries[:] = [e for e in rs.entries if id(e) in keep or rs.head(e).split("(")[0] not in gone]
    for r in rs.all():
        if id(r) not in keep:
            rs.set_lits(r, [rewrite_calls(l) for l in rs.lits(r)])
    for n in gone:
        decls.pop(n, None)
    return len(gone) - len(fams)


def _slot_index(first, r, si, info, m):
    """Slots are numbered across the member's sorted rules; return the index within rule r's own slot list."""
    before = 0
    for _, _, slots, rr in first:
        if rr is r:
            return si - before
        before += len(slots)
    raise AssertionError("rule not in family")


def _unq(s):
    import json
    return json.loads(s)


def _json_name(note, m):
    for line in note:
        g = re.search(r"the JSON util `([^`]+)`", line)
        if g:
            return g.group(1)
    return m


def _unstratified(rs, defs, node_of):
    """Collapsed family nodes on a cycle that has an edge through `!` or an aggregate."""
    graph, neg = {}, set()
    for name, rules in defs.items():
        a = node_of.get(name, name)
        for r in rules:
            for l in rs.lits(r):
                negative = l.lstrip().startswith("!") or " : {" in l or "!" in "".join(
                    p for i, p in enumerate(_split_protected(l)) if i % 2 == 0)
                code = "".join(p for i, p in enumerate(_split_protected(l)) if i % 2 == 0)
                called = set(re.findall(r"(?<![\w.@$])([a-z_]\w*)\(", code))
                called |= {x for x in re.findall(r"(?<![\w.@$])([a-z_]\w*)\b", code) if GENERATED.match(x)}
                for c in called:
                    if c in defs:
                        b = node_of.get(c, c)
                        graph.setdefault(a, set()).add(b)
                        if negative:
                            neg.add((a, b))
    comp = _scc(graph)
    fam_nodes = set(node_of.values())
    bad = set()
    for a, b in neg:
        if comp.get(a) == comp.get(b):
            bad |= {a, b} & fam_nodes
    return bad


def _scc(graph):
    """Tarjan's strongly connected components, iteratively: node -> a representative of its component."""
    index, low, comp, stack, on = {}, {}, {}, [], set()
    nodes = sorted(set(graph) | {b for bs in graph.values() for b in bs})
    for root in nodes:
        if root in index:
            continue
        index[root] = low[root] = len(index)
        stack.append(root)
        on.add(root)
        work = [(root, iter(sorted(graph.get(root, ()))))]
        while work:
            v, it = work[-1]
            nxt = next(it, None)
            if nxt is not None:
                if nxt not in index:
                    index[nxt] = low[nxt] = len(index)
                    stack.append(nxt)
                    on.add(nxt)
                    work.append((nxt, iter(sorted(graph.get(nxt, ())))))
                elif nxt in on:
                    low[v] = min(low[v], index[nxt])
                continue
            work.pop()
            if work:
                low[work[-1][0]] = min(low[work[-1][0]], low[v])
            if low[v] == index[v]:
                while True:
                    w = stack.pop()
                    on.discard(w)
                    comp[w] = v
                    if w == v:
                        break
    return comp


def _nested_calls(rs, names):
    """Relations called with an argument that holds parentheses (`r(first(n))`): the call rewrite cannot take them."""
    out = set()
    for r in rs.all():
        for l in rs.lits(r):
            code = "".join(p for i, p in enumerate(_split_protected(l)) if i % 2 == 0)
            for m in re.finditer(r"(?<![\w.@$])([a-z_]\w*)\(", code):
                if m.group(1) in names and not re.match(r"[^()]*\)", code[m.end():]):
                    out.add(m.group(1))
    return out
