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
