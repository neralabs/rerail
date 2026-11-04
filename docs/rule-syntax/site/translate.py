"""Render a rerail.policy/v1 JSON document as Rerail logic (.rr) text.

Display translation for the syntax page. Structural terms lower to Datalog over the core tape relations;
terms without a lowering here stay as matcher atoms, `ast($n, {...})`, which the engine compiles as it does today.
"""

import json
import re

LANG_LIB = {
    "python": "python",
    "typescript": "tsx",
    "javascript": "tsx",
    "tsx": "tsx",
}
SIMPLE_KEYS = {"kind"}


def q(s):
    return json.dumps(s, ensure_ascii=False)


def snake(name):
    s = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", name)
    s = re.sub(r"[^0-9A-Za-z]+", "_", s).strip("_")
    if not s:
        s = "r"
    if s[0].isdigit():
        s = "r_" + s
    return s.lower()


def ident(code):
    s = re.sub(r"[^0-9A-Za-z]+", "_", code).strip("_")
    if not s or s[0].isdigit():
        s = "V_" + s if s and not s[0].isdigit() else "V" + s
    return s


def compact(obj):
    return json.dumps(obj, ensure_ascii=False, separators=(", ", ": "))


class Program:
    def __init__(self, lang):
        self.lang = lang
        self.rules = []  # (head, [literals])
        self.decls = {}  # name -> signature
        self.uses = set()
        self.untranslated = 0
        self.helpers = 0
        self.names = set()

    def helper(self, base):
        self.helpers += 1
        name = f"{base}_{self.helpers}"
        self.decls[name] = f".decl {name}(n: node)"
        return name

    def use(self, module):
        lib = LANG_LIB.get(self.lang, self.lang)
        self.uses.add(f"{lib}/{module}")


class Vars:
    def __init__(self):
        self.n = 0

    def fresh(self, p):
        self.n += 1
        return f"${p}{self.n}"


def lower(prog, rule, v, vs, skip=()):
    """Literals that hold when node v matches the rule object."""
    lits = []
    if not isinstance(rule, dict):
        prog.untranslated += 1
        return [f"ast({v}, {compact(rule)})"]
    rest = {}
    for k, val in rule.items():
        if k in skip:
            continue
        if k == "kind":
            lits.append(f"kind({v}, {q(val)})")
        elif k == "regex" and isinstance(val, dict) and "$option" in val:
            o = vs.fresh("o")
            lits.append(f"option({q(val['$option'])}, {o}), {v} =~ {o}")
        elif k == "regex":
            lits.append(f"{v} =~ {q(val)}")
        elif k == "spread" and isinstance(val, bool):
            lits.append(f"spread({v})" if val else f"!spread({v})")
        elif k == "capture" and isinstance(val, str):
            lits.append(f"${val} = {v}")
        elif k == "compare" and isinstance(val, dict) and {"left", "op", "right"} <= set(val):
            lits.extend(compare(val, vs))
        elif k == "count" and isinstance(val, dict):
            lits.extend(count(prog, val, v, vs))
        elif k == "nthChild" and isinstance(val, dict) and isinstance(val.get("position"), int):
            lits.extend(nth_child(prog, val, v, vs))
        elif k == "binds" and isinstance(val, dict):
            prog.use("names")
            lits.extend(site_filters(prog, val, v, vs))
        elif k == "parse" and isinstance(val, dict) and "grammar" in val:
            r = vs.fresh("r")
            lits.append(f"parsed({v}, {q(val['grammar'])}, {r})")
            lits.extend(lower(prog, {kk: vv for kk, vv in val.items() if kk != "grammar"}, r, vs))
        elif k == "pattern" and isinstance(val, dict) and "context" in val:
            sel = f" at {q(val['selector'])}" if "selector" in val else ""
            lits.append(f"{v} ~ `{val['context'].replace(chr(10), chr(92) + 'n')}`{sel}")
        elif k == "matches" and isinstance(val, dict) and len(val) == 1:
            name, args = next(iter(val.items()))
            prog.names.add(snake(name))
            lits.append(f"{snake(name)}({v}, {', '.join(str(a) for a in args.values())})")
        elif k == "notRegex":
            lits.append(f"!({v} =~ {q(val)})")
        elif k == "pattern" and isinstance(val, str):
            lits.append(f"{v} ~ `{val.replace(chr(10), chr(92) + 'n')}`")
        elif k == "value" and isinstance(val, dict) and (len(val) > 1 or "type" in val):
            x = vs.fresh("v")
            lits.append(f"{'static_value' if val.get('follow') else 'value'}({v}, {x})")
            for op, arg in val.items():
                if op == "follow":
                    continue
                if op == "regex":
                    lits.append(f"{x} =~ {q(arg)}")
                elif op == "eq":
                    lits.append(f"{x} = {q(arg)}")
                elif op == "type":
                    lits.append(f"value_type({x}, {q(arg)})")
                elif op in ("ne", "lt", "le", "gt", "ge"):
                    lits.append(f"{x} {({'ne': '!=', 'lt': '<', 'le': '<=', 'gt': '>', 'ge': '>='})[op]} {q(arg)}")
                else:
                    prog.untranslated += 1
                    lits.append(f"ast({v}, {compact({'value': {op: arg}})})")
        elif k == "$option" and isinstance(val, str) and "cases" in rule:
            lits.extend(option_cases(prog, val, rule, v))
        elif k in ("cases", "default") and "$option" in rule:
            continue
        elif k == "value" and isinstance(val, dict) and len(val) == 1:
            op, arg = next(iter(val.items()))
            if op == "eq":
                lits.append(f"value({v}, {q(arg)})")
            elif op in ("ne", "lt", "le", "gt", "ge"):
                x = vs.fresh("v")
                sym = {"ne": "!=", "lt": "<", "le": "<=", "gt": ">", "ge": ">="}[op]
                lits.append(f"value({v}, {x}), {x} {sym} {q(arg)}")
            elif op == "in" and isinstance(arg, list):
                x = vs.fresh("v")
                lits.append(f"value({v}, {x}), {x} in {{{', '.join(q(a) for a in arg)}}}")
            elif op == "regex":
                x = vs.fresh("v")
                lits.append(f"value({v}, {x}), {x} =~ {q(arg)}")
            elif op in ("startsWith", "endsWith", "contains") and isinstance(arg, str):
                x = vs.fresh("v")
                pat = {"startsWith": "^" + re.escape(arg), "endsWith": re.escape(arg) + "$", "contains": re.escape(arg)}[op]
                lits.append(f"value({v}, {x}), {x} =~ {q(pat)}")
            else:
                rest[k] = val
        elif k in ("has", "inside", "precedes", "follows") and isinstance(val, dict):
            lits.extend(relation(prog, k, val, v, vs))
        elif k == "all" and isinstance(val, list):
            for sub in val:
                lits.extend(lower(prog, sub, v, vs))
        elif k == "any" and isinstance(val, list):
            lits.extend(alternatives(prog, val, v, vs))
        elif k == "not":
            inner = lower(prog, val, v, vs)
            if len(inner) == 1 and re.match(r"^[a-z_][a-z0-9_]*\(", inner[0]):
                lits.append("!" + inner[0])
            else:
                lits.append("!(" + ", ".join(inner) + ")")
        elif k == "matches" and isinstance(val, str):
            prog.names.add(snake(val))
            lits.append(f"{snake(val)}({v})")
        elif k == "nthChild" and isinstance(val, int):
            lits.append(f"child(_, {v}, {val - 1})")
        elif k == "resolves" and isinstance(val, dict):
            lits.extend(resolves(prog, val, v, vs))
        else:
            rest[k] = val
    if rest:
        prog.untranslated += 1
        lits.append(f"ast({v}, {compact(rest)})")
    if not lits:
        lits.append(f"node(_, {v}, _, _, _)")
    return lits


def compare(c, vs):
    l, r, op = "$" + str(c["left"]), "$" + str(c["right"]), str(c["op"])
    a, b = vs.fresh("t"), vs.fresh("t")
    reader = "text" if c.get("text", "node") in ("node", "text") else "value"
    if op == "eq":
        return [f"{reader}({l}, {a}), {reader}({r}, {a})"]
    sym = {"ne": "!=", "lt": "<", "le": "<=", "gt": ">", "ge": ">="}.get(op, op)
    return [f"{reader}({l}, {a}), {reader}({r}, {b}), {a} {sym} {b}"]


def count(prog, c, v, vs):
    inner = {k: x for k, x in c.items() if k not in ("min", "max", "eq")}
    body = lower(prog, inner, v, Vars())
    k = vs.fresh("k")
    lits = [f"{k} = count {{ {', '.join(body)} }}"]
    if "eq" in c:
        lits.append(f"{k} = {c['eq']}")
    if "min" in c:
        lits.append(f"{k} >= {c['min']}")
    if "max" in c:
        lits.append(f"{k} <= {c['max']}")
    return lits


def nth_child(prog, c, v, vs):
    p, s, k = vs.fresh("p"), vs.fresh("s"), vs.fresh("k")
    order = f"precedes({v}, {s})" if c.get("reverse") else f"precedes({s}, {v})"
    if "ofRule" in c:
        rel = prog.helper("counted")
        prog.rules.append((f"{rel}($n)", lower(prog, c["ofRule"], "$n", Vars())))
        return [f"child({p}, {v}, _), {rel}({v})",
                f"{k} = count {{ child({p}, {s}, _), {order}, {rel}({s}) }}", f"{k} = {c['position'] - 1}"]
    return [f"child({p}, {v}, _), {k} = count {{ child({p}, {s}, _), {order} }}", f"{k} = {c['position'] - 1}"]


def site_filters(prog, filters, s, vs):
    lits = []
    for bk, bv in filters.items():
        if bk == "form" and isinstance(bv, str):
            lits.append(f"site_form({s}, {q(bv)})")
        elif bk == "form" and isinstance(bv, list):
            f = vs.fresh("f")
            lits.append(f"site_form({s}, {f}), {f} in {{{', '.join(q(x) for x in bv)}}}")
        elif bk in ("site",):
            lits.extend(lower(prog, bv, s, vs))
        elif bk == "value":
            x = vs.fresh("x")
            lits.append(f"binding_value({s}, {x})")
            lits.extend(lower(prog, bv, x, vs))
        elif bk in ("occurrence", "annotation") and isinstance(bv, dict):
            x = vs.fresh("o")
            lits.append(f"site_{bk}({s}, {x})")
            lits.extend(lower(prog, bv, x, vs))
        elif isinstance(bv, list) and all(isinstance(x, str) for x in bv):
            x = vs.fresh("f")
            lits.append(f"site_{snake(bk)}({s}, {x}), {x} in {{{', '.join(q(y) for y in bv)}}}")
        elif bv is True:
            lits.append(f"site_{snake(bk)}({s})")
        elif bv is False:
            lits.append(f"!site_{snake(bk)}({s})")
        elif isinstance(bv, (str, int, float)):
            lits.append(f"site_{snake(bk)}({s}, {q(bv)})")
        else:
            prog.untranslated += 1
            lits.append(f"ast({s}, {compact({bk: bv})})")
    return lits


def option_cases(prog, name, rule, v):
    rel = prog.helper("option")
    keys = []
    for case, body in (rule.get("cases") or {}).items():
        keys.append(case)
        bodies = body if isinstance(body, list) else [body]
        for b in bodies:
            prog.rules.append((f"{rel}($n)", [f"option({q(name)}, {q(case)})"] + lower(prog, b, "$n", Vars())))
    if "default" in rule:
        guard = f"!some {{ option({q(name)}, $o), $o in {{{', '.join(q(k) for k in keys)}}} }}"
        prog.rules.append((f"{rel}($n)", [guard] + lower(prog, rule["default"], "$n", Vars())))
    return [f"{rel}({v})"]


def kinds_only(alts):
    return all(isinstance(a, dict) and set(a) == {"kind"} for a in alts)


def value_eq_only(alts):
    return all(isinstance(a, dict) and set(a) == {"value"} and isinstance(a["value"], dict)
               and set(a["value"]) == {"eq"} for a in alts)


def grouped(prog, alts, v, vs):
    """One body per alternative, with plain kind and value alternatives folded into set tests."""
    kinds = [a["kind"] for a in alts if isinstance(a, dict) and set(a) == {"kind"} and isinstance(a["kind"], str)]
    vals = [a["value"]["eq"] for a in alts if isinstance(a, dict) and set(a) == {"value"}
            and isinstance(a["value"], dict) and set(a["value"]) == {"eq"}]
    bodies = []
    if len(kinds) > 1:
        k = vs.fresh("k")
        bodies.append([f"kind({v}, {k}), {k} in {{{', '.join(q(x) for x in kinds)}}}"])
    if len(vals) > 1:
        x = vs.fresh("v")
        bodies.append([f"value({v}, {x}), {x} in {{{', '.join(q(y) for y in vals)}}}"])
    for a in alts:
        folded = (len(kinds) > 1 and isinstance(a, dict) and set(a) == {"kind"} and isinstance(a.get("kind"), str)) or \
                 (len(vals) > 1 and isinstance(a, dict) and set(a) == {"value"} and isinstance(a["value"], dict)
                  and set(a["value"]) == {"eq"})
        if not folded:
            bodies.append(lower(prog, a, v, vs))
    return bodies


def alternatives(prog, alts, v, vs):
    bodies = grouped(prog, alts, v, vs)
    if len(bodies) == 1:
        return bodies[0]
    name = prog.helper("alt")
    for alt in alts:
        pass
    for body in grouped(prog, alts, "$n", Vars()):
        prog.rules.append((f"{name}($n)", body))
    return [f"{name}({v})"]


def stop_set(prog, stop):
    if isinstance(stop, dict) and set(stop) == {"kind"}:
        return "{" + q(stop["kind"]) + "}"
    if isinstance(stop, dict) and set(stop) == {"any"} and kinds_only(stop["any"]):
        return "{" + ", ".join(q(a["kind"]) for a in stop["any"]) + "}"
    name = prog.helper("stop")
    if isinstance(stop, dict) and set(stop) == {"any"}:
        for body in grouped(prog, stop["any"], "$n", Vars()):
            prog.rules.append((f"{name}($n)", body))
    else:
        prog.rules.append((f"{name}($n)", lower(prog, stop, "$n", Vars())))
    return name


def relation(prog, k, sub, v, vs):
    f = sub.get("field")
    stop = sub.get("stopBy", "neighbor")
    c = vs.fresh("c" if k == "has" else "p" if k == "inside" else "s")
    fld = f", field: {q(f)}" if f else ""
    lits = []
    if k in ("has", "inside"):
        lo, hi = (c, v) if k == "has" else (v, c)
        if stop == "neighbor":
            lits.append(f"field({hi}, {q(f)}, {lo})" if f else f"child({hi}, {lo}, _)")
        elif stop == "end":
            lits.append(f"within({lo}, {hi}{fld})")
        else:
            lits.append(f"within({lo}, {hi}, {stop_set(prog, stop)}{fld})")
    else:
        a, b = (v, c) if k == "precedes" else (c, v)
        if stop == "neighbor":
            lits.append(f"next_sibling({a}, {b})")
        elif stop == "end":
            lits.append(f"precedes({a}, {b})")
        else:
            lits.append(f"precedes({a}, {b}, {stop_set(prog, stop)})")
    lits.extend(lower(prog, sub, c, vs, skip=("field", "stopBy")))
    return lits


def resolves(prog, r, v, vs):
    lits = []
    rest = {}
    for k, val in r.items():
        if k == "imported" and isinstance(val, list):
            prog.use("names")
            x = vs.fresh("q")
            lits.append(f"imported_sym({v}, {x}), {x} in {{{', '.join(q(s) for s in val)}}}")
        elif k == "import" and isinstance(val, list):
            prog.use("names")
            x = vs.fresh("q")
            if r.get("prefix"):
                alt = "|".join(re.escape(s) for s in val)
                lits.append(f"sym_of({v}, {x}), {x} =~ {q('^(' + alt + r')(\.|$)')}")
            else:
                lits.append(f"sym_of({v}, {x}), {x} in {{{', '.join(q(s) for s in val)}}}")
        elif k == "prefix":
            continue
        elif k == "directives" and isinstance(val, str):
            lits.append(f"global_directives({v}, {q(val)})")
        elif k == "global":
            prog.use("names")
            if isinstance(val, list):
                x = vs.fresh("g")
                lits.append(f"global_ref({v}, {x}), {x} in {{{', '.join(q(s) for s in val)}}}")
            else:
                lits.append(f"global_ref({v}, _)")
        elif k == "unresolved" and val is True:
            prog.use("names")
            lits.append(f"!ref_site({v}, _), !global_ref({v}, _)")
        elif k == "binding" and isinstance(val, dict):
            prog.use("names")
            s = vs.fresh("b")
            lits.append(f"ref_site({v}, {s})")
            lits.extend(site_filters(prog, val, s, vs))
        else:
            rest[k] = val
    if rest:
        prog.untranslated += 1
        lits.append(f"ast({v}, {compact({'resolves': rest})})")
    return lits


def wrap(head, lits, width=112):
    if not lits:
        return head + "."
    out = head + " :- "
    line = out
    lines = []
    for i, lit in enumerate(lits):
        piece = lit + ("," if i < len(lits) - 1 else ".")
        if len(line) + len(piece) + 1 > width and line.strip() not in (head + " :-",):
            lines.append(line.rstrip())
            line = "    " + piece + " "
        else:
            line += piece + " "
    lines.append(line.rstrip())
    return "\n".join(lines)


def render_util(prog, name, rule, out):
    rel = snake(name)
    params = rule.get("params") if isinstance(rule, dict) else None
    if params:
        rule = rule["rule"] if "rule" in rule else {k: v for k, v in rule.items() if k != "params"}
        args = ", ".join("$" + p for p in params)
        prog.decls[rel] = f".decl {rel}(n: node, {', '.join(p.lower() + ': node' for p in params)})"
        out.append((f"{rel}($n, {args})", lower(prog, rule, "$n", Vars())))
        return
    prog.decls[rel] = f".decl {rel}(n: node)"
    if isinstance(rule, dict) and set(rule) == {"any"} and isinstance(rule["any"], list):
        for body in grouped(prog, rule["any"], "$n", Vars()):
            out.append((f"{rel}($n)", body))
    else:
        out.append((f"{rel}($n)", lower(prog, rule, "$n", Vars())))


def violation_lines(viol, rel):
    code = viol.get("code", rel)
    msg = viol.get("message", "")
    head = f".violation {rel}(at: node)"
    if code != rel:
        head += f" code {q(code)}"
    lines = [f"{head} {q(msg)}"]
    anchor = viol.get("anchor")
    if anchor and anchor != "witness.span":
        lines.append(f"    anchor {anchor.replace('witness.', '')}")
    for tool, spec in (viol.get("suppress") or {}).items():
        names = spec.get("codes") or spec.get("rules") or []
        if names:
            lines.append(f"    suppress {tool} {', '.join(q(n) for n in names)}")
        else:
            lines.append(f"    suppress {tool}")
    if "fix" in viol:
        lines.append(f"    fix {compact(viol['fix'])}")
    if "each" in viol:
        lines.append(f"    each {compact(viol['each'])}")
    return lines


def translate(doc):
    clauses = doc.get("clauses", [])
    langs = [c["forbid"]["ast"].get("language") for c in clauses if "ast" in c.get("forbid", {})]
    langs += [c["define"].get("language") for c in clauses if "define" in c]
    lang = next((l for l in langs if l), "")
    prog = Program(lang)
    head = [f".policy {q(doc.get('id', ''))} revision {doc.get('revision', 1)}"]
    if doc.get("description"):
        head.append(f".description {q(doc['description'])}")
    if lang:
        head.append(f".language {lang}")
    head.append(f".mode {doc.get('mode', 'log')}")
    scope = doc.get("scope") or {}
    if scope.get("include"):
        head.append(".scope include " + ", ".join(q(g) for g in scope["include"]))
    if scope.get("exclude"):
        head.append(".scope exclude " + ", ".join(q(g) for g in scope["exclude"]))
    for k, val in scope.items():
        if k not in ("include", "exclude"):
            head.append(f".scope {k} {compact(val)}")

    body = []
    seen_rel = set()
    for c in clauses:
        cid = c.get("id", "clause")
        if "define" in c:
            d = c["define"]
            utils = []
            for name, rule in (d.get("utils") or {}).items():
                render_util(prog, name, rule, utils)
            if d.get("options"):
                body.append(("opt", f".options {compact(d['options'])}"))
            for h, lits in utils:
                body.append(("rule", (h, lits)))
            continue
        forbid = c.get("forbid", {})
        viol = c.get("violation", {})
        rel = ident(viol.get("code", cid))
        if rel in seen_rel:
            rel = f"{rel}_{snake(cid)}"
        seen_rel.add(rel)
        if "ast" not in forbid:
            body.append(("viol", violation_lines(viol, rel)))
            body.append(("raw", f"{rel}($d) :- text_rule($d, {compact(forbid)})."))
            prog.untranslated += 1
            continue
        a = forbid["ast"]
        local = []
        for name, rule in (a.get("utils") or {}).items():
            render_util(prog, name, rule, local)
        opts = {k: v for k, v in a.items() if k not in ("language", "rule", "utils", "constraints")}
        vl = violation_lines(viol, rel)
        if opts:
            vl.append(f"    options {compact(opts)}")
        body.append(("viol", vl))
        rule = a.get("rule", {})
        vs = Vars()
        if isinstance(rule, dict) and set(rule) == {"any"} and isinstance(rule["any"], list):
            bodies = grouped(prog, rule["any"], "$n", vs)
        else:
            bodies = [lower(prog, rule, "$n", vs)]
        cons = []
        for var, crule in (a.get("constraints") or {}).items():
            cons.extend(lower(prog, crule, "$" + var, vs))
        for b in bodies:
            body.append(("rule", (f"{rel}($n)", b + cons)))
        for h, lits in local:
            body.append(("rule", (h, lits)))

    out = head + [""]
    if prog.uses:
        out += [f".use {u}" for u in sorted(prog.uses)] + [""]
    declared = set()
    pending_helpers = list(prog.rules)
    for kind, item in body:
        if kind == "rule":
            h, lits = item
            name = h.split("(")[0]
            if name in prog.decls and name not in declared:
                out.append(prog.decls[name])
                declared.add(name)
            out.append(wrap(h, lits))
        elif kind == "viol":
            if out[-1] != "":
                out.append("")
            out.extend(item)
        else:
            out.append(item)
    if pending_helpers:
        out.append("")
        for h, lits in pending_helpers:
            name = h.split("(")[0]
            if name not in declared:
                out.append(prog.decls[name])
                declared.add(name)
            out.append(wrap(h, lits))
    text = "\n".join(out).rstrip() + "\n"
    return text, prog.untranslated
