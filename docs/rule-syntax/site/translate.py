"""Render a rerail.policy/v1 JSON document as Rerail logic (.rr) text, in Soufflé form.

Display translation for the syntax page. Structural terms lower to Datalog over the core tape relations;
terms without a lowering here stay as matcher atoms, `ast(n, {...})`, which the engine compiles as it does today.

Variables: the JSON's ast-grep metavariables keep their names (FUNC, OBJ: capitals); variables the translation
introduces are lower case (n, k1, q2). A helper relation that reads a metavariable its caller binds (ast-grep
shares metavariables across utils) takes it as an extra column, and every call passes it on.
"""

from textglob import text_test
import json
import os
import re

from souffle import check_rule, whole
from simplify import simplify
import main_terms

LANG_LIB = {
    "python": "python",
    "typescript": "tsx",
    "javascript": "tsx",
    "tsx": "tsx",
}
INLINE_MAX = 4      # alternatives written as a disjunction; more become a relation of facts


class OptVar(str):
    """A variable holding an option's configured value, standing where a JSON constant would."""


def q(s):
    if isinstance(s, OptVar):
        return str(s)
    return json.dumps(s, ensure_ascii=False)


def tick(s):
    """A backtick literal: fenced with one more backtick than the longest run inside, as Markdown code spans are."""
    s = s.replace("\n", "\\n")
    runs = [len(r) for r in re.findall(r"`+", s)]
    if not runs:
        return f"`{s}`"
    f = "`" * (max(runs) + 1)
    return f"{f} {s} {f}"


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


def col(name):
    """A helper's column for a metavariable: its name in lower case, unless that is `n`, the node column."""
    c = mv(name).lower()
    return c + "_mv" if c == "n" else c


def compact(obj):
    return json.dumps(obj, ensure_ascii=False, separators=(", ", ": "))


def mv(name):
    """A JSON metavariable reference ("$X", "X", "$$$X") as a Datalog variable."""
    return str(name).lstrip("$")


def cap(name):
    """A capture name as a variable. Captures are metavariables, which are written in capitals."""
    return mv(name).upper()


def caps(pattern):
    """Named groups renamed to capitals, so a group binds the same variable its anchor or message names."""
    return re.sub(r"\(\?(P?)<([A-Za-z_]\w*)>", lambda m: f"(?{m.group(1)}<{m.group(2).upper()}>", pattern)


def opt_alts(arg, render):
    """A JSON operand that may be {"$option": name, "cases": {…}, "default": …}: one guarded alternative per
    configured value. render(constant) gives the literal for one value."""
    if not (isinstance(arg, dict) and "$option" in arg):
        return render(arg)
    if not (arg.get("cases") or "default" in arg):      # the operand is the option's value itself
        return render_option(arg["$option"], render)
    name, keys = q(arg["$option"]), list(arg.get("cases") or {})
    parts = [f"option({name}, {q(c)}), {opt_alts(arg['cases'][c], render)}" for c in keys]
    guard = [f"!option({name}, {q(c)})" for c in keys]
    if "default" in arg:
        parts.append(", ".join(guard + [opt_alts(arg["default"], render)]))
    return "(" + " ; ".join(parts) + ")" if len(parts) > 1 else parts[0]


def render_option(name, render):
    o = OptVar(f"o_{snake(name)}")
    return f"option({q(name)}, {o}), " + render(o)


def match(pattern, subject):
    if isinstance(pattern, dict) and "$option" in pattern:
        return opt_alts(pattern, lambda p: match(p, subject))
    if not isinstance(pattern, str):
        return f"ast({subject}, {compact({'regex': pattern})})"
    if isinstance(pattern, OptVar):
        return f"match({pattern}, {subject})"
    rx = whole(caps(pattern))
    # A glob-family test where it is exactly the regex (ALIGNMENT.md, "Text"); `match` otherwise.
    return text_test(rx, subject) or f"match({q(rx)}, {subject})"


def _balanced(s):
    d = 0
    instr = False
    for i, ch in enumerate(s):
        if ch == '"' and (i == 0 or s[i - 1] != "\\"):
            instr = not instr
        if instr:
            continue
        if ch in "({[":
            d += 1
        elif ch in ")}]":
            d -= 1
            if d == 0 and i != len(s) - 1:
                return False
    return d == 0


def is_atom(s):
    return bool(re.match(r"^[a-z_][A-Za-z0-9_]*\(", s)) and s.endswith(")") and _balanced(s[s.index("("):])


def disj_atoms(s):
    """The branches of `(a ; b ; c)` when each is a single atom; else None."""
    if not (s.startswith("(") and s.endswith(")") and _balanced(s)):
        return None
    parts, d, cur, instr = [], 0, "", False
    for i, ch in enumerate(s[1:-1]):
        if ch == '"':
            instr = not instr
        if not instr:
            if ch in "({[":
                d += 1
            elif ch in ")}]":
                d -= 1
            elif ch == ";" and d == 0:
                parts.append(cur.strip())
                cur = ""
                continue
        cur += ch
    parts.append(cur.strip())
    return parts if len(parts) > 1 and all(is_atom(p) for p in parts) else None


def negate(lits):
    """!(…) in Soufflé form: one atom, a disjunction of atoms (De Morgan), or an aggregate."""
    if len(lits) == 1 and is_atom(lits[0]):
        return ["!" + lits[0]]
    if len(lits) == 1 and disj_atoms(lits[0]):
        return ["!" + a for a in disj_atoms(lits[0])]
    return ["count : { " + ", ".join(lits) + " } = 0"]


class Program:
    def __init__(self, lang):
        self.lang = lang
        self.rules = []  # (head, [literals]) or ("#facts", name, values)
        self.decls = {}  # name -> signature
        self.uses = set()
        self.untranslated = 0
        self.helpers = 0
        self.lib = {}  # name -> (decl, note lines, [(head, lits)]): relations main_terms.py's lowerings share
        self.names = set()
        self.notes = {}  # name -> comment lines printed above its .decl
        self.numeric = set()  # metavariables bound to a count
        self.symbolic = set()  # metavariables bound to a string by a transform
        self.scope = {}  # JSON util name -> its relation, where a clause's own util takes another name
        self.util_rels = set()  # relations utils already have

    def util_rel(self, name):
        """The relation of the util `name` where it is called or defined: the clause's own, if it has one."""
        return self.scope.get(name, snake(name))

    def helper(self, base, note):
        self.helpers += 1
        name = f"{base}_{self.helpers}"
        self.decls[name] = f".decl {name}(n: node)"
        self.notes[name] = [f"// {name}(n): {note}"]
        return name

    def facts(self, base, col, values, note):
        self.helpers += 1
        name = f"{base}_{self.helpers}"
        self.decls[name] = f".decl {name}({col}: symbol)"
        self.notes[name] = [f"// {name}: {note}"]
        self.rules.append(("#facts", name, list(values)))
        return name

    def use(self, module):
        lib = LANG_LIB.get(self.lang, self.lang)
        self.uses.add(f"{lib}/{module}")


class Vars:
    def __init__(self):
        self.n = 0

    def fresh(self, p):
        self.n += 1
        return f"{p}{self.n}"


def one_of(prog, vs, fmt, values, p, note):
    """fmt with one `{}`: an atom per value. One value: the atom; a few: a disjunction; many: a relation of facts."""
    if len(values) == 1:
        return fmt.format(q(values[0]))
    if len(values) <= INLINE_MAX:
        return "(" + " ; ".join(fmt.format(q(x)) for x in values) + ")"
    name = prog.facts("set", "v", values, note)
    x = vs.fresh(p)
    return f"{fmt.format(x)}, {name}({x})"


def lower(prog, rule, v, vs, skip=()):
    """Literals that hold when node v matches the rule object."""
    lits = []
    if isinstance(rule, list):
        # A list of rules is their conjunction (an option case's `default: [...]`); an empty list holds for any node.
        for sub in rule:
            lits.extend(lower(prog, sub, v, vs))
        return lits or [f"node(_, {v}, _, _, _)"]
    if not isinstance(rule, dict):
        prog.untranslated += 1
        return [f"ast({v}, {compact(rule)})"]
    rest = {}
    for k, val in rule.items():
        if k in skip:
            continue
        if k == "kind":
            lits.append(f"kind({v}, {q(val)})")
        elif k == "regex" and isinstance(val, dict) and "$option" in val and not (val.get("cases") or "default" in val):
            o = vs.fresh("o")   # the regex is the option's value itself
            lits.append(f"option({q(val['$option'])}, {o}), match({o}, {v})")
        elif k == "regex":
            lits.append(match(val, v))
        elif k == "spread" and "$option" in rule:
            continue  # `spread: true` on an `$option` splices the chosen list into the enclosing array
        elif k == "spread" and isinstance(val, bool):
            lits.append(f"spread({v})" if val else f"!spread({v})")
        elif k == "capture" and isinstance(val, str):
            lits.append(f"{cap(val)} = {v}")
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
            lits.append(f"{v} ~ {tick(val['context'])}{sel}")
        elif k == "matches" and isinstance(val, dict) and len(val) == 1:
            name, args = next(iter(val.items()))
            prog.names.add(prog.util_rel(name))
            lits.append(f"{prog.util_rel(name)}({v}, {', '.join(mv(a) for a in args.values())})")
        elif k == "notRegex":
            lits.append("!" + match(val, v))
        elif k == "pattern" and isinstance(val, str):
            lits.append(f"{v} ~ {tick(val)}")
        elif k == "value" and isinstance(val, dict) and (len(val) > 1 or "type" in val):
            x = vs.fresh("v")
            lits.append(f"{'static_value' if val.get('follow') else 'value'}({v}, {x})")
            for op, arg in val.items():
                if op == "follow":
                    continue
                if op == "regex":
                    lits.append(match(arg, x))
                elif op == "eq":
                    lits.append(opt_alts(arg, lambda c: f"{x} = {q(c)}"))
                elif op == "type":
                    lits.append(opt_alts(arg, lambda c: f"value_type({x}, {q(c)})"))
                elif op in ("ne", "lt", "le", "gt", "ge"):
                    sym = {'ne': '!=', 'lt': '<', 'le': '<=', 'gt': '>', 'ge': '>='}[op]
                    lits.append(opt_alts(arg, lambda c: f"{x} {sym} {q(c)}"))
                else:
                    prog.untranslated += 1
                    lits.append(f"ast({v}, {compact({'value': {op: arg}})})")
        elif k == "$option" and isinstance(val, str) and ("cases" in rule or "default" in rule):
            lits.extend(option_cases(prog, val, rule, v))
        elif k in ("cases", "default") and "$option" in rule:
            continue
        elif k == "value" and isinstance(val, dict) and len(val) == 1:
            op, arg = next(iter(val.items()))
            if op == "eq":
                lits.append(opt_alts(arg, lambda c: f"value({v}, {q(c)})"))
            elif op in ("ne", "lt", "le", "gt", "ge"):
                x = vs.fresh("v")
                sym = {"ne": "!=", "lt": "<", "le": "<=", "gt": ">", "ge": ">="}[op]
                lits.append(f"value({v}, {x}), " + opt_alts(arg, lambda c: f"{x} {sym} {q(c)}"))
            elif op == "in" and isinstance(arg, list):
                lits.append(one_of(prog, vs, f"value({v}, {{}})", arg, "v", "the JSON `value.in` list"))
            elif op == "regex":
                x = vs.fresh("v")
                lits.append(f"value({v}, {x}), {match(arg, x)}")
            elif op in ("startsWith", "endsWith", "contains") and isinstance(arg, str):
                x = vs.fresh("v")
                pat = {"startsWith": "^" + re.escape(arg), "endsWith": re.escape(arg) + "$", "contains": re.escape(arg)}[op]
                lits.append(f"value({v}, {x}), {match(pat, x)}")
            else:
                rest[k] = val
        elif k in ("has", "inside", "precedes", "follows") and isinstance(val, dict):
            lits.extend(relation(prog, k, val, v, vs))
        elif k == "all" and isinstance(val, list):
            for sub in val:
                if isinstance(sub, dict) and "$option" in sub and sub.get("spread") is True:
                    lits.extend(option_cases(prog, sub["$option"], sub, v, conj=True))  # spliced into the `all`
                else:
                    lits.extend(lower(prog, sub, v, vs))
        elif k == "any" and isinstance(val, list):
            lits.extend(alternatives(prog, val, v, vs))
        elif k == "not":
            lits.extend(negate(lower(prog, val, v, vs)))
        elif k == "matches" and isinstance(val, str):
            prog.names.add(prog.util_rel(val))
            lits.append(f"{prog.util_rel(val)}({v})")
        elif k == "nthChild" and isinstance(val, int):
            lits.append(f"child(_, {v}, {val - 1})")
        elif k == "arguments" and isinstance(val, dict) and prog.lang == "python":
            lits.extend(main_terms.arguments(prog, val, v, vs))
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
    if not isinstance(c["op"], str):     # e.g. {"$option": …}: no .rr form yet
        return [f"ast({mv(c['left'])}, {compact({'compare': c})})"]
    l, r, op = mv(c["left"]), mv(c["right"]), str(c["op"])
    a, b = vs.fresh("t"), vs.fresh("t")
    reader = "text" if c.get("text", "node") in ("node", "text") else "value"
    if op == "eq":
        return [f"{reader}({l}, {a}), {reader}({r}, {a})"]
    sym = {"ne": "!=", "lt": "<", "le": "<=", "gt": ">", "ge": ">="}.get(op, op)
    return [f"{reader}({l}, {a}), {reader}({r}, {b}), {a} {sym} {b}"]


def count(prog, c, v, vs):
    inner = {k: x for k, x in c.items() if k not in ("min", "max", "eq", "as", "offset")}
    body = lower(prog, inner, v, vs)     # the same counter: a reused name would join with the outer variable
    k = vs.fresh("k")
    lits = [f"{k} = count : {{ {', '.join(body)} }}"]
    if isinstance(c.get("as"), str):     # the count, as a metavariable the message can print
        off = c.get("offset", 0)
        lits.append(f"{mv(c['as'])} = {k}" + (f" + {off}" if off else ""))
        prog.numeric.add(mv(c["as"]))
    for key, op in (("eq", "="), ("min", ">="), ("max", "<=")):
        if key in c:
            if isinstance(c[key], (int, float)):
                lits.append(f"{k} {op} {c[key]}")
            else:                       # e.g. {"$option": …}: no .rr form yet
                prog.untranslated += 1
                lits.append(f"ast({v}, {compact({'count': {key: c[key]}})})")
    return lits


def nth_child(prog, c, v, vs):
    p, s, k = vs.fresh("p"), vs.fresh("s"), vs.fresh("k")
    order = f"precedes({v}, {s})" if c.get("reverse") else f"precedes({s}, {v})"
    if "ofRule" in c:
        rel = prog.helper("counted", "the siblings the JSON `nthChild.ofRule` counts")
        prog.rules.append((f"{rel}(n)", lower(prog, c["ofRule"], "n", Vars())))
        return [f"child({p}, {v}, _), {rel}({v})",
                f"{k} = count : {{ child({p}, {s}, _), {order}, {rel}({s}) }}", f"{k} = {c['position'] - 1}"]
    return [f"child({p}, {v}, _), {k} = count : {{ child({p}, {s}, _), {order} }}", f"{k} = {c['position'] - 1}"]


def site_filters(prog, filters, s, vs):
    lits = []
    for bk, bv in filters.items():
        if bk == "form" and isinstance(bv, str):
            lits.append(f"site_form({s}, {q(bv)})")
        elif bk == "form" and isinstance(bv, list):
            lits.append(one_of(prog, vs, f"site_form({s}, {{}})", bv, "f", "binding forms the JSON accepts"))
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
            lits.append(one_of(prog, vs, f"site_{snake(bk)}({s}, {{}})", bv, "f", f"the JSON `binds.{bk}` list"))
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


def option_cases(prog, name, rule, v, conj=False):
    """`$option`: the configured value picks a case, else the default. A list value is spliced into the array that
    holds the `$option` (`spread: true`), so it means what that array means: alternatives under `any` (an empty
    list matches nothing), conjuncts under `all` (an empty list matches everything)."""
    rel = prog.helper("option", f"the JSON `$option` on {q(name)}: one rule per configured value, plus the default")
    keys = list((rule.get("cases") or {}))
    guard = [f"!option({q(name)}, {q(k)})" for k in keys]
    choices = [([f"option({q(name)}, {q(c)})"], b) for c, b in (rule.get("cases") or {}).items()]
    if "default" in rule:
        choices.append((guard, rule["default"]))
    for when, body in choices:
        if isinstance(body, list) and conj:
            lits = [l for b in body for l in lower(prog, b, "n", Vars())]
            prog.rules.append((f"{rel}(n)", when + (lits or ["node(_, n, _, _, _)"])))
        else:
            for b in (body if isinstance(body, list) else [body]):
                prog.rules.append((f"{rel}(n)", when + lower(prog, b, "n", Vars())))
    if not any(r[0] == f"{rel}(n)" for r in prog.rules):
        prog.decls.pop(rel, None)
        return ["false"]  # no case can match
    return [f"{rel}({v})"]


def kinds_only(alts):
    return all(isinstance(a, dict) and set(a) == {"kind"} for a in alts)


def grouped(prog, alts, v, vs):
    """One body per alternative, with plain kind and value alternatives folded into one literal each."""
    kinds = [a["kind"] for a in alts if isinstance(a, dict) and set(a) == {"kind"} and isinstance(a["kind"], str)]
    vals = [a["value"]["eq"] for a in alts if isinstance(a, dict) and set(a) == {"value"}
            and isinstance(a["value"], dict) and set(a["value"]) == {"eq"}]
    bodies = []
    if len(kinds) > 1:
        bodies.append([one_of(prog, vs, f"kind({v}, {{}})", kinds, "k", "node kinds from a JSON `any`")])
    if len(vals) > 1:
        bodies.append([one_of(prog, vs, f"value({v}, {{}})", vals, "v", "values from a JSON `any`")])
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
    name = prog.helper("alt", "the JSON `any`: n matches if any one of these rules does")
    for body in grouped(prog, alts, "n", Vars()):
        prog.rules.append((f"{name}(n)", body))
    return [f"{name}({v})"]


def stop_set(prog, stop):
    """The stop argument of within_stop / precedes_stop: one kind, a relation of kinds, or a helper relation."""
    if isinstance(stop, dict) and set(stop) == {"kind"}:
        return q(stop["kind"])
    if isinstance(stop, dict) and set(stop) == {"any"} and kinds_only(stop["any"]):
        return prog.facts("stop", "kind", [a["kind"] for a in stop["any"]],
                          "the kinds of the JSON `stopBy` rule: the tree search stops at these nodes")
    name = prog.helper("stop", "the JSON `stopBy` rule: the tree search stops at these nodes")
    if isinstance(stop, dict) and set(stop) == {"any"}:
        for body in grouped(prog, stop["any"], "n", Vars()):
            prog.rules.append((f"{name}(n)", body))
    else:
        prog.rules.append((f"{name}(n)", lower(prog, stop, "n", Vars())))
    return name


def relation(prog, k, sub, v, vs):
    f = sub.get("field")
    stop = sub.get("stopBy", "neighbor")
    c = vs.fresh("c" if k == "has" else "p" if k == "inside" else "s")
    lits = []
    skip = ("field", "stopBy")
    if k in ("has", "inside"):
        lo, hi = (c, v) if k == "has" else (v, c)
        if stop == "neighbor":
            lits.append(f"field({hi}, {q(f)}, {lo})" if f else f"child({hi}, {lo}, _)")
        elif stop == "end" and isinstance(sub.get("kind"), str):
            # An ancestor or descendant of one kind: the tape axes inside(n, a, set) / has(a, n, set).
            lits.append(f"{k}({v}, {c}, {q(sub['kind'])})")
            skip = skip + ("kind",)
        elif stop == "end":
            lits.append(f"within({lo}, {hi})")
        else:
            lits.append(f"within_stop({lo}, {hi}, {stop_set(prog, stop)})")
        if f and stop != "neighbor":
            lits.append(f"field(_, {q(f)}, {lo})")
    else:
        a, b = (v, c) if k == "precedes" else (c, v)
        if stop == "neighbor":
            lits.append(f"next_sibling({a}, {b})")
        elif stop == "end":
            lits.append(f"precedes({a}, {b})")
        else:
            lits.append(f"precedes_stop({a}, {b}, {stop_set(prog, stop)})")
    lits.extend(lower(prog, sub, c, vs, skip=skip))
    return lits


def resolves(prog, r, v, vs):
    lits = []
    rest = {}
    for k, val in r.items():
        if k == "imported" and isinstance(val, list):
            prog.use("names")
            lits.append(one_of(prog, vs, f"imported_sym({v}, {{}})", val, "q", "symbols from the JSON `resolves.imported`"))
        elif k == "import" and isinstance(val, list):
            prog.use("names")
            if r.get("prefix"):
                x = vs.fresh("q")
                alt = "|".join(re.escape(s) for s in val)
                lits.append(f"sym_of({v}, {x}), {match('^(' + alt + r')(\.|$)', x)}")
            else:
                lits.append(one_of(prog, vs, f"sym_of({v}, {{}})", val, "q", "symbols from the JSON `resolves.import`"))
        elif k == "prefix":
            continue
        elif k == "directives" and isinstance(val, str):
            lits.append(f"global_directives({v}, {q(val)})")
        elif k == "global":
            prog.use("names")
            if isinstance(val, list):
                lits.append(one_of(prog, vs, f"global_ref({v}, {{}})", val, "g", "names from the JSON `resolves.global`"))
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


def split_alts(s):
    """The top-level ` ; `-separated alternatives of a disjunction body."""
    out, d, start, ins = [], 0, 0, False
    for i, ch in enumerate(s):
        if ch == '"' and (i == 0 or s[i - 1] != "\\"):
            ins = not ins
        elif not ins and ch in "({":
            d += 1
        elif not ins and ch in ")}":
            d -= 1
        elif not ins and d == 0 and s.startswith(" ; ", i):
            out.append(s[start:i].strip())
            start = i + 3
    out.append(s[start:].strip())
    return out


def wrap(head, lits, width=88, edits=None):
    if not lits:
        return head + (f" => {', '.join(edits)}." if edits else ".")
    if edits:
        lits = lits[:-1] + [lits[-1] + " => " + ", ".join(edits)]
    out = head + " :- "
    line = out
    lines = []
    for i, lit in enumerate(lits):
        piece = lit + ("," if i < len(lits) - 1 else ".")
        if len(piece) > width - 4 and lit.startswith("(") and " ; " in lit:
            # A long disjunction: one alternative per line, under its opening parenthesis.
            alts = split_alts(lit[1:-1])
            if len(alts) > 1 and lit.endswith(")"):
                lines.append(line.rstrip())
                lines.append("    ( " + alts[0])
                lines += ["    ; " + a for a in alts[1:]]
                lines.append("    )" + piece[len(lit):])
                line = "    "
                continue
        if len(line) + len(piece) + 1 > width and line.strip() not in (head + " :-",):
            lines.append(line.rstrip())
            line = "    " + piece + " "
        else:
            line += piece + " "
    lines.append(line.rstrip())
    return "\n".join(lines)


def render_util(prog, name, rule, out):
    rel = prog.util_rel(name)
    prog.util_rels.add(rel)
    params = rule.get("params") if isinstance(rule, dict) else None
    if params:
        rule = rule["rule"] if "rule" in rule else {k: v for k, v in rule.items() if k != "params"}
        args = ", ".join(params)
        prog.decls[rel] = f".decl {rel}(n: node, {', '.join(col(p) + ': node' for p in params)})"
        prog.notes[rel] = [f"// {rel}(n, …): the JSON util `{name}`, called with `matches` and arguments {', '.join(params)}"]
        out.append((f"{rel}(n, {args})", lower(prog, rule, "n", Vars())))
        return
    prog.decls[rel] = f".decl {rel}(n: node)"
    prog.notes[rel] = [f"// {rel}(n): the JSON util `{name}`, used where the JSON says `matches: {q(name)}`"]
    if isinstance(rule, dict) and set(rule) == {"any"} and isinstance(rule["any"], list):
        for body in grouped(prog, rule["any"], "n", Vars()):
            out.append((f"{rel}(n)", body))
    else:
        out.append((f"{rel}(n)", lower(prog, rule, "n", Vars())))


def transform_targets(tr):
    """Metavariables a JSON `transform` defines, through `$option` cases too."""
    if not isinstance(tr, dict):
        return set()
    if "$option" in tr:
        out = set()
        for sub in list((tr.get("cases") or {}).values()) + [tr.get("default", {})]:
            out |= transform_targets(sub)
        return out
    return {t for t in tr if not t.startswith("$")}


def transform_lits(prog, tr, vs, defined):
    """Body literals for a JSON `transform`: each target is a functor of its source's text (or value).
    `defined` holds every target, so a source that is itself a target is read as a string, not a node."""
    if "$option" in tr:
        name, keys, parts = q(tr["$option"]), list(tr.get("cases") or {}), []
        every = transform_targets(tr)
        branches = [([f"option({name}, {q(c)})"], tr["cases"][c]) for c in keys]
        if "default" in tr:
            branches.append(([f"!option({name}, {q(c)})" for c in keys], tr["default"]))
        else:
            branches.append(([f"!option({name}, {q(c)})" for c in keys], {}))
        for guard, sub in branches:
            body = guard + transform_lits(prog, sub, vs, defined)
            body += [f"unset({t})" for t in sorted(every - transform_targets(sub))]   # not defined in this case
            parts.append(", ".join(body))
        return ["(" + " ; ".join(parts) + ")"]
    lits = []
    for t, spec in tr.items():
        if not (isinstance(spec, dict) and len(spec) == 1):
            prog.untranslated += 1
            lits.append(f"ast(n, {compact({'transform': {t: spec}})})")
            continue
        op, a = next(iter(spec.items()))
        src = mv(a.get("source", "")) if isinstance(a, dict) else ""

        def read():
            if src in defined:
                return src
            x = vs.fresh("t")
            lits.append(f"{'value' if a.get('value') else 'text'}({src}, {x})")
            return x
        if op == "count":
            lits.append(f"list_len({src}, {t})")
            prog.numeric.add(t)
        elif op == "line":
            lits.append(f"line({src}, {t})")
            prog.numeric.add(t)
        elif op == "replace" and isinstance(a.get("replace"), str) and isinstance(a.get("by"), str):
            x = read()
            f = f"{t} = @replace_all({x}, {q(a['replace'])}, {q(a['by'])})"
            if "default" in a and src not in defined:
                lits[-1:] = [f"(unset({src}), {t} = {q(a['default'])} ; !unset({src}), {lits[-1]}, {f})"]
            else:
                lits.append(f)
            prog.symbolic.add(t)
        elif op == "convert" and isinstance(a.get("toCase"), (str, dict)):
            x = read()
            seps = q(",".join(a.get("separatedBy") or []))
            case = a["toCase"]
            if isinstance(case, str):
                lits.append(f"{t} = @convert({x}, {q(case)}, {seps})")
            else:       # the case depends on an option
                name, keys = q(case["$option"]), list(case.get("cases") or {})
                parts = [f"option({name}, {q(c)}), {t} = @convert({x}, {q(case['cases'][c])}, {seps})" for c in keys]
                parts.append(", ".join([f"!option({name}, {q(c)})" for c in keys]
                                       + [f"{t} = @convert({x}, {q(case.get('default', 'lowerCase'))}, {seps})"]))
                lits.append("(" + " ; ".join(parts) + ")")
            prog.symbolic.add(t)
        elif op == "substring":
            x = read()
            start, end = a.get("startChar", 0), a.get("endChar")
            lits.append(f"{t} = {x}" if not start and end is None else
                        f"{t} = @slice({x}, {start}, {end if end is not None else f'strlen({x})'})")
            prog.symbolic.add(t)
        elif op == "toPrecision" and isinstance(a.get("digits"), str):
            x, d = read(), vs.fresh("d")
            lits.append(f"text({mv(a['digits'])}, {d}), {t} = @to_precision({x}, to_number({d}))")
            prog.symbolic.add(t)
        else:
            prog.untranslated += 1
            lits.append(f"ast(n, {compact({'transform': {t: spec}})})")
    return lits


def fix_edits(viol):
    """Today's ast fix template as a native edit: replace the witness, or the capture `range` names."""
    fx = viol.get("fix")
    if isinstance(fx, str):
        return [f"replace(n, {tick(fx)})"]
    if isinstance(fx, dict) and isinstance(fx.get("template"), str):
        rng = fx.get("range", "")
        target = cap(rng[len("capture."):]) if rng.startswith("capture.") else "n"
        return [f"replace({target}, {tick(fx['template'])})"]
    return None


def message_vars(msg):
    """The metavariables a JSON message interpolates ($X, $$$X), in order."""
    if not isinstance(msg, str):
        return []
    return list(dict.fromkeys(re.findall(r"\$\$?\$?([A-Z_][A-Z0-9_]*)", msg)))


def violation_columns(viol):
    """Metavariables that become columns of the finding: `each`, a capture anchor, and those in the message."""
    cols = []
    if isinstance(viol.get("each"), str):
        cols.append(mv(viol["each"]))
    anchor = viol.get("anchor") or ""
    if anchor.startswith("capture."):
        cols.append(cap(anchor[len("capture."):]))
    cols += message_vars(viol.get("message", ""))
    return list(dict.fromkeys(cols))


def violation_lines(viol, rel, cols, ast=False, numeric=(), symbolic=()):
    code = viol.get("code", rel)
    msg = viol.get("message", "")
    for c in sorted(cols, key=len, reverse=True) if isinstance(msg, str) else ():
        msg = re.sub(r"\$\$?\$?" + c + r"\b", "{" + c.lower() + "}", msg)
    colsig = "".join(f", {c.lower()}: {'number' if c in numeric else 'symbol' if c in symbolic else 'loc'}"
                     for c in cols)
    head = f".violation {rel}(at: node{colsig})"
    if code != rel:
        head += f" code {q(code)}"
    lines = [f"{head} {q(msg)}"]
    anchor = viol.get("anchor")
    if anchor and anchor.startswith("capture."):
        lines.append(f"    anchor {anchor[len('capture.'):].lower()}")
    elif anchor and anchor != "witness.span":
        lines.append(f"    anchor {anchor.replace('witness.', '')}")
    for tool, spec in (viol.get("suppress") or {}).items():
        names = spec.get("codes") or spec.get("rules") or []
        if names:
            lines.append(f"    suppress {tool} {', '.join(q(n) for n in names)}")
        else:
            lines.append(f"    suppress {tool}")
    if "fix" in viol and not ast:
        lines.append(f"    fix {compact(viol['fix'])}")
    if "each" in viol and not isinstance(viol["each"], str):
        lines.append(f"    each {compact(viol['each'])}")
    return lines


# ---- one violation for clauses that differ only in their message -----------------------------------------------

_VIOL_HEAD = re.compile(r'^\.violation (\w+)\((.*?)\)( code "(?:[^"\\]|\\.)*")? ("(?:[^"\\]|\\.)*")$')


def fold_messages(body):
    """Clauses with the same code, columns, suppressions, fix and options, whose messages differ, become one
    violation: the words that differ are a `detail` column, and each clause's rules put theirs in the head.
    Findings keep their keys (policy, code, path, line); only the relation's name and the message's spelling
    change, and the message reads the same for every finding."""
    rels_with_rules = {item["head"].split("(")[0] for kind, item in body if kind == "rule"}
    viols = {}
    for i, (kind, item) in enumerate(body):
        if kind != "viol":
            continue
        m = _VIOL_HEAD.match(item[0])
        if not m or m.group(1) not in rels_with_rules or "detail" in m.group(2):
            continue
        # the code a finding gets: the `code` line, or else the relation's own name
        key = (m.group(2), m.group(3) or f' code "{m.group(1)}"', tuple(item[1:]))
        viols.setdefault(key, []).append((i, m.group(1), json.loads(m.group(4))))
    groups = [g for g in viols.values() if len(g) > 1]
    if not groups:
        return body
    rename, drop, header = {}, set(), {}
    for g in groups:
        msgs = [x[2] for x in g]
        pre = os.path.commonprefix(msgs)
        suf = os.path.commonprefix([x[::-1] for x in msgs])[::-1]
        short = min(len(x) for x in msgs)
        if len(pre) + len(suf) > short:
            suf = suf[len(pre) + len(suf) - short:]
        if pre and not pre.endswith(" ") and any(len(x) > len(pre) and x[len(pre)] != " " for x in msgs):
            pre = pre[:pre.rfind(" ") + 1]
        if suf and not suf[0] in " ,.;:)" and any(len(x) > len(suf) and x[-len(suf) - 1] not in " " for x in msgs):
            cut = min([suf.find(c) for c in " ,.;:)" if c in suf] or [len(suf)])
            suf = suf[cut:]
        holes = [x[len(pre):len(x) - len(suf) if suf else len(x)] for x in msgs]
        if any("{" in h or "}" in h for h in holes) or any("{detail}" in x for x in msgs):
            continue
        same = len(set(msgs)) == 1  # one message: the clauses merge, with no column
        name = min((x[1] for x in g), key=lambda r: (len(r), r))
        first_i = g[0][0]
        m = _VIOL_HEAD.match(body[first_i][1][0])
        cols = m.group(2) + ("" if same else ", detail: symbol")
        tmpl = msgs[0] if same else pre + "{detail}" + suf
        cids, extra = [], []
        for i, _, _ in g:
            if i and body[i - 1][0] == "note":
                lines = body[i - 1][1]
                cids += re.findall(r"from clause `([^`]*)`", lines[0])
                extra += [l for l in lines[2:] if l not in extra]
        shown = ", ".join(f"`{c}`" for c in cids[:4]) + (f" and {len(cids) - 4} more" if len(cids) > 4 else "")
        code = m.group(3) or f" code {q(g[0][1])}"  # implicit codes are the relation's name: keep it when renamed
        header[first_i] = ([f".violation {name}({cols}){code if (m.group(3) or name != g[0][1]) else ''} {q(tmpl)}"] + body[first_i][1][1:],
                           ([f"// {name}(n): findings from clauses {shown}, which say the same.",
                             "// Each rule is one way a JSON `rule` can match."] if same else
                            [f"// {name}(n, detail): findings from clauses {shown}. Their messages differ only in",
                             "// `detail`: each rule's head gives its clause's words. Each rule is one way a JSON `rule` can match."])
                           + extra)
        for (i, rel, _), h in zip(g, holes):
            rename[rel] = (name, None if same else h)
            if i != first_i:
                drop.add(i)
    if not rename:
        return body
    out, moved = [], set()
    group_of = {rel: v[0] for rel, v in rename.items()}
    for i, (kind, item) in enumerate(body):
        if i in drop:
            if out and out[-1][0] == "note":
                out.pop()  # the dropped clause's own note
            continue
        if kind == "rule" and item["head"].split("(")[0] in rename:
            if id(item) in moved:
                continue
        if i in header:
            lines, note = header[i]
            if out and out[-1][0] == "note":
                out[-1] = ("note", note)
            out.append(("viol", lines))
            target = group_of[_VIOL_HEAD.match(item[0]).group(1)]
            for k2, it2 in body:
                if k2 == "rule" and group_of.get(it2["head"].split("(")[0]) == target:
                    rel = it2["head"].split("(")[0]
                    args = it2["head"].split("(", 1)[1][:-1]
                    it2["head"] = f"{target}({args})" if rename[rel][1] is None else f"{target}({args}, {q(rename[rel][1])})"
                    out.append(("rule", it2))
                    moved.add(id(it2))
            continue
        out.append((kind, item))
    return out


# ---- shared metavariables -------------------------------------------------------------------------------------

def _mask_ticks(s):
    return re.sub(r"`+[^`]*`+", lambda m: " " * len(m.group(0)), s)


def _upper_vars(s):
    return set(re.findall(r"(?<![\w$\"@])([A-Z][A-Z0-9_]*)\b(?!\s*\()", _mask_ticks(_strip_strings(s))))


def _strip_strings(s):
    return re.sub(r'"(?:[^"\\]|\\.)*"', '""', s)


def _binders(s):
    """Metavariables a rule body binds itself: in a pattern, by a capture `X = …`, or by a named regex group."""
    out = set()
    for t in re.findall(r"~\s*(`+)(.*?)\1", s):
        out |= set(re.findall(r"\$\$?\$?([A-Z_][A-Z0-9_]*)", t[1]))
    out |= set(re.findall(r"(?<![\w$])([A-Z][A-Z0-9_]*)\s*=\s*[a-z0-9]", _mask_ticks(_strip_strings(s))))
    out |= set(re.findall(r"\(\?P?<([A-Za-z_]\w*)>", s))
    for b in re.findall(r'"bind": \{([^}]*)\}', s):       # an ast(…) atom's argument binding
        out |= set(re.findall(r'"([A-Z_][A-Z0-9_]*)":', b))
    return out


def _calls(s, name):
    """Spans of calls to relation `name` in s (outside ticks and strings)."""
    masked = _mask_ticks(_strip_strings_keep(s))
    out = []
    for m in re.finditer(r"(?<![\w.])" + name + r"\(", masked):
        d = 0
        for j in range(m.end() - 1, len(masked)):
            if masked[j] == "(":
                d += 1
            elif masked[j] == ")":
                d -= 1
                if d == 0:
                    out.append((m.start(), j))
                    break
    return out


def _strip_strings_keep(s):
    return re.sub(r'"(?:[^"\\]|\\.)*"', lambda m: '"' + " " * (len(m.group(0)) - 2) + '"', s)


def share_metavariables(entries, helpers):
    """ast-grep keeps one set of metavariables per match, shared by the rule and every util it calls; Datalog
    variables belong to one rule. So a helper relation gets a column for each metavariable that crosses its
    boundary: one it reads but does not bind (passed in), and one it binds that a caller uses (passed out).
    Each call passes the caller's variable of that name, or `_` where the caller does not use it."""
    helpers = sorted(helpers)  # one order everywhere below: the fixpoint must not depend on the hash seed
    rules = []
    for e in entries:
        text = ", ".join(e["lits"])
        masked = _mask_ticks(_strip_strings_keep(text))
        calls = sorted(set(re.findall(r"(?<![\w.])([a-z_]\w*)\(", masked)) & set(helpers))
        rules.append({"e": e, "owner": e["head"].split("(")[0], "head": _upper_vars(e["head"].split("(", 1)[1]),
                      "up": _upper_vars(text) | set(re.findall(r"\$\$?\$?([A-Z_][A-Z0-9_]*)", " ".join(e.get("edits") or []))),
                      "binds": _binders(text), "calls": calls,
                      # a util's own `params` (`kw(n, N, V)`): the caller's arguments, local to the util
                      "params": _upper_vars(e["head"].split("(", 1)[1]) if e["head"].split("(")[0] in helpers else set()})
    by_owner = {}
    for r in rules:
        by_owner.setdefault(r["owner"], []).append(r)
    callers = {h: [] for h in helpers}
    for r in rules:
        for h in r["calls"]:
            callers[h].append(r)
    provides = {h: set() for h in helpers}
    free_in = {h: set() for h in helpers}
    needed = {h: set() for h in helpers}
    iface = {h: set() for h in helpers}
    for _ in range(50):
        old = {h: (set(provides[h]), set(free_in[h]), set(needed[h])) for h in helpers}
        # free_in is recomputed each round, not accumulated: a metavariable that looked free before a callee was
        # found to pass it out is bound after all (accumulating left it an input the caller fills with `_`).
        fresh = {h: set() for h in helpers}
        for h in helpers:
            for r in by_owner.get(h, ()):
                bound = r["binds"] | r["params"] | set().union(*[provides[g] & iface[g] for g in r["calls"]] or [set()])
                provides[h] |= (r["binds"] | set().union(*[provides[g] for g in r["calls"]] or [set()])) - r["params"]
                use = r["up"] | set().union(*[free_in[g] for g in r["calls"]] or [set()])
                fresh[h] |= use - bound
        free_in = fresh
        for h in helpers:
            for r in callers[h]:
                own = iface.get(r["owner"], set()) | needed.get(r["owner"], set())
                others = set().union(*[iface[g] for g in r["calls"] if g != h] or [set()])
                needed[h] |= r["head"] | r["up"] | others | own
        for h in helpers:
            iface[h] = free_in[h] | (needed[h] & provides[h])
        if all(old[h] == (provides[h], free_in[h], needed[h]) for h in helpers):
            break
    order = {h: sorted(iface[h]) for h in helpers}
    share_metavariables.inputs = {h: set(free_in[h]) for h in helpers}
    for r in rules:
        e = r["e"]
        if order.get(r["owner"]):
            e["head"] = e["head"][:-1] + ", " + ", ".join(order[r["owner"]]) + ")"
        hit = [h for h in r["calls"] if order[h]]
        if not hit:
            continue
        relevant = r["head"] | r["up"] | r["binds"] | iface.get(r["owner"], set())
        new = []
        for l in e["lits"]:
            for h in hit:
                rel = relevant | set().union(*[iface[g] for g in r["calls"] if g != h] or [set()])
                args = ", ".join(v if v in rel else "_" for v in order[h])
                for _, b in reversed(_calls(l, h)):
                    l = l[:b] + ", " + args + l[b:]
            new.append(l)
        e["lits"] = new
    return order


# ---- helpers whose rules leave a metavariable column unbound --------------------------------------------------

_NOT_VARS = {"count", "sum", "min", "max", "mean", "join", "by", "sep", "true", "false", "nil"}


def _split_args(s):
    out, d, cur, ins = [], 0, "", None
    for ch in s:
        if ins:
            cur += ch
            if ch == ins:
                ins = None
            continue
        if ch in "\"`":
            ins = ch
        elif ch in "([{":
            d += 1
        elif ch in ")]}":
            d -= 1
        if ch == "," and d == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    out.append(cur.strip())
    return out


def _rename_vars(text, mapping):
    """Variables of `text` renamed by `mapping` (a name -> its replacement), outside string and pattern literals."""
    if not mapping:
        return text
    pat = re.compile(r"(?<![\w$@\"])(" + "|".join(sorted(map(re.escape, mapping), key=len, reverse=True)) + r")\b(?!\s*\()")
    return "".join(p if p.startswith(("\"", "`")) else pat.sub(lambda m: mapping[m.group(1)], p)
                   for p in re.split(r'("(?:[^"\\]|\\.)*"|`+[^`]*`+)', text))


def inline_unsafe(entries, helpers):
    """A helper whose rule does not mention one of its metavariable columns (an `any` alternative that does not
    use a metavariable its caller passes in) has an unbound head variable, which a bottom-up engine cannot
    evaluate. Such a helper, when it is not recursive, is unfolded at each call: the call becomes the disjunction
    of its rules' bodies, with the call's arguments for the head's columns (`!call` becomes `count : { … } = 0`).
    The unfolded rule binds what the caller binds; ast-grep's meaning (the alternative ignores that metavariable)
    is unchanged. Repeated, since an unfolded caller can itself be such a helper."""
    by = {}
    for e in entries:
        by.setdefault(e["head"].split("(")[0], []).append(e)

    def head_vars(e):
        return [a.strip() for a in e["head"].split("(", 1)[1][:-1].split(",")]

    def bound_on_every_path(lits):
        """Metavariables every way through the body binds: aggregates bind nothing outside, and a disjunction
        binds what all its branches bind."""
        out = set()
        for l in lits:
            l = l.strip()
            alts = split_alts(l[1:-1]) if l.startswith("(") and l.endswith(")") and _balanced(l) else [l]
            if len(alts) > 1:
                out |= set.intersection(*[bound_on_every_path(_split_args(a)) for a in alts])
                continue
            flat, prev = l, None      # aggregates removed (a pattern's `$X` binds, so ticks stay)
            while flat != prev:
                prev, flat = flat, re.sub(r"\{[^{}]*\}", "", flat)
            out |= _upper_vars(flat) | _binders(flat)
        return out

    def unsafe(e):
        present = bound_on_every_path(e["lits"])
        return any(re.fullmatch(r"[A-Z][A-Z0-9_]*", v) and v not in present for v in head_vars(e))

    def calls_of(h):
        return {g for e in by.get(h, []) if not e.get("drop")
                for g in re.findall(r"(?<![\w.])([a-z_]\w*)\(", _mask_ticks(_strip_strings_keep(", ".join(e["lits"]))))
                if g in by}

    def recursive(h):
        seen, todo = set(), [h]
        while todo:
            for g in calls_of(todo.pop()):
                if g == h:
                    return True
                if g not in seen:
                    seen.add(g)
                    todo.append(g)
        return False

    counter = [0]

    def expansion(h, args):
        alts = []
        outputs = set().union(*[_binders(", ".join(e["lits"])) for e in by[h] if not e.get("drop")])
        for e in by[h]:
            if e.get("drop"):
                continue
            counter[0] += 1
            k = counter[0]
            params = head_vars(e)
            mapping = {}
            for p, a in zip(params, args):
                mapping[p] = a if a != "_" else f"w{k}_{p.lower()}"
            body = ", ".join(e["lits"])
            for v in set(re.findall(r"(?<![\w$@\"])([a-z][a-z0-9_]*)\b(?!\s*\()", _mask_ticks(_strip_strings(body)))):
                if v not in mapping and v not in _NOT_VARS:
                    mapping[v] = f"{v}_{k}"
            # a column this rule leaves empty that another rule binds is an output: empty here, as in ast-grep
            missing = [p for p in params if re.fullmatch(r"[A-Z][A-Z0-9_]*", p)
                       and p not in bound_on_every_path(e["lits"]) and p in outputs]
            alts.append(_rename_vars(body + "".join(f", unset({p})" for p in missing), mapping))
        return alts

    for _ in range(10):
        targets = [h for h in sorted(helpers) if h in by and any(unsafe(e) for e in by[h] if not e.get("drop"))
                   and not recursive(h)]
        if not targets:
            break
        h = targets[0]
        for e in entries:
            if e.get("drop") or e["head"].split("(")[0] == h:
                continue
            new = []
            for l in e["lits"]:
                for s, end in reversed(_calls(l, h)):
                    args = _split_args(l[s + len(h) + 1:end])
                    alts = expansion(h, args)
                    if s > 0 and l[s - 1] == "!":
                        rep = ", ".join(f"count : {{ {a} }} = 0" for a in alts)
                        l = l[:s - 1] + rep + l[end + 1:]
                    else:
                        rep = alts[0] if len(alts) == 1 else "(" + " ; ".join(alts) + ")"
                        l = l[:s] + rep + l[end + 1:]
                new.append(l)
            e["lits"] = new
        for e in by[h]:
            e["drop"] = True


# ---- demand: helpers that take a metavariable from their caller ------------------------------------------------

def _split_top_text(s, sep):
    out, d, cur, ins = [], 0, "", None
    i = 0
    while i < len(s):
        ch = s[i]
        if ins:
            cur += ch
            if ch == ins and s[i - 1] != "\\":
                ins = None
            i += 1
            continue
        if ch in "\"`":
            ins = ch
        elif ch in "([{":
            d += 1
        elif ch in ")]}":
            d -= 1
        if ch == sep and d == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
        i += 1
    out.append(cur.strip())
    return [x for x in out if x]


_AGG = re.compile(r"^(?:[A-Za-z]\w* = )?(?:count|sum|min|max|mean)(?: [^:{]*)? : \{(.*)\}(.*)$", re.S)
_POSITIVE = re.compile(r"^[a-z_]\w*\(.*\)$", re.S)
_EQ = re.compile(r"^[A-Za-z]\w* = [A-Za-z_]\w*$|^[a-z]\w* ~ (`+).*\1( at \"[^\"]*\")?$", re.S)


def _call_sites(lits, h, ctx=()):
    """(context positives, call text) for each call of h in a body: the positive literals of every scope the call
    sits in (the rule body, then each enclosing aggregate or disjunction branch). None where a call sits where
    this cannot say (inside another construct)."""
    parts = _split_top_text(", ".join(lits) if isinstance(lits, list) else lits, ",")
    here = [p for p in parts if (_POSITIVE.match(p) and _balanced(p[p.index("("):]) and not _calls(p, h)) or _EQ.match(p)]
    scope = list(ctx) + here
    out = []
    for p in parts:
        if not _calls(p, h):
            continue
        m = _AGG.match(p)
        bare = p[1:] if p.startswith("!") else p
        if _POSITIVE.match(bare) and len(_calls(bare, h)) == 1 and _calls(bare, h)[0] == (0, len(bare) - 1):
            out.append((scope, bare))
        elif m and not _calls(m.group(2), h):
            out += _call_sites(m.group(1), h, scope)
        elif p.startswith("(") and p.endswith(")") and len(split_alts(p[1:-1])) > 1:
            for b in split_alts(p[1:-1]):
                out += _call_sites(b, h, scope)
        else:
            out.append(None)
    return out


def demand(entries, helpers, inputs, decls, lib):
    """Magic sets, by hand: a helper that reads a metavariable its caller binds (`shadowed(n, N)`) is evaluated
    bottom-up for every value of that column unless something limits it. Each call site gives the values it asks
    for: demand_h(x, N) :- <the positive literals in scope at the call>. Every rule of h starts with
    demand_h(n, N), so h holds exactly where a caller asks; its callers read the same answers as before.
    A helper whose call sites cannot all say what they ask for is left as it is."""
    by = {}
    for e in entries:
        if not e.get("drop"):
            by.setdefault(e["head"].split("(")[0], []).append(e)
    reach = {}

    def callees(h):
        if h not in reach:
            reach[h] = set()
            todo = [h]
            while todo:
                for e in by.get(todo.pop(), []):
                    for g in re.findall(r"(?<![\w.])([a-z_]\w*)\(", _mask_ticks(_strip_strings_keep(", ".join(e["lits"])))):
                        if g in by and g not in reach[h]:
                            reach[h].add(g)
                            todo.append(g)
        return reach[h]
    def try_demand(h):
        ins = inputs.get(h) or set()
        if not ins or h not in by:
            return False
        params = [a.strip() for a in by[h][0]["head"].split("(", 1)[1][:-1].split(",")]
        if any(p not in ins and not re.fullmatch(r"[A-Z][A-Z0-9_]*", p) for p in params[1:]):
            return False
        pos = [0] + [i for i, p in enumerate(params) if p in ins]
        dname = f"demand_{h}"
        rules, ok = [], True
        for e in entries:
            if e.get("drop"):
                continue
            owner = e["head"].split("(")[0]
            for site in _call_sites(e["lits"], h):
                if site is None:
                    ok = False
                    break
                scope, call = site
                args = _split_args(call[len(h) + 1:-1])
                want = [args[i] for i in pos]
                if any(a == "_" for a in want):
                    ok = False
                    break
                # the call's scope, without what could depend on h through recursion other than h's own demand
                lits = [l for l in scope if not (l.split("(")[0] in by and (h in callees(l.split("(")[0])
                                                                       or l.split("(")[0] == h))]
                if owner == h:      # recursion: the call asks for what the rule's own demand allows
                    hp = [a.strip() for a in e["head"].split("(", 1)[1][:-1].split(",")]
                    lits = [f"{dname}({', '.join(hp[i] for i in pos)})"] + lits
                elif owner in callees(h):   # mutual recursion: not handled
                    ok = False
                    break
                head = f"{dname}({', '.join(want)})"
                if check_rule(wrap(head, lits)) or not lits:
                    ok = False
                    break
                rules.append((head, lits))
            if not ok:
                break
        if not ok or not rules:
            return False
        cols = [c.strip() for c in decls[h][decls[h].index("(") + 1:-1].split(",")]
        lib[dname] = (f".decl {dname}({', '.join(cols[i] for i in pos)})",
                      [f"// {dname}: the rows of {h} its callers ask for (the metavariable columns they pass in)"],
                      [r for i, r in enumerate(rules) if r not in rules[:i]])
        for e in by[h]:
            hp = [a.strip() for a in e["head"].split("(", 1)[1][:-1].split(",")]
            e["lits"] = [f"{dname}({', '.join(hp[i] for i in pos)})"] + e["lits"]
        return True

    done = []
    while True:   # callers first: a caller's own demand is what binds its call's metavariables
        new = [h for h in sorted(set(helpers) - set(done)) if try_demand(h)]
        if not new:
            break
        done += new
    return done


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
    fix_rels = set()
    for c in clauses:
        cid = c.get("id", "clause")
        if "define" in c:
            d = c["define"]
            prog.scope = {}
            utils = []
            for name, rule in (d.get("utils") or {}).items():
                render_util(prog, name, rule, utils)
            if d.get("options"):
                body.append(("opt", f".options {compact(d['options'])}"))
            for h, lits in utils:
                body.append(("rule", {"head": h, "lits": lits}))
            continue
        forbid = c.get("forbid", {})
        viol = c.get("violation", {})
        rel = ident(viol.get("code", cid))
        if rel in seen_rel:
            rel = f"{rel}_{snake(cid)}"
        seen_rel.add(rel)
        if "ast" not in forbid:
            body.append(("note", [f"// {rel}: a finding from a text rule. It has no AST pattern, so it stays as JSON."]))
            body.append(("viol", violation_lines(viol, rel, [])))
            body.append(("raw", f"{rel}(d) :- text_rule(d, {compact(forbid)})."))
            prog.untranslated += 1
            continue
        a = forbid["ast"]
        local = []
        # A clause's utils are its own (ast-grep scopes them to the rule): a name another clause already gave a
        # relation gets the clause's id, so two clauses' `outer` stay two relations.
        prog.scope = {name: (snake(name) if snake(name) not in prog.util_rels else f"{snake(name)}_{snake(cid)}")
                      for name in (a.get("utils") or {})}
        for name, rule in (a.get("utils") or {}).items():
            render_util(prog, name, rule, local)
        cols = violation_columns(viol)
        note = [f"// {rel}(n): a finding at node n, from clause `{cid}`.",
                "// Each rule below is one way the JSON `rule` can match."]
        if cols:
            note.append(f"// Columns after `at` are the metavariables the JSON's message, anchor or `each` names: one "
                        f"finding per distinct row.")
        opts = {k: v for k, v in a.items() if k not in ("language", "rule", "utils", "constraints", "transform")}
        vs = Vars()
        tr = a.get("transform") if isinstance(a.get("transform"), dict) else {}
        tlits = transform_lits(prog, tr, vs, transform_targets(tr)) if tr else []
        if tr:
            note.append(f"// The JSON `transform` computes {', '.join(sorted(transform_targets(tr)))} from other "
                        f"metavariables' text; its literals end each rule.")
        vl = violation_lines(viol, rel, cols, ast=True, symbolic=prog.symbolic)
        edits = fix_edits(viol)
        if edits:
            note.append("// `=> replace(…)` is today's fix template: the edit suggested with each finding.")
        if opts:
            vl.append(f"    options {compact(opts)}")
        if a.get("constraints"):
            note.append(f"// The JSON `constraints` on {', '.join(a['constraints'])} are added to every rule.")
        if opts:
            note.append("// `options` carries JSON settings that have no .rr form yet.")
        body.append(("note", note))
        body.append(("viol", vl))
        rule = a.get("rule", {})
        if isinstance(rule, dict) and set(rule) == {"any"} and isinstance(rule["any"], list):
            bodies = grouped(prog, rule["any"], "n", vs)
        else:
            bodies = [lower(prog, rule, "n", vs)]
        cons = []
        for var, crule in (a.get("constraints") or {}).items():
            cons.extend(lower(prog, crule, mv(var), vs))
        hd = f"{rel}(n{''.join(', ' + c for c in cols)})"
        for b in bodies:
            body.append(("rule", {"head": hd, "lits": b + cons + tlits, "edits": edits}))
        for h, lits in local:
            body.append(("rule", {"head": h, "lits": lits}))

    # Remove what the construct-by-construct lowering alone put there (simplify.py): duplicate helpers, helpers
    # used once, and grounding atoms that something else already provides.
    entries = [item for kind, item in body if kind == "rule"]
    protected = set(seen_rel) | {e["head"].split("(")[0] for e in entries if "," in e["head"]}
    simplify(entries, prog, protected)
    keep = set(map(id, entries))
    body = [(kind, item) for kind, item in body if kind != "rule" or id(item) in keep]

    # Helper relations (utils, alternatives, stop sets) share the JSON's metavariables with their callers.
    entries = [item for kind, item in body if kind == "rule"]
    helper_entries = [{"head": r[0], "lits": r[1]} for r in prog.rules if r[0] != "#facts"]
    helpers = set(prog.decls)
    params = share_metavariables(entries + helper_entries, helpers)
    for name, ps in params.items():
        if ps and name in prog.decls:
            prog.decls[name] = prog.decls[name][:-1] + "".join(
                f", {col(p)}: {'number' if p in prog.numeric else 'symbol' if p in prog.symbolic else 'loc'}"
                for p in ps) + ")"
            ins = [p for p in ps if p in share_metavariables.inputs[name]]
            outs = [p for p in ps if p not in ins]
            if ins:
                prog.notes.setdefault(name, []).append(f"//   {', '.join(ins)}: metavariable{'s' if len(ins) > 1 else ''} "
                                                       f"of the calling rule, passed in (bound by the caller)")
            if outs:
                prog.notes.setdefault(name, []).append(f"//   {', '.join(outs)}: metavariable{'s' if len(outs) > 1 else ''} "
                                                       f"this relation binds for the calling rule")
    inline_unsafe(entries + helper_entries, helpers)
    demand(entries + helper_entries, helpers, getattr(share_metavariables, "inputs", {}), prog.decls, prog.lib)
    body = [(kind, item) for kind, item in body if kind != "rule" or not item.get("drop")]
    body = fold_messages(body)
    hi = iter(helper_entries)
    helper_rules = []
    for r in prog.rules:
        if r[0] == "#facts":
            helper_rules.append(r)
        else:
            e = next(hi)
            if not e.get("drop"):
                helper_rules.append((e["head"], e["lits"]))

    out = head + ["// Generated from today's JSON by docs/rule-syntax/site/translate.py. Display only: not compiled or run.", ""]
    if prog.uses:
        out += [f'#include "{u}.rr"' for u in sorted(prog.uses)] + [""]
    declared = set()
    ungrounded = 0

    def emit_rule(h, lits, edits=None):
        nonlocal ungrounded
        text = wrap(h, lits, edits=edits)
        inputs = getattr(share_metavariables, "inputs", {}).get(h.split("(")[0], ())
        bad = check_rule(text, fix_rels, inputs)
        if "n" in bad and h.split("(", 1)[1].startswith("n"):
            # A helper that only tests the node it is given (`!kind(n, …)`, a regex): n ranges over all nodes.
            lits = [f"node(_, n, _, _, _)"] + list(lits)
            text = wrap(h, lits, edits=edits)
            bad = check_rule(text, fix_rels, inputs)
        once = [x for x in bad if re.fullmatch(r"[a-z]\w*", x)
                and len(re.findall(r"(?<![\w\"@$])" + x + r"\b", _mask_ticks(_strip_strings(text)))) == 1]
        if once:
            # A variable used once, under `!`, means "any value": Soufflé writes it `_`.
            pat = re.compile(r"(?<![\w@$])(" + "|".join(once) + r")\b(?!\s*\()")
            lits = ["".join(p if p.startswith(("\"", "`")) else pat.sub("_", p)
                            for p in re.split(r'("(?:[^"\\]|\\.)*"|`+[^`]*`+)', l)) for l in lits]
            text = wrap(h, lits, edits=edits)
            bad = check_rule(text, fix_rels, inputs)
        empty = [x for x in bad if re.fullmatch(r"[A-Z][A-Z0-9_]*", x)]
        if empty:
            # ast-grep leaves a metavariable empty when the match does not bind it; core's unset is that value.
            lits = list(lits) + [f"unset({x})" for x in empty]
            text = wrap(h, lits, edits=edits)
            bad = check_rule(text, fix_rels, inputs)
            out.append(f"// {', '.join(empty)}: not bound by this rule, so empty (unset), as in ast-grep.")
        if bad:
            ungrounded += 1
            out.append(f"// Not grounded: no positive atom binds {', '.join(bad)}.")
        out.append(text)

    def emit_decl(name):
        if name in prog.decls and name not in declared:
            if out[-1] != "":
                out.append("")
            out.extend(prog.notes.get(name, []))
            out.append(prog.decls[name])
            declared.add(name)

    for kind, item in body:
        if kind == "rule":
            emit_decl(item["head"].split("(")[0])
            emit_rule(item["head"], item["lits"], item.get("edits"))
        elif kind == "note":
            if out[-1] != "":
                out.append("")
            out.extend(item)
        elif kind == "viol":
            first = item[0]
            for c in prog.numeric:
                first = re.sub(r"\b" + c.lower() + r": loc\b", c.lower() + ": number", first)
            out.extend([first] + item[1:])
        else:
            out.append(item)
    for name, (decl, note, rules) in prog.lib.items():
        # relations a term's lowering shares (main_terms.py): plain Datalog over the tape, no metavariables
        out += ["", *note, decl]
        out += [wrap(h, lits) for h, lits in rules]
    if helper_rules:
        out.append("")
        for r in helper_rules:
            if r[0] == "#facts":
                _, name, vals = r
                emit_decl(name)
                line = ""
                for v in vals:
                    args = ", ".join(map(q, v)) if isinstance(v, (tuple, list)) else q(v)
                    f = f"{name}({args})."
                    if line and len(line) + len(f) + 1 > 88:
                        out.append(line)
                        line = ""
                    line = (line + " " + f).strip()
                if line:
                    out.append(line)
                continue
            h, lits = r
            emit_decl(h.split("(")[0])
            emit_rule(h, lits)
    if any("ast(" in l for l in out if not l.lstrip().startswith("//")):
        i = out.index("") if "" in out else len(out)
        out.insert(i, "// ast(n, {…}) atoms are JSON terms with no .rr form yet; the engine matches them as it does today.")
    text = "\n".join(out).rstrip() + "\n"
    return text, prog.untranslated, ungrounded
