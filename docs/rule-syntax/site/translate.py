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
import sys

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
    """fmt with one `{}`: an atom per value. One value: the atom; a few: a disjunction; many: a relation of facts.
    A `{"$option": name, "spread": true}` entry splices in the option's configured list (option_item)."""
    spread = [x["$option"] for x in values if isinstance(x, dict) and "$option" in x and x.get("spread") is True]
    if spread:
        values = [x for x in values if not isinstance(x, dict)]
        name = prog.facts("set", "v", values, note)
        for o in spread:
            prog.rules.append((f"{name}(x)", [f"option_item({q(o)}, _, x)"]))
        x = vs.fresh(p)
        return f"{fmt.format(x)}, {name}({x})"
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
        elif k == "layout" and isinstance(val, dict) and main_terms.layout(prog, val, v, Vars()) is not None:
            lits.extend(main_terms.layout(prog, val, v, vs))
        elif k == "nthChild" and isinstance(val, dict) and isinstance(val.get("position"), int):
            lits.extend(nth_child(prog, val, v, vs))
        elif k == "binds" and val == {}:
            # the name binds (an assignment or for target, a parameter, an import, `x += 1`)
            prog.use("names")
            lits.append(f"(bind_only({v}) ; aug_target({v}))")
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
        elif k == "value" and isinstance(val, dict) and (len(val) > 1 or "type" in val
                                                         or next(iter(val)) in main_terms.VALUE_OPS):
            lits.extend(main_terms.value(prog, val, v, vs, opt_alts, match))
        elif k == "$option" and isinstance(val, str) and ("cases" in rule or "default" in rule):
            lits.extend(option_cases(prog, val, rule, v))
        elif k in ("cases", "default") and "$option" in rule:
            continue
        elif k == "value" and isinstance(val, dict) and len(val) == 1:
            op, arg = next(iter(val.items()))
            if op == "eq":
                # value is a symbol, numbers normalised ("255" for 0xff): a JSON number compares as its text
                num = lambda c: q(str(c)) if isinstance(c, (int, float)) and not isinstance(c, bool) else q(c)
                lits.append(opt_alts(arg, lambda c: f"value({v}, {num(c)})"))
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


def stop_axis(prog, axis, lo, hi, stop, vs):
    """within_stop / precedes_stop over the stop rule's relation (see stop_with_columns for a stop rule that reads
    a metavariable of the caller)."""
    return [f"{axis}_stop({lo}, {hi}, {stop_set(prog, stop)})"]


def passed_in_patterns(entries, helpers):
    """`pattern: "$X"` where the caller passes X in matches a node with X's text (ast-grep's metavariable
    consistency), not X itself: `n ~ \\`$X\\`` becomes the text join."""
    inputs = getattr(share_metavariables, "inputs", {})
    k = [0]
    for e in entries:
        h = e["head"].split("(")[0]
        if e.get("drop") or not inputs.get(h):
            continue
        for x in sorted(inputs[h]):
            def join(m, x=x):
                k[0] += 1
                return f"text({m.group(1)}, t_{x.lower()}{k[0]}), text({x}, t_{x.lower()}{k[0]})"
            e["lits"] = [re.sub(r"(?<![\w$])([a-z][a-z0-9_]*) ~ `\$" + x + r"`", join, l) for l in e["lits"]]

            def join_eq(m, x=x):
                k[0] += 1
                return f"text({m.group(1)}, t_{x.lower()}{k[0]}), text({x}, t_{x.lower()}{k[0]})"
            e["lits"] = [re.sub(r"(?<![\w$@])" + x + r" = ([a-z][a-z0-9_]*)\b(?!\s*\()", join_eq, l) for l in e["lits"]]


def drop_dead_columns(entries, helpers, decls, notes):
    """ast-grep passes a util's argument (`matches: {u: {N: "$C"}}`) or a shared metavariable whether or not the
    caller bound it; an unbound one is unset there, and the util matches as if it were not passed. In Datalog such
    a column is bound by nothing, so it is ungrounded. Found as a least fixpoint: a column passed in (an input or
    a util param) is fed when some call passes it a constant or a variable bound in the caller; a variable is bound
    in a rule when the rule reads or binds it other than by passing it on, passes it to a column that binds it,
    or has it in its head at a fed column. A column never fed is removed, from the declaration, the heads and
    every call; a variable that is not bound where it is passed becomes `_`."""
    live = [e for e in entries if not e.get("drop")]
    heads = {}
    for e in live:
        h = e["head"].split("(")[0]
        if h in helpers:
            heads.setdefault(h, []).append(e)
    inputs = getattr(share_metavariables, "inputs", {})
    params = getattr(share_metavariables, "params", {})

    def head_args(e):
        return _split_args(e["head"].split("(", 1)[1][:-1])

    def binds_in(g, x):
        # a param the util's body mentions is captured there (or by the rules it calls): an output
        return any(x in _upper_vars(t) or re.search(r"\$\$?\$?" + x + r"\b", t)
                   for t in (", ".join(e["lits"]) for e in heads[g]))

    def input_col(g, j):
        """A column the callee reads and does not bind: passed in, or a util param it never captures."""
        names = head_args(heads[g][0])
        return 0 < j < len(names) and (names[j] in inputs.get(g, ()) or
                                       names[j] in params.get(g, ()) and not binds_in(g, names[j]))

    def calls_in(e):
        text = ", ".join(e["lits"])
        out = []
        for g in heads:
            for a, b in _calls(text, g):
                if not text[:a].endswith("@"):
                    out.append((a, b, g, _split_args(text[a + len(g) + 1:b]), text[:a].rstrip().endswith("!")))
        return text, out

    info = []    # per rule: owner, head args, {var: [(g, j)] or None (used directly)}
    for e in live:
        text, calls = calls_in(e)
        rest = list(_mask_ticks(_strip_strings_keep(text)))
        for a, b, _, _, _ in calls:
            for k in range(a, b + 1):
                rest[k] = " "
        direct = _upper_vars("".join(rest)) | set(re.findall(r"\$\$?\$?([A-Z_][A-Z0-9_]*)", text)) | \
            set(re.findall(r"\$\$?\$?([A-Z_][A-Z0-9_]*)", " ".join(e.get("edits") or [])))
        passed = {}
        for _, _, g, args, neg in calls:
            for j, x in enumerate(args):
                if re.fullmatch(r"[A-Z][A-Z0-9_]*", x):
                    passed.setdefault(x, []).append((g, j, neg))
                elif x != "_":
                    direct |= _upper_vars(x)      # nested in a bigger argument: a use
        info.append((e, e["head"].split("(")[0], head_args(e), direct, passed))
    cols = {(g, j) for g in heads for j in range(len(head_args(heads[g][0]))) if input_col(g, j)}
    named = {g for e in live for g in heads
             if re.search(r"(?<![\w.])" + g + r"(?![\w(])", _mask_ticks(_strip_strings(", ".join(e["lits"]))))}
    fed = {(g, j) for g, j in cols if g in named}

    def bound(owner, hargs, direct, passed, var):
        if var in direct:
            return True
        if any((g, j) not in cols and not neg for g, j, neg in passed.get(var, ())):
            return True           # a column that binds it (an output, in a positive call)
        return any(x == var and (owner, i) in fed for i, x in enumerate(hargs)) or \
            (owner not in heads and var in hargs)
    changed = True
    while changed:
        changed = False
        for e, owner, hargs, direct, passed in info:
            _, calls = calls_in(e)
            for _, _, g, args, _ in calls:
                for j, x in enumerate(args):
                    if (g, j) in cols and (g, j) not in fed and x != "_" and (
                            not re.fullmatch(r"[A-Z][A-Z0-9_]*", x) or bound(owner, hargs, direct, passed, x)):
                        fed.add((g, j))
                        changed = True
    dead = cols - fed
    by = {}
    for g, j in dead:
        by.setdefault(g, set()).add(j)
    gone = {g: {head_args(heads[g][0])[j] for j in js} for g, js in by.items()}

    def rewrite(text, e_info):
        _, owner, hargs, direct, passed = e_info
        for g in heads:
            for a, b in reversed(_calls(text, g)):
                if text[:a].endswith("@"):
                    continue
                args = _split_args(text[a + len(g) + 1:b])
                neg = text[:a].rstrip().endswith("!")
                new = [("_" if ((g, j) in cols or neg) and re.fullmatch(r"[A-Z][A-Z0-9_]*", x)
                        and not bound(owner, hargs, direct, passed, x) else x)
                       for j, x in enumerate(args) if j not in by.get(g, ())]
                text = text[:a] + f"{g}({', '.join(new)})" + text[b + 1:]
        return text
    for item in info:
        e = item[0]
        e["lits"] = [rewrite(l, item) for l in e["lits"]]
        g = item[1]
        if g in by:
            args = [x for j, x in enumerate(item[2]) if j not in by[g]]
            e["head"] = f"{g}({', '.join(args)})"
    for g, js in by.items():
        m = re.match(r"(\.decl \w+)\((.*)\)(.*)", decls[g])
        parts = [x for j, x in enumerate(_split_args(m.group(2))) if j not in js]
        decls[g] = f"{m.group(1)}({', '.join(parts)}){m.group(3)}"
        kept = []
        for l in notes.get(g, []):
            m = re.match(r"(//\s+)([A-Z0-9_, ]+): (metavariables? of the calling rule, passed in.*)", l)
            if m:
                names = [x.strip() for x in m.group(2).split(",") if x.strip() not in gone[g]]
                if not names:
                    continue
                l = f"{m.group(1)}{', '.join(names)}: {m.group(3)}"
            kept.append(l)
        notes[g] = kept


def stop_with_columns(entries, decls):
    """A stopBy rule that reads a metavariable of the caller gets more columns from share_metavariables and is no
    set of nodes: `within_stop(a, b, s)` becomes the axis with no node of s (at the caller's metavariables)
    strictly between a and b."""
    wide = {}
    for name, d in decls.items():
        m = re.match(r"\.decl (\w+)\((.*)\)", d)
        if m and m.group(1) == name and m.group(2).count(":") > 1:
            wide[name] = [c.split(":")[0].strip() for c in m.group(2).split(",")]
    heads = {e["head"].split("(")[0]: e["head"] for e in entries if not e.get("drop")}
    # the metavariables the stop rule's head reads, as its callers name them
    wide = {r: [a.strip() for a in heads[r].split("(", 1)[1].rstrip(")").split(",")] for r in wide if r in heads}
    if not wide:
        return
    k = [0]

    def one(m):
        axis, a, b, rel = m.group(1), m.group(2), m.group(3), m.group(4)
        if rel not in wide:
            return m.group(0)
        k[0] += 1
        x = f"m_s{k[0]}"
        args = ", ".join([x] + wide[rel][1:])
        return (f"{axis}({a}, {b}), count : {{ {axis}({a}, {x}), {axis}({x}, {b}), {rel}({args}) }} = 0")
    pat = re.compile(r"(within|precedes)_stop\((\w+), (\w+), (\w+)\)")
    for e in entries:
        e["lits"] = [pat.sub(one, l) for l in e["lits"]]


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
            lits.extend(stop_axis(prog, "within", lo, hi, stop, vs))
        if f and stop != "neighbor":
            lits.append(f"field(_, {q(f)}, {lo})")
    else:
        a, b = (v, c) if k == "precedes" else (c, v)
        if stop == "neighbor":
            lits.append(f"next_sibling({a}, {b})")
        elif stop == "end":
            lits.append(f"precedes({a}, {b})")
        else:
            lits.extend(stop_axis(prog, "precedes", a, b, stop, vs))
    lits.extend(lower(prog, sub, c, vs, skip=skip))
    if set(sub) - {"field", "stopBy"} == {"regex"} and not f and stop in ("neighbor", "end") \
            and not (k in ("has", "inside") and stop == "end") and k != "inside":
        # ast-grep's siblings and children include anonymous tokens (`:`, `,`, `in`); the tape's are named
        # only, so a regex-only target may also be a token: op(p, s) for a child, token_before/after for a sibling
        t = vs.fresh("t")
        if k == "has":
            tok = [f"op({v}, {t})"]
        else:
            step = "token_after" if k == "precedes" else "token_before"
            if stop == "neighbor":
                tok = [f"{step}({v}, {t})"]
            else:
                # the token before (after) v, the one before that, or the one before (after) a sibling that
                # precedes (follows) v: a relation of its own (a disjunction inside an aggregate miscounts)
                name = f"sibling_{step}"
                sib = "precedes(s, n)" if k == "follows" else "precedes(n, s)"
                side = "earlier" if k == "follows" else "later"
                prog.lib[name] = (f".decl {name}(n: loc, t: span)",
                                  [f"// {name}(n, t): an anonymous token among n's {side} siblings (approximately: "
                                   f"the next two tokens, and the one next to each sibling)"],
                                  [(f"{name}(n, t)", [f"{step}(n, t)"]),
                                   (f"{name}(n, t)", [f"{step}(n, t0)", f"{step}(t0, t)"]),
                                   (f"{name}(n, t)", [sib, f"{step}(s, t)"])])
                tok = [f"{name}({v}, {t})"]
        if k == "has" and isinstance(sub["regex"], str):     # op's token is a symbol: matched, not globbed
            tok.append(f"match({q(whole(caps(sub['regex'])))}, {t})")
        else:
            tok += lower(prog, {"regex": sub["regex"]}, t, vs)
        # one relation with a rule per form (a disjunction inside an aggregate is not counted right)
        ren = lambda ls: [re.sub(r"(?<![\w$])" + re.escape(v) + r"\b", "n", l) for l in ls]
        rules = [("{0}(n)", ren(lits)), ("{0}(n)", ren(tok))]
        sig = repr(rules)
        seen = prog.__dict__.setdefault("token_rels", {})
        if sig not in seen:
            seen[sig] = f"tok_{k}_{len(seen) + 1}"
            name = seen[sig]
            what = {"has": "a child", "precedes": "a later sibling", "follows": "an earlier sibling"}[k]
            prog.lib[name] = (f".decl {name}(n: node)",
                              [f"// {name}(n): {what} of n, a named node or an anonymous token, matches "
                               f"{json.dumps(sub['regex'])}"],
                              [(h.format(name), b) for h, b in rules])
        lits = [f"{seen[sig]}({v})"]
    return lits


def resolves(prog, r, v, vs):
    lits = []
    rest = {}
    if r.get("seenBefore") is True and isinstance(r.get("imported"), list):
        return main_terms.seen_before(prog, r["imported"], v, vs)
    if r.get("relative") == "text" and isinstance(r.get("imported"), list):
        return main_terms.statement_imported(prog, r["imported"], v, vs, bool(r.get("prefix")))
    for k, val in r.items():
        if k == "import" and val is True:
            # Ruff's resolve_qualified_name(..).is_some(): an import, a module definition or a builtin
            prog.use("names")
            lits.append(f"sym_of({v}, _)")
        elif k == "imported" and isinstance(val, list):
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
        elif k == "unresolved" and val in (True, "star"):
            prog.use("names")
            lits.extend(main_terms.unresolved(prog, val, v, vs))
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
        start = len(lits)
        if not isinstance(getattr(prog, "transform_defs", None), dict):
            prog.transform_defs = {}
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
        prog.transform_defs[t] = (src, lits[start:])
    return lits


def transform_defs_for(prog, lits):
    """The transform literals a helper needs for the transformed metavariables it reads but does not define
    (ast-grep computes a transform where it is read): each from its source, which the helper binds."""
    defs = getattr(prog, "transform_defs", None) or {}
    text = ", ".join(lits)
    have = set(re.findall(r"(?<![\w$])([A-Z][A-Z0-9_]*)\s*=\s*[@a-z0-9\"]", _mask_ticks(_strip_strings(text))))
    out = []

    def need(t, depth=0):
        if t in have or t not in defs or depth > 20:
            return
        src, body = defs[t]
        if src in defs:
            need(src, depth + 1)
        have.add(t)
        # its own variables (`text(X, t2)`) renamed apart from the helper's
        out.extend("".join(q if q.startswith('"') else re.sub(r"(?<![\w$])([td]\d+)\b", r"\1_td", q)
                           for q in re.split(r'("(?:[^"\\]|\\.)*")', l)) for l in body)
    for t in sorted(set(defs) & _upper_vars(text)):
        need(t)
    return out


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


def _binders(s, inside=False):
    """Metavariables a rule body binds itself: in a pattern, by a capture `X = …`, or by a named regex group.
    A capture inside an aggregate (a JSON `not` or `count`) does not escape it, as in ast-grep. With `inside`:
    the captures the aggregates bind."""
    out = set()
    whole = s
    if inside:
        return set().union(set(), *(_binders(s[a:b].replace("{", " ").replace("}", " "))
                                    for a, b in _aggregate_spans(s)))
    s = _drop_aggregates(s)
    for b in re.findall(r'"bind": \{([^}]*)\}', whole):    # an ast(…) atom's argument binding
        out |= set(re.findall(r'"([A-Z_][A-Z0-9_]*)":', b))
    for t in re.findall(r"~\s*(`+)(.*?)\1", s):
        out |= set(re.findall(r"\$\$?\$?([A-Z_][A-Z0-9_]*)", t[1]))
    out |= set(re.findall(r"(?<![\w$])([A-Z][A-Z0-9_]*)\s*=\s*[a-z0-9]", _mask_ticks(_strip_strings(s))))
    out |= set(re.findall(r"\(\?P?<([A-Za-z_]\w*)>", s))
    return out


def _drop_aggregates(s):
    """s without the bodies of its aggregates (`count : { … }`), braces in strings and ticks aside."""
    for a, b in reversed(_aggregate_spans(s)):
        s = s[:a] + " " * (b - a) + s[b:]
    return s


def _aggregate_spans(s):
    """The outermost aggregate bodies of s, as (start, end) offsets."""
    masked = _mask_ticks(_strip_strings_keep(s))
    spans = []
    for m in re.finditer(r"\b(?:count|sum|min|max|mean)\b[^:{}()]*:\s*\{", masked):
        if spans and m.start() < spans[-1][1]:
            continue
        d = 0
        for j in range(m.end() - 1, len(masked)):
            d += masked[j] == "{"
            d -= masked[j] == "}"
            if d == 0:
                spans.append((m.end(), j))
                break
    return spans


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


def _top_binders(lits):
    """Captures the body's own literals bind (not in a disjunction, under `!` or in an aggregate); a metavariable
    only read there (`value(B, t)`) is not one."""
    top = ", ".join(l for l in lits if not l.startswith(("(", "!")) and " : {" not in l)
    out = set()
    for x in _binders(top):
        uses = re.findall(r"(\w*\()?(?<![\w$])\$?" + re.escape(x) + r"\b", top)
        if not uses or not all(u in ("text(", "value(", "str(", "line(") for u in uses):
            out.add(x)
    return out


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
        # a capture bound inside an aggregate and read nowhere outside it is the aggregate's own variable
        head_vars = _upper_vars(e["head"].split("(", 1)[1])
        local = _binders(text, inside=True) - _upper_vars(_drop_aggregates(text)) - head_vars
        rules.append({"e": e, "owner": e["head"].split("(")[0], "head": head_vars,
                      "up": (_upper_vars(text) - local) | set(re.findall(r"\$\$?\$?([A-Z_][A-Z0-9_]*)", " ".join(e.get("edits") or []))),
                      "binds": _binders(text), "calls": calls, "local": local,
                      # captures and calls of the body's own literals: not in a disjunction, under `!` or in an aggregate
                      "tbinds": _top_binders(e["lits"]),
                      "tcalls": sorted(set(re.findall(r"(?<![\w.!])([a-z_]\w*)\(", _mask_ticks(_strip_strings_keep(
                          ", ".join(l for l in e["lits"] if not l.startswith(("(", "!")) and " : {" not in l))))) & set(helpers)),
                      # calls that can bind a capture for the rule: not under `!` or in an aggregate
                      "top_calls": sorted(set(re.findall(r"(?<![\w.!])([a-z_]\w*)\(",
                                                         _mask_ticks(_strip_strings_keep(_drop_aggregates(text))))) & set(helpers)),
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
    # forced[h]: metavariables h captures that every caller has bound before the call. ast-grep binds a
    # metavariable where the match first meets it, so in h the capture is a check against the caller's value
    # (passed in, compared by text), not a new binding.
    forced = {h: set() for h in helpers}
    for _ in range(50):
        old = {h: (set(provides[h]), set(free_in[h]), set(needed[h]), set(forced[h])) for h in helpers}
        # free_in is recomputed each round, not accumulated: a metavariable that looked free before a callee was
        # found to pass it out is bound after all (accumulating left it an input the caller fills with `_`).
        fresh = {h: set() for h in helpers}
        for h in helpers:
            for r in by_owner.get(h, ()):
                own_binds = r["binds"] - forced[h]
                bound = own_binds | r["params"] | r["local"] | set().union(*[provides[g] & iface[g] for g in r["calls"]] or [set()])
                provides[h] |= (own_binds | set().union(*[provides[g] for g in r["calls"]] or [set()])) - r["params"]
                use = r["up"] | set().union(*[free_in[g] for g in r["calls"]] or [set()])
                fresh[h] |= use - bound
            provides[h] -= forced[h]
            fresh[h] |= forced[h]           # the caller's value comes in
        free_in = fresh
        # always[g]: captures every rule of g binds outside disjunctions, `!` and aggregates
        # (a greatest fixpoint: a recursive rule binds what its other rules bind)
        every = set().union(set(), *[r["binds"] for r in rules])
        always = {g: set(every) for g in helpers}
        for _ in range(50):
            prev = {g: set(v) for g, v in always.items()}
            for g in helpers:
                acc = None
                for r in by_owner.get(g, ()):
                    b = (r["tbinds"] - r["local"]).union(*[always[c] for c in r["tcalls"]])
                    acc = b if acc is None else acc & b
                always[g] = acc if acc is not None else set()
            if always == prev:
                break
        for h in helpers:
            if not callers[h]:
                continue
            tprov = set().union(set(), *[(r["binds"] - r["local"]) |
                                         set().union(set(), *[provides[g] for g in r["top_calls"]])
                                         for r in by_owner.get(h, ())])
            for x in sorted(provides[h] & tprov):
                if x.startswith("_"):     # `$_F`: matches anything, never compared
                    continue
                if all(x in (c["params"] | free_in.get(c["owner"], set())
                             | (c["tbinds"] - c["local"] - forced.get(c["owner"], set()))
                             | set().union(set(), *[always[g] for g in c["tcalls"] if g != h]))
                       for c in callers[h]):
                    forced[h].add(x)
            provides[h] -= forced[h]
        for h in helpers:
            for r in callers[h]:
                own = iface.get(r["owner"], set()) | needed.get(r["owner"], set())
                others = set().union(*[iface[g] for g in r["calls"] if g != h] or [set()])
                needed[h] |= r["head"] | r["up"] | others | own
        for h in helpers:
            iface[h] = free_in[h] | (needed[h] & provides[h])
        if all(old[h] == (provides[h], free_in[h], needed[h], forced[h]) for h in helpers):
            break
    order = {h: sorted(iface[h]) for h in helpers}
    share_metavariables.inputs = {h: set(free_in[h]) for h in helpers}
    share_metavariables.params = {h: set().union(set(), *(r["params"] for r in by_owner.get(h, ()))) for h in helpers}
    for r in rules:
        e = r["e"]
        if order.get(r["owner"]):
            e["head"] = e["head"][:-1] + ", " + ", ".join(order[r["owner"]]) + ")"
        hit = [h for h in r["calls"] if order[h]]
        if not hit:
            continue
        relevant = r["head"] | r["up"] | r["binds"] | r["local"] | iface.get(r["owner"], set())
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

    def expansion(h, args, known=frozenset()):
        """The alternatives of h's rules at a call with args; `known`: the caller's variables bound elsewhere,
        which an alternative that does not bind them leaves as they are (no `unset`)."""
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
                if v not in mapping and v not in _NOT_VARS and v not in helpers:   # a kind set is a relation
                    mapping[v] = f"{v}_{k}"
            # a column this rule leaves empty that another rule binds is an output: empty here, as in ast-grep
            missing = [p for p in params if re.fullmatch(r"[A-Z][A-Z0-9_]*", p)
                       and p not in bound_on_every_path(e["lits"]) and p in outputs
                       and mapping.get(p) not in known]
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
            for i, l in enumerate(e["lits"]):
                for s, end in reversed(_calls(l, h)):
                    args = _split_args(l[s + len(h) + 1:end])
                    others = ", ".join(e["lits"][:i] + [l[:s] + l[end + 1:]] + e["lits"][i + 1:])
                    known = _binders(others)
                    alts = expansion(h, args, known)
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

def head_span_ok(t):
    """Whether t's first `(` closes at its last character (t is one parenthesised group)."""
    masked = _mask_ticks(_strip_strings_keep(t))
    d = 0
    for i, ch in enumerate(masked):
        d += ch == "("
        d -= ch == ")"
        if d == 0:
            return i == len(masked) - 1
    return False


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
                # (any relation a literal names, under `!` and in aggregates too: fewer literals ask for more)
                def reaches(l):
                    names = re.findall(r"(?<![\w.])([a-z_]\w*)\(", _mask_ticks(_strip_strings(l)))
                    # a helper that takes demand itself (or calls one) would make the demand relations
                    # depend on each other through its rules, which may negate: kept out
                    return any(g in by and (g == h or h in callees(g) or
                                            any(inputs.get(x) for x in callees(g) | {g})) for g in names)
                lits = [l for l in scope if not reaches(l)]
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


def node_demand(entries, helpers, decls, lib):
    """Magic sets on the node column, for a recursive helper that looks at every descendant or later sibling
    (`has` / `precedes` with `stopBy: end` inside a recursion): bottom-up it holds at every node of the tree, each
    with all it reaches. Its callers only ask at a few nodes, so each rule of the recursion starts with
    demand_h(n): demand_h(x) :- <the positive literals in scope at a call h(x, ...)>, and inside the recursion
    the caller's own demand. A recursion whose call sites cannot all say where they ask is left as it is."""
    by = {}
    for e in entries:
        if not e.get("drop"):
            by.setdefault(e["head"].split("(")[0], []).append(e)

    def names(t):
        return set(re.findall(r"(?<![\w.])([a-z_]\w*)\(", _mask_ticks(_strip_strings_keep(t))))
    body_of = {h: ", ".join(l for e in by[h] for l in e["lits"]) for h in by}
    edges = {h: names(body_of[h]) & set(by) for h in by}
    reach = {}

    def callees(h):
        if h not in reach:
            reach[h], todo = set(), [h]
            while todo:
                for g in edges.get(todo.pop(), ()):
                    if g not in reach[h]:
                        reach[h].add(g)
                        todo.append(g)
        return reach[h]
    fan = re.compile(r"(?<![\w.])(?:within|within_stop)\(\w+, n\b|(?<![\w.])precedes(?:_stop)?\(n, ")
    done, out, sccs = set(), [], []
    for h in sorted(by):
        if h in done or h not in helpers or h not in callees(h):
            continue
        scc = {g for g in callees(h) if h in callees(g)}
        done |= scc
        sccs.append(scc)
    # callers first: a caller's own demand is what limits where it asks
    for scc in sorted(sccs, key=lambda c: (-len(callees(min(c))), min(c))):
        if any(g not in helpers for g in scc) or not any(fan.search(body_of[g]) for g in scc):
            continue
        if any(e["lits"] and e["lits"][0].startswith("demand_") for g in scc for e in by[g]):
            continue
        heads = {g: [a.strip() for a in by[g][0]["head"].split("(", 1)[1][:-1].split(",")] for g in scc}
        if any(hp[0] != "n" for hp in heads.values()):
            continue

        def reaches(l):
            return any(g in by and (g in scc or scc & callees(g)) for g in names(l))
        rules, ok = {g: [] for g in scc}, True
        for e in entries:
            if e.get("drop") or not ok:
                continue
            owner = e["head"].split("(")[0]
            for g in scc:
                for site in _call_sites(e["lits"], g):
                    if site is None:
                        ok = False
                        break
                    scope, call = site
                    x = _split_args(call[len(g) + 1:-1])[0]
                    if x == "_":
                        ok = False
                        break
                    lits = [l for l in scope if not reaches(l)]
                    if owner in scc:
                        hp = [a.strip() for a in e["head"].split("(", 1)[1][:-1].split(",")]
                        lits = [f"demand_{owner}({hp[0]})"] + lits
                    elif owner in callees(g) or not lits:
                        ok = False
                        break
                    if check_rule(wrap(f"demand_{g}({x})", lits)):
                        ok = False
                        break
                    rules[g].append((f"demand_{g}({x})", lits))
                if not ok:
                    break
        if not ok or not all(rules[g] for g in scc):
            continue
        for g in sorted(scc):
            col = decls[g][decls[g].index("(") + 1:-1].split(",")[0].strip()
            lib[f"demand_{g}"] = (f".decl demand_{g}({col})",
                                  [f"// demand_{g}: the nodes {g} is asked at (its recursion looks at every descendant)"],
                                  [r for i, r in enumerate(rules[g]) if r not in rules[g][:i]])
            for e in by[g]:
                hp = [a.strip() for a in e["head"].split("(", 1)[1][:-1].split(",")]
                e["lits"] = [f"demand_{g}({hp[0]})"] + e["lits"]
        out += sorted(scc)
    return out


# ---- recursion through `!` or an aggregate --------------------------------------------------------------------

UNROLL = 8   # levels of nesting a recursion through negation is exact for (tuples of tuples, nested parentheses)


def unroll_negative(entries, helpers, decls, inputs):
    """ast-grep matches a util recursively whether or not the recursion goes through `not` (a tuple is constant
    when no element is not constant): well founded, since each step goes to a child. Datalog needs strata. Such
    a recursive group of helpers becomes UNROLL copies, level k reading level k-1 (level -1 is empty), and the
    helper is its last level: exact wherever the recursion nests at most UNROLL deep. Returns the new entries."""
    by = {}
    for e in entries:
        if not e.get("drop"):
            by.setdefault(e["head"].split("(")[0], []).append(e)
    names = [h for h in sorted(helpers) if h in by]

    def edges(h):
        out = []
        for e in by[h]:
            text = _mask_ticks(_strip_strings_keep(", ".join(e["lits"])))
            for m in re.finditer(r"(?<![\w.])([a-z_]\w*)\(", text):
                g = m.group(1)
                if g in by and g in helpers:
                    depth = text[:m.start()].count("{") - text[:m.start()].count("}")
                    out.append((g, depth > 0 or text[:m.start()].rstrip().endswith("!")))
        return out
    graph = {h: edges(h) for h in names}
    index, low, stack, on, sccs, counter = {}, {}, [], set(), [], [0]

    def strong(v):
        index[v] = low[v] = counter[0]
        counter[0] += 1
        stack.append(v)
        on.add(v)
        for w, _ in graph[v]:
            if w not in index:
                strong(w)
                low[v] = min(low[v], low[w])
            elif w in on:
                low[v] = min(low[v], index[w])
        if low[v] == index[v]:
            comp = []
            while True:
                w = stack.pop()
                on.discard(w)
                comp.append(w)
                if w == v:
                    break
            sccs.append(comp)
    for h in names:
        if h not in index:
            strong(h)
    new, empty = [], []
    for comp in sccs:
        members = set(comp)
        if not any(neg and w in members for v in comp for w, neg in graph[v]):
            continue
        pat = re.compile(r"(?<![\w.])(" + "|".join(sorted(map(re.escape, members), key=len, reverse=True)) + r")\(")

        def at(level):
            return lambda m: f"{m.group(1)}_u{level}(" if level >= 0 else f"{m.group(1)}_u0_empty("
        for h in sorted(members):
            sig = decls[h]
            for k in range(UNROLL):
                decls[f"{h}_u{k}"] = sig.replace(f".decl {h}(", f".decl {h}_u{k}(", 1)
                inputs[f"{h}_u{k}"] = inputs.get(h, set())
                before = len(new)
                for e in by[h]:
                    lits = ["".join(p if p.startswith(("\"", "`")) else pat.sub(at(k - 1), p)
                                    for p in re.split(r'("(?:[^"\\]|\\.)*"|`+[^`]*`+)', l)) for l in e["lits"]]
                    if k == 0 and any(f"_u0_empty(" in l for l in lits if not l.startswith("!") and "{" not in l):
                        continue     # a positive use of level -1, which is empty: the rule cannot fire
                    # `!empty(…)` holds: dropped, so no unbound column is left to place
                    lits = [l for l in (re.sub(r"(,\s*)?!\w+_u0_empty\([^()]*\)(\s*,)?",
                                               lambda m: ", " if m.group(1) and m.group(2) else "", l)
                                        for l in lits) if l.strip()]
                    new.append({"head": e["head"].replace(f"{h}(", f"{h}_u{k}(", 1), "lits": lits})
                if len(new) == before:      # no rule at this level: declared, empty
                    empty.append((f"{h}_u{k}", decls[f"{h}_u{k}"]))
            empty.append((f"{h}_u0_empty", sig.replace(f".decl {h}(", f".decl {h}_u0_empty(", 1)))
            args = by[h][0]["head"].split("(", 1)[1]
            first = by[h][0]
            first["head_note"] = f"// {h}: level {UNROLL - 1} of its recursion through `!` (unroll_negative)"
            first["lits"] = [f"{h}_u{UNROLL - 1}({args}"]
            for e in by[h][1:]:
                e["drop"] = True
    return new, empty


def translate(doc):
    clauses = doc.get("clauses", [])
    langs = [c["forbid"]["ast"].get("language") for c in clauses if "ast" in c.get("forbid", {})]
    langs += [c["define"].get("language") for c in clauses if "define" in c]
    lang = next((l for l in langs if l), "")
    prog = Program(lang)
    head = [f".policy {q(doc.get('id', ''))} revision {doc.get('revision', 1)}",
            # main matches files only, never the guest trees inside string annotations
            ".guests off"]
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
    passed_in_patterns(entries + helper_entries, helpers)
    drop_dead_columns(entries + helper_entries, helpers, prog.decls, prog.notes)
    stop_with_columns(entries + helper_entries, prog.decls)
    inline_unsafe(entries + helper_entries, helpers)
    demand(entries + helper_entries, helpers, getattr(share_metavariables, "inputs", {}), prog.decls, prog.lib)
    node_demand(entries + helper_entries, helpers, prog.decls, prog.lib)
    unrolled, empty = unroll_negative(entries + helper_entries, helpers, prog.decls,
                                      getattr(share_metavariables, "inputs", {}))
    body += [("rule", e) for e in unrolled]
    for name, decl in empty:
        prog.lib[name] = (decl, [f"// {name}: level -1 of an unrolled recursion, empty"], [])
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
        if h.split("(")[0][:1].islower() and getattr(prog, "transform_defs", None):
            lits = list(lits) + transform_defs_for(prog, lits)
        if prog.symbolic:
            # a transform's result is a string, not a node: its text is itself
            pat = re.compile(r"(?<![\w$@])(?:text|value)\((" + "|".join(sorted(prog.symbolic)) + r"), (\w+)\)")
            lits = [pat.sub(r"\2 = \1", l) for l in lits]
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
            # A disjunction some of whose branches bind it: the others leave it empty.
            lits, rest = list(lits), []
            for x in empty:
                done = False
                for i, l in enumerate(lits):
                    t = l.strip()
                    if not (t.startswith("(") and t.endswith(")") and head_span_ok(t)):
                        continue
                    branches = _split_top_text(t[1:-1], ";")
                    # (a named group under `!match` binds nothing)
                    # (a helper call passing x out binds it too)
                    binding = [x in _binders(re.sub(r'!\s*match\("(?:[^"\\]|\\.)*"', "", b)) or f"unset({x})" in b
                               or re.search(r"(?<![!\w.@])[a-z_]\w*\([^()]*(?<![\w$])" + x + r"\b[^()]*\)", b)
                               for b in branches]
                    if len(branches) > 1 and any(binding):
                        lits[i] = "(" + " ; ".join(b.strip() if ok else f"{b.strip()}, unset({x})"
                                                  for b, ok in zip(branches, binding)) + ")"
                        done = True
                        break
                if not done:
                    rest.append(x)
            lits = lits + [f"unset({x})" for x in rest]
            text = wrap(h, lits, edits=edits)
            bad = check_rule(text, fix_rels, inputs)
            out.append(f"// {', '.join(empty)}: not bound by {'this rule' if rest == empty else 'every branch'}, "
                       f"so empty (unset) {'here' if rest == empty else 'where not bound'}, as in ast-grep.")
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
