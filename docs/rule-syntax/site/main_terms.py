"""Lowerings of main's Rerail-extension matcher terms (native/src/code_rule_dsl/matcher/) into plain Datalog.

Each function returns body literals for one JSON term at node v and adds the relations it needs to the program:
per-term relations as `prog.lib` entries named after the term, and the relations every use of a term shares
(`args_pos`, …) once per program. They read only the tape (kind, field, child, text, str): no library.
"""

import json
import re


def lib(prog, name, decl, note, rules):
    if name not in prog.lib:
        prog.lib[name] = (f".decl {name}{decl}", [f"// {name}: {l}" if i == 0 else f"//   {l}" for i, l in enumerate(note)],
                          rules)
    return name


def _q(s):
    return json.dumps(s, ensure_ascii=False)


# ---- arguments ----------------------------------------------------------------------------------------------------
# matcher/arguments.rs, Python: a call's arguments bound to a signature's slots. Positional arguments fill the
# positional slots in order (`f(x for x in y)` passes its generator as the first); `k=v` its slot by name; `*xs`
# makes every positional slot from its position on unknown; `**{"k": v}` (a dict display with string keys) passes
# keywords, any other `**kw` makes every keyword-passable slot not passed otherwise unknown; a slot passed twice is
# unknown. `bind` needs the slot passed, `absent` needs it not passed; an unknown slot matches neither.
# `mode: "syntactic"` is Ruff's find_argument: the first keyword of the slot's name, else the positional argument at
# the slot's position before the first starred one; `**` passes nothing; nothing is unknown.

SPLATS = ("list_splat", "parenthesized_list_splat")
NOT_PLAIN = ("keyword_argument", "dictionary_splat") + SPLATS


def _args_shared(prog):
    lib(prog, "args_plain", "(l: node, a: node, i: number)",
        ["a is child i of argument list l and a positional argument (not `k=v`, `*`, `**`, a comment)"],
        [("args_plain(l, a, i)", ['kind(l, "argument_list")', "child(l, a, i)"]
          + [f"!kind(a, {_q(k)})" for k in NOT_PLAIN + ("comment",)])])
    lib(prog, "args_splat", "(l: node, i: number)", ["child i of argument list l is `*xs`"],
        [("args_splat(l, i)", ['kind(l, "argument_list")', "child(l, s, i)", f"(kind(s, {_q(SPLATS[0])}) ; kind(s, {_q(SPLATS[1])}))"])])
    lib(prog, "args_pos", "(l: node, a: node, j: number)",
        ["a is the positional argument at position j of argument list l, before any `*xs`"],
        [("args_pos(l, a, j)", ["args_plain(l, a, i)", "count : { args_splat(l, k), k < i } = 0",
                                "j = count : { args_plain(l, _, m), m < i }"])])
    lib(prog, "args_star", "(l: node, j: number)",
        ["argument list l has `*xs`, after j positional arguments: positional slots from j on are unknown"],
        [("args_star(l, j)", ["args_splat(l, i)", "count : { args_splat(l, k), k < i } = 0",
                              "j = count : { args_plain(l, _, m), m < i }"])])
    lib(prog, "args_kwp", "(l: node, k: symbol, v: node)", ["`k=v` in argument list l"],
        [("args_kwp(l, k, v)", ['kind(l, "argument_list")', "child(l, a, _)", 'kind(a, "keyword_argument")',
                                'field(a, "name", x)', "text(x, k)", 'field(a, "value", v)'])])
    lib(prog, "args_lit_dict", "(s: node, d: node)",
        ["`**{…}` s spreads the dict display d, whose every entry is a pair with a string key: its pairs are keywords"],
        [("args_lit_dict(s, d)", ['kind(s, "dictionary_splat")', "child(s, d, _)", 'kind(d, "dictionary")',
                                  'count : { child(s, o, _), !kind(o, "comment") } = 1',
                                  "count : { child(d, p, _), !kind(p, \"comment\"), !args_str_pair(p) } = 0"])])
    lib(prog, "args_str_pair", "(p: node)", ["p is a `\"k\": v` pair whose key is a string with a constant value"],
        [("args_str_pair(p)", ['kind(p, "pair")', 'field(p, "key", k)', '(kind(k, "string") ; kind(k, "concatenated_string"))',
                               "str(k, _)", 'field(p, "value", _)'])])
    lib(prog, "args_kw", "(l: node, k: symbol, v: node)", ["keyword k passed with value v in argument list l, `**{…}` included"],
        [("args_kw(l, k, v)", ["args_kwp(l, k, v)"]),
         ("args_kw(l, k, v)", ['kind(l, "argument_list")', "child(l, s, _)", "args_lit_dict(s, d)", "child(d, p, _)",
                               'kind(p, "pair")', 'field(p, "key", x)', "str(x, k)", 'field(p, "value", v)'])])
    lib(prog, "args_kw_spread", "(l: node)", ["argument list l has a `**kw` whose keywords are not known"],
        [("args_kw_spread(l)", ['kind(l, "argument_list")', "child(l, s, _)", 'kind(s, "dictionary_splat")',
                                "!args_lit_dict(s, _)"])])


def _signature(sig):
    """[(name, position or None, keyword-passable)], as compile/arguments.rs reads `/` and `*`."""
    slots, kind = [], "either"
    for s in sig:
        if s == "/":
            slots = [(n, "positional") for n, _ in slots]
        elif s == "*":
            kind = "keyword"
        else:
            slots.append((s, kind))
    out, p = [], 0
    for n, k in slots:
        out.append((n, None if k == "keyword" else p, k != "positional"))
        if k != "keyword":
            p += 1
    return out


def _slot_rels(prog, sig, slot, syntactic):
    """(value relation, absent relation) for one slot of one signature."""
    slots = _signature(sig)
    name, pos, kwok = next(s for s in slots if s[0] == slot)
    key = (tuple(sig), slot, syntactic)
    prog.arg_rels = getattr(prog, "arg_rels", {})
    if key in prog.arg_rels:
        return prog.arg_rels[key]
    _args_shared(prog)
    k = len(prog.arg_rels) + 1
    base = re.sub(r"[^0-9a-z]+", "_", slot.lower())
    val, absent = f"arg_{base}_{k}", f"arg_no_{base}_{k}"
    shown = ", ".join(sig)
    rules_v, rules_a = [], []
    pre = ['kind(n, "call")', 'field(n, "arguments", l)']
    if syntactic:
        if kwok:
            rules_v.append((f"{val}(n, v)", pre + [f"args_kwp(l, {_q(name)}, v)"]))
        if pos is not None:
            rules_v.append((f"{val}(n, v)", pre + [f"args_pos(l, v, {pos})"] + ([f"!args_kwp(l, {_q(name)}, _)"] if kwok else [])))
            if pos == 0:
                rules_v.append((f"{val}(n, v)", ['kind(n, "call")', 'field(n, "arguments", v)', 'kind(v, "generator_expression")']))
        neg = ([f"!args_kwp(l, {_q(name)}, _)"] if kwok else []) + ([f"!args_pos(l, _, {pos})"] if pos is not None else [])
        if pos == 0:
            neg.append('!kind(l, "generator_expression")')
        rules_a.append((f"{absent}(n)", pre + neg))
        mode = "syntactic: Ruff's find_argument"
    else:
        if pos is not None:
            rules_v.append((f"{val}(n, v)", pre + [f"args_pos(l, v, {pos})"] + ([f"!args_kw(l, {_q(name)}, _)"] if kwok else [])))
            if pos == 0:
                rules_v.append((f"{val}(n, v)", ['kind(n, "call")', 'field(n, "arguments", v)', 'kind(v, "generator_expression")']))
        if kwok:
            rules_v.append((f"{val}(n, v)", pre + [f"args_kw(l, {_q(name)}, v)", f"count : {{ args_kw(l, {_q(name)}, _) }} = 1"]
                            + ([f"!args_pos(l, _, {pos})"] if pos is not None else [])))
        neg = []
        if pos is not None:
            neg += [f"!args_pos(l, _, {pos})", f"count : {{ args_star(l, j), j <= {pos} }} = 0"]
            if pos == 0:
                neg.append('!kind(l, "generator_expression")')
        if kwok:
            neg += [f"!args_kw(l, {_q(name)}, _)", "!args_kw_spread(l)"]
        rules_a.append((f"{absent}(n)", pre + neg))
        mode = "as Python binds the call"
    lib(prog, val, "(n: node, v: node)", [f"call n passes v for slot `{slot}` of ({shown}), {mode}"], rules_v)
    lib(prog, absent, "(n: node)", [f"call n does not pass slot `{slot}` of ({shown}), {mode}"], rules_a)
    prog.arg_rels[key] = (val, absent)
    return val, absent


def arguments(prog, spec, v, vs):
    sig, syntactic = spec.get("signature") or [], spec.get("mode") == "syntactic"
    lits = [f'kind({v}, "call")']
    for cap, slot in (spec.get("bind") or {}).items():
        val, _ = _slot_rels(prog, sig, slot, syntactic)
        x = vs.fresh("a")
        lits.append(f"{val}({v}, {x}), {cap} = {x}")
    for slot in spec.get("absent") or []:
        _, absent = _slot_rels(prog, sig, slot, syntactic)
        lits.append(f"{absent}({v})")
    return lits


# ---- value ----------------------------------------------------------------------------------------------------------
# matcher/value.rs: predicates over a node's own value (core's `value`, `value_type`). A node whose value is not
# known fails every predicate but `constant: false`. Python's type names are `type(v).__name__`; core's value_type
# says "boolean" for a bool. Truthiness is Python's bool(v); ordering bounds hold for numbers only.

VALUE_OPS = {"truthy", "bitsAny", "constant", "type", "startsWith", "gt", "ge", "lt", "le"}
PY_TYPE = {"bool": "boolean"}
NUMERIC = '(value_type({x}, "int") ; value_type({x}, "float"))'


def _truthy(prog):
    zero = '(x = "0" ; x = "0.0" ; x = "-0.0")'
    nonzero = 'x != "0", x != "0.0", x != "-0.0"'
    num = NUMERIC.format(x="x")
    text = '(value_type(x, "str") ; value_type(x, "bytes"))'
    return lib(prog, "truthy_value", "(n: node, b: symbol)",
               ["b is \"true\" when n's value is truthy (Python's bool(v)), \"false\" when it is falsy"],
               [('truthy_value(n, "false")', ["value(n, x)", 'value_type(x, "NoneType")']),
                ('truthy_value(n, b)', ["value(n, x)", 'value_type(x, "boolean")', "b = x"]),
                ('truthy_value(n, "false")', ["value(n, x)", num, zero]),
                ('truthy_value(n, "true")', ["value(n, x)", num, nonzero]),
                ('truthy_value(n, "false")', ["value(n, x)", text, 'x = ""']),
                ('truthy_value(n, "true")', ["value(n, x)", text, 'x != ""'])])


def _bits(x, mask):
    tests = [f"(to_number({x}) / {1 << i}) % 2 = 1" if i else f"to_number({x}) % 2 = 1"
             for i in range(mask.bit_length()) if mask >> i & 1]
    return tests[0] if len(tests) == 1 else "(" + " ; ".join(tests) + ")"


def value(prog, val, v, vs, opt_alts, match):
    reader = "static_value" if val.get("follow") else "value"
    ops = {k: a for k, a in val.items() if k != "follow"}
    if ops.get("constant") is False and len(ops) == 1:
        return [f"!{reader}({v}, _)"]
    x = vs.fresh("v")
    lits = [f"{reader}({v}, {x})"]
    for op, arg in ops.items():
        if op == "constant":
            if arg is False:        # known and not constant cannot both hold
                lits.append("false")
        elif op == "regex":
            lits.append(match(arg, x))
        elif op == "eq":
            lits.append(opt_alts(arg, lambda c: f"{x} = {_q(c)}"))
        elif op == "ne":
            lits.append(opt_alts(arg, lambda c: f"{x} != {_q(c)}"))
        elif op == "type":
            types = arg if isinstance(arg, list) else [arg]
            alts = [f"value_type({x}, {_q(PY_TYPE.get(t, t) if prog.lang == 'python' else t)})" for t in types]
            lits.append(alts[0] if len(alts) == 1 else "(" + " ; ".join(alts) + ")")
        elif op in ("gt", "ge", "lt", "le") and isinstance(arg, (int, float)) and not isinstance(arg, bool):
            sym = {"lt": "<", "le": "<=", "gt": ">", "ge": ">="}[op]
            lits += [NUMERIC.format(x=x), f"to_number({x}) {sym} {arg}"]     # value is a symbol: "255" for 0xff
        elif op in ("gt", "ge", "lt", "le"):
            sym = {"lt": "<", "le": "<=", "gt": ">", "ge": ">="}[op]
            lits.append(opt_alts(arg, lambda c: f"{x} {sym} {_q(c)}"))
        elif op == "startsWith":
            pre = arg if isinstance(arg, list) else [arg]
            alts = [f"prefix({_q(p)}, {x})" for p in pre]
            lits.append(alts[0] if len(alts) == 1 else "(" + " ; ".join(alts) + ")")
        elif op == "bitsAny" and isinstance(arg, int):
            lits += [f'value_type({x}, "int")', _bits(x, arg)]
        elif op == "truthy" and isinstance(arg, bool):
            lits.append(f"{_truthy(prog)}({v}, {_q('true' if arg else 'false')})")
        else:
            prog.untranslated += 1
            lits.append(f"ast({v}, {json.dumps({'value': {op: arg}}, ensure_ascii=False)})")
    return lits


# ---- resolves: names in import statements, and modules seen before ----------------------------------------------
# matcher/resolve.rs `imported` with `relative: "text"` (statement_name): a dotted name in an import statement and
# the qualified name it imports. `import a.b` imports "a.b"; `from m import x` imports "m.x" and its module name is
# "m"; a relative module is its text without the dots (`from .a import b` imports "a.b", `from . import b` "b");
# `from __future__ import x` imports "__future__.x".

FROM = '(kind(s, "import_from_statement") ; kind(s, "future_import_statement"))'


def _statement_names(prog):
    lib(prog, "from_module", "(s: node, m: symbol)",
        ["the module text of from-import s, read as `relative: \"text\"` does (dots dropped)"],
        [("from_module(s, m)", ['kind(s, "future_import_statement")', 'm = "__future__"']),
         ("from_module(s, m)", ['kind(s, "import_from_statement")', 'field(s, "module_name", x)', 'kind(x, "dotted_name")',
                                "name(x, m)"]),
         ("from_module(s, m)", ['kind(s, "import_from_statement")', 'field(s, "module_name", x)', 'kind(x, "relative_import")',
                                'child(x, y, _)', 'kind(y, "dotted_name")', "name(y, m)"]),
         ('from_module(s, "")', ['kind(s, "import_from_statement")', 'field(s, "module_name", x)', 'kind(x, "relative_import")',
                                 'count : { child(x, y, _), kind(y, "dotted_name") } = 0'])])
    return lib(prog, "statement_name", "(n: node, q: symbol)",
               ["dotted name n in an import statement imports q (relative modules by their text)"],
               [("statement_name(n, q)", ['kind(n, "dotted_name")', "child(s, n, _)", 'kind(s, "import_statement")', "name(n, q)"]),
                ("statement_name(n, q)", ['kind(n, "dotted_name")', "child(s, n, _)", 'kind(s, "relative_import")', "name(n, q)"]),
                ("statement_name(n, q)", ['kind(n, "dotted_name")', 'field(a, "name", n)', 'kind(a, "aliased_import")',
                                          "child(s, a, _)", 'kind(s, "import_statement")', "name(n, q)"]),
                ("statement_name(n, q)", ['kind(n, "dotted_name")', 'field(a, "name", n)', 'kind(a, "aliased_import")',
                                          "child(s, a, _)", FROM, "from_module(s, m)", "name(n, x)", "q = @dot(m, x)"]),
                ("statement_name(n, m)", ['kind(n, "dotted_name")', 'field(s, "module_name", n)', FROM, "from_module(s, m)"]),
                ("statement_name(n, q)", ['kind(n, "dotted_name")', "child(s, n, _)", FROM, '!field(s, "module_name", n)',
                                          "from_module(s, m)", "name(n, x)", "q = @dot(m, x)"])])


def statement_imported(prog, names, v, vs, prefix):
    rel = _statement_names(prog)
    q = vs.fresh("q")
    if prefix:
        alts = [f"({q} = {_q(n)} ; prefix({_q(n + '.')}, {q}))" for n in names]
    else:
        alts = [f"{q} = {_q(n)}" for n in names]
    return [f"{rel}({v}, {q})", alts[0] if len(alts) == 1 else "(" + " ; ".join(alts) + ")"]


# `seenBefore` (Ruff's seen_module): Ruff has visited an absolute import of one of the top-level modules before
# node n: an import at fewer enclosing function bodies (Ruff visits function bodies after their scope), or as many
# and earlier in the file; anything in a lambda body is visited after every import.

FN_DEPTH = 'count : {{ within({n}, b{k}), field(f{k}, "body", b{k}), kind(f{k}, "function_definition") }}'


def seen_before(prog, modules, v, vs):
    rel = lib(prog, "seen_import", "(s: node, h: symbol, k: number)",
              ["import statement s imports absolute module h (its first segment), inside k function bodies"],
              [("seen_import(s, h, k)", ['kind(s, "import_statement")', "child(s, c, _)",
                                         '(kind(c, "dotted_name"), x = c ; kind(c, "aliased_import"), field(c, "name", x))',
                                         "child(x, i, 0)", 'kind(i, "identifier")', "text(i, h)",
                                         "k = " + FN_DEPTH.format(n="s", k="")]),
               ("seen_import(s, h, k)", ['kind(s, "import_from_statement")', 'field(s, "module_name", x)',
                                         'kind(x, "dotted_name")', "child(x, i, 0)", 'kind(i, "identifier")', "text(i, h)",
                                         "k = " + FN_DEPTH.format(n="s", k="")])])
    d, st, s, so, h, ks, kn = (vs.fresh(p) for p in ("d", "st", "s", "so", "h", "ks", "kn"))
    mods = " ; ".join(f"{h} = {_q(m)}" for m in modules)
    k = vs.fresh("")
    return [f"node({d}, {v}, _, {st}, _)", f"{rel}({s}, {h}, {ks})", f"({mods})", f"node({d}, {s}, _, {so}, _)",
            f"{kn} = " + FN_DEPTH.format(n=v, k=k),
            f'({ks} < {kn} ; {ks} = {kn}, {so} < {st} ; count : {{ within({v}, l{k}), field(m{k}, "body", l{k}), kind(m{k}, "lambda") }} > 0)']


# ---- resolves.unresolved (matcher/resolve.rs is_unresolved, is_unresolved_export) ----------------------------------
# A reference (an identifier that is looked up, not only bound) whose lookup reaches the module without a binding,
# that is no builtin; or a string entry of a module-level `__all__` definition naming no module binding or builtin.
# `true` asks for no star import in the module (the name is unbound); "star" for one (it may come from it).

def _dunder_all(prog):
    lib(prog, "all_expr", "(x: node)",
        ["x is a module-level `__all__ =`, `+=` or `: T =` value, or a part of one that lists names",
         "(Ruff's extract_dunder_all_names: `+` chains, parentheses, walrus values, list(...) and tuple(...))"],
        [("all_expr(x)", ['(kind(st, "assignment") ; kind(st, "augmented_assignment"))', 'field(st, "left", l)',
                          'kind(l, "identifier")', 'text(l, "__all__")', "child(es, st, _)",
                          'kind(es, "expression_statement")', "child(m, es, _)", 'kind(m, "module")',
                          'field(st, "right", x)']),
         ("all_expr(y)", ["all_expr(x)", 'kind(x, "assignment")', 'field(x, "right", y)']),
         ("all_expr(y)", ["all_expr(x)", 'kind(x, "binary_operator")', '(field(x, "left", y) ; field(x, "right", y))']),
         ("all_expr(y)", ["all_expr(x)", 'kind(x, "parenthesized_expression")', "child(x, y, _)",
                          '!kind(y, "comment")']),
         ("all_expr(y)", ["all_expr(x)", 'kind(x, "named_expression")', 'field(x, "value", y)']),
         ("all_expr(y)", ["all_expr(x)", 'kind(x, "call")', 'field(x, "function", f)', 'kind(f, "identifier")',
                          '(text(f, "list") ; text(f, "tuple"))', 'field(x, "arguments", a)', "child(a, y, _)"])])
    return lib(prog, "all_entry", "(e: node, x: symbol)",
               ["string e is an entry of a module `__all__` definition, exporting the name x"],
               [("all_entry(e, x)", ["all_expr(l)", '(kind(l, "list") ; kind(l, "tuple"))', "child(l, e, _)",
                                     '(kind(e, "string") ; kind(e, "concatenated_string"))',
                                     'count : { within(i, e), kind(i, "interpolation") } = 0', "str(e, x)"])])


def unresolved(prog, mode, v, vs):
    rel = _dunder_all(prog)
    x, d, m = vs.fresh("x"), vs.fresh("d"), vs.fresh("m")
    star = f"star({m})" if mode == "star" else f"!star({m})"
    return [f"docof({v}, {d})", f"modnode({d}, {m})", f'( kind({v}, "identifier"), !bind_only({v}), miss({v}, {x})'
            f' ; (kind({v}, "string") ; kind({v}, "concatenated_string")), {rel}({v}, {x}), !bound_in({m}, {x}) )',
            f"!builtin({x})", star]


# ---- layout (matcher/layout.rs) -----------------------------------------------------------------------------------
# span: line breaks inside the node; start: its first byte's offset; before/after: the line breaks between the node
# and its neighbour on that side: the nearest token, the nearest named sibling, or a capture bound before (which
# must lie wholly on that side). `edge` lets the file's start or end stand in for a missing neighbour.

def _bounds(c, x):
    ops = {"min": ">=", "max": "<=", "eq": "="}
    return [f"{x} {ops[k]} {int(n)}" for k, n in c.items() if k in ops]


def layout(prog, spec, v, vs):
    """Literals for a `layout` term, or None when it asks for something not lowered (blank lines, comments)."""
    if set(spec) - {"span", "start", "before", "after"}:
        return None
    lits = []
    if "span" in spec:
        a, b, x = vs.fresh("l"), vs.fresh("l"), vs.fresh("k")
        lits += [f"line({v}, {a})", f"end_line({v}, {b})", f"{x} = {b} - {a}"] + _bounds(spec["span"], x)
    if "start" in spec:
        s = vs.fresh("st")
        lits += [f"node(_, {v}, _, {s}, _)"] + _bounds(spec["start"], s)
    for side in ("before", "after"):
        gap = spec.get(side)
        if gap is None:
            continue
        if set(gap) - {"to", "breaks", "edge"}:
            return None
        to, x, nb = gap.get("to", "token"), vs.fresh("k"), vs.fresh("t")
        a, b = vs.fresh("l"), vs.fresh("l")
        # the gap runs from line a (the earlier end) to line b (the later start)
        if side == "before":
            here, there = f"line({v}, {b})", f"end_line({nb}, {a})"
        else:
            here, there = f"end_line({v}, {a})", f"line({nb}, {b})"
        if to == "token":
            find = [f"token_{side}({v}, {nb})"]
        elif to == "sibling":
            find = [f"next_sibling({nb}, {v})" if side == "before" else f"next_sibling({v}, {nb})"]
        elif isinstance(to, str) and to.startswith("capture."):
            c = to.split(".", 1)[1]
            s1, e1, s2, e2 = vs.fresh("s"), vs.fresh("e"), vs.fresh("s"), vs.fresh("e")
            find = [f"{nb} = {c}", f"node(_, {v}, _, {s1}, {e1})", f"node(_, {c}, _, {s2}, {e2})",
                    f"{e2} <= {s1}" if side == "before" else f"{s2} >= {e1}"]
        else:
            return None
        bounds = _bounds(gap.get("breaks") or {}, x)
        found = find + [here, there, f"{x} = {b} - {a}"] + bounds
        if gap.get("edge"):
            if side == "before":
                edge = [f"count : {{ {', '.join(find)} }} = 0", f"line({v}, {b})", f"{x} = {b} - 1"] + bounds
            else:
                d, k = vs.fresh("d"), vs.fresh("k")
                edge = [f"count : {{ {', '.join(find)} }} = 0", f"end_line({v}, {a})", f"docof({v}, {d})",
                        f"line_count({d}, {k})", f"{x} = {k} - {a}"] + bounds
            lits.append("( " + ", ".join(found) + " ; " + ", ".join(edge) + " )")
        else:
            lits += found
    return lits
