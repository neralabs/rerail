"""rdl: the spike's compiler for Rerail's Datalog dialect.

One parser and AST, a stratifier, a planner that orders each rule body by cost from statistics kept in SQL, and two
backends generated from the same plan:

    python3 -I rdl.py sql    <program.rdl> <facts.sqlite> <out dir>       run semi-naive SQL in SQLite
    python3 -I rdl.py plan   <program.rdl> <facts.sqlite> <plan.json>     write the plan (for the Ascent build)
    python3 -I rdl.py ascent <program.rdl> <plan.json>    <program.rs>    generate an Ascent program

Tree structure is never a relation. `nearest($n, $s, set)` reads the tape (preorder id, par, size): the planner either
climbs from a bound $n through `par`, or descends from a bound $s over [s+1, s+size] skipping the subtrees of nested
set members. It picks between them, and picks each rule's anchor, from the `kfreq`/`efreq` statistics: a climb costs
one parent read per step (expected steps from the efreq parent-kind chain), a descent one row per node walked (mean
subtree size of the anchor kind). No constant is fitted to a corpus.

Environment: RDL_NODEMAND=1 ignores `.demand`; RDL_MAGIC=all (or `.demand *`) applies Soufflé's magic-set transformation
to every derived relation (`magic_adorn`); RDL_COARSE=1 passes no recursive or negated literals sideways into
magic rules; RDL_NATIVE=nearest,fixpoint,kseed,direct,vtab,setfix,upsert (or all) uses the native pieces in sqlite-ext/,
loaded from RDL_EXT; RDL_PROFILE=1 prints the lattice merge phases (2: also each recursive rule body, timed alone); RDL_TRACE=1 prints per-stratum
times and plans (2: also each native stratum's seed and probe plans); RDL_PLAN0=1 turns off skew-weighted fanout, document ranges and view materialization.
"""
import json, os, re, sqlite3, sys, time
from collections import defaultdict

TOP = 4611686018427387904
# RDL_NATIVE=nearest,fixpoint,kseed,direct,vtab,setfix,upsert (all = every one but direct and upsert, which measured slower): native pieces from sqlite-ext (RDL_EXT is the library path).
NATIVE = set(os.environ.get("RDL_NATIVE", "").split(",")) - {""}
# RDL_PLAN0=1: the planner before skew-weighted fanout, document ranges and view materialization (for measuring them).
PLAN0 = bool(os.environ.get("RDL_PLAN0"))
if "all" in NATIVE: NATIVE = {"nearest", "fixpoint", "kseed", "vtab", "setfix", "join"} | (NATIVE - {"all"})
# RDL_NATIVE=...,program: every set-valued stratum whose rules have native steps runs on the native set engine, its
# relations kept native for later strata (only what SQL reads, and the outputs, are written to tables).
if "program" in NATIVE: NATIVE |= {"nearest", "setfix", "join"}

# ---- parser ------------------------------------------------------------------------------------------------------

TOKEN = re.compile(r'\s*(?://[^\n]*|(\$[A-Za-z_]\w*)|("[^"]*")|(-?\d+)|(:-|!=|<=|>=|[(),.;:=<>!{}|@])|([A-Za-z_]\w*))')

def tokens(src):
    pos, out = 0, []
    while pos < len(src):
        m = TOKEN.match(src, pos)
        if not m or m.end() == pos:
            if src[pos:].strip() == "":
                break
            raise SyntaxError(f"bad input at {src[pos:pos + 40]!r}")
        pos = m.end()
        var, s, num, punct, ident = m.groups()
        if var: out.append(("var", var[1:]))
        elif s: out.append(("str", s[1:-1]))
        elif num: out.append(("int", int(num)))
        elif punct: out.append(("p", punct))
        elif ident: out.append(("id", ident))
    return out

class Var:
    def __init__(s, n): s.n = n
    def __repr__(s): return "$" + s.n
class Wild:
    def __repr__(s): return "_"
class Const:
    def __init__(s, v): s.v = v
    def __repr__(s): return json.dumps(s.v) if isinstance(s.v, str) else str(s.v)
class TopT:
    def __repr__(s): return "top"
class Call:
    def __init__(s, f, args): s.f, s.args = f, args
    def __repr__(s): return f"@{s.f}({', '.join(map(repr, s.args))})"

class Atom:
    def __init__(s, rel, args, neg=False): s.rel, s.args, s.neg = rel, args, neg
    def vars(s): return {a.n for a in s.args if isinstance(a, Var)}
    def __repr__(s): return ("!" if s.neg else "") + f"{s.rel}({', '.join(map(repr, s.args))})"
class Cmp:
    def __init__(s, l, op, r): s.l, s.op, s.r = l, op, r
    def vars(s): return term_vars(s.l) | term_vars(s.r)
    def __repr__(s): return f"{s.l!r} {s.op} {s.r!r}"
class Agg:
    def __init__(s, var, atom): s.var, s.atom = var, atom
    def vars(s): return {s.var} | s.atom.vars()
    def __repr__(s): return f"${s.var} = count {{ {s.atom!r} }}"
class Nearest:
    def __init__(s, n, a, kset): s.n, s.a, s.kset = n, a, kset
    def vars(s): return {s.n, s.a}
    def __repr__(s): return f"nearest(${s.n}, ${s.a}, {s.kset})"

def term_vars(t):
    if isinstance(t, Var): return {t.n}
    if isinstance(t, Call): return set().union(*map(term_vars, t.args))
    return set()

class UnboundDemand(ValueError):
    def __init__(s, msg, rel): super().__init__(msg); s.rel = rel

class NotStratifiable(ValueError):
    def __init__(s, msg, scc, neg): super().__init__(msg); s.scc, s.neg = scc, neg

class Program:
    def __init__(s):
        s.decls, s.lattice, s.facts, s.rules, s.inputs, s.outputs, s.kindsets = {}, {}, defaultdict(list), [], [], [], {}
        s.demand = []
        s.view_src = {}   # view over one input atom -> (input, the input column behind each view column, or None)
        s.docof = {}   # rel -> (node column, document column) for projections of node(d, n): documents are id intervals

def parse(src):
    toks, i = tokens(src), [0]
    prog = Program()
    peek = lambda k=0: toks[i[0] + k] if i[0] + k < len(toks) else ("eof", None)
    def take(kind=None, val=None):
        t = peek()
        if (kind and t[0] != kind) or (val is not None and t[1] != val):
            raise SyntaxError(f"expected {kind} {val}, got {t} near {toks[max(0, i[0] - 5):i[0] + 3]}")
        i[0] += 1
        return t
    def term():
        t = peek()
        if t[0] == "var": take(); return Var(t[1])
        if t[0] == "str": take(); return Const(t[1])
        if t[0] == "int": take(); return Const(t[1])
        if t == ("id", "_"): take(); return Wild()
        if t == ("id", "top"): take(); return TopT()
        if t == ("p", "@"):
            take(); f = take("id")[1]; take("p", "(")
            args = [term()]
            while peek() == ("p", ","): take(); args.append(term())
            take("p", ")")
            return Call(f, args)
        raise SyntaxError(f"bad term {t}")
    def atom(neg=False):
        rel = take("id")[1]; take("p", "(")
        args = [term()]
        while peek() == ("p", ","): take(); args.append(term())
        take("p", ")")
        return Atom(rel, args, neg)
    def literal():
        if peek() == ("p", "!"): take(); return atom(True)
        if peek() == ("id", "nearest"):
            take(); take("p", "("); n = take("var")[1]; take("p", ","); a = take("var")[1]; take("p", ",")
            ks = take("id")[1]; take("p", ")")
            return Nearest(n, a, ks)
        if peek()[0] == "id": return atom()
        l = term(); op = take("p")[1]
        if op == "=" and peek() == ("id", "count"):
            take(); take("p", "{"); a = atom(); take("p", "}")
            return Agg(l.n, a)
        return Cmp(l, op, term())
    while peek()[0] != "eof":
        if peek() == ("p", "."):
            take(); d = take("id")[1]
            if d in ("decl", "lattice"):
                name = take("id")[1]; take("p", "(")
                cols, lat = [], None
                while True:
                    c = take("id")[1]; take("p", ":"); ty = take("id")[1]; cols.append((c, ty))
                    if peek() == ("p", ";"): take(); lat = len(cols)
                    elif peek() == ("p", ","): take()
                    else: break
                take("p", ")")
                prog.decls[name] = cols
                if d == "lattice": prog.lattice[name] = len(cols) - 1
            elif d in ("input", "output"):
                names = [take("id")[1]]
                while peek() == ("p", ","): take(); names.append(take("id")[1])
                (prog.inputs if d == "input" else prog.outputs).extend(names)
            elif d == "demand":
                if peek() == ("p", "*"): take(); prog.demand.append("*"); continue
                prog.demand.append(take("id")[1])
                while peek() == ("p", ","): take(); prog.demand.append(take("id")[1])
            elif d == "kindset":
                name = take("id")[1]; take("p", "="); ks = [take("str")[1]]
                while peek() == ("p", "|"): take(); ks.append(take("str")[1])
                prog.kindsets[name] = ks
            continue
        head = atom()
        if peek() == ("p", "."):
            take(); prog.facts[head.rel].append([a.v for a in head.args]); continue
        take("p", ":-")
        body = [literal()]
        while peek() == ("p", ","): take(); body.append(literal())
        take("p", ".")
        # A computed term is a value, not a pattern: an atom never matches on one (the backends would drop it).
        for l in body:
            if isinstance(l, Atom) and any(isinstance(t, Call) for t in l.args):
                raise SyntaxError(f"computed term in body atom {l!r} of the rule for {head.rel}: bind it with `$v = @f(...)`")
        prog.rules.append((head, body))
    return prog

# ---- demand (magic sets) -----------------------------------------------------------------------------------------

def magic_sets(prog):
    """Applies demand, dropping a demanded relation whose negated use would close a negative cycle through its own
    demand (for example `!bound_in($s, $x)` in `lookup`, when `bound_in`'s demand comes from `lookup`'s): that
    relation is computed in full instead. The relations dropped are in `prog.demand_dropped`."""
    import copy
    dropped = []
    while True:
        p = copy.deepcopy(prog)
        p.demand = [r for r in prog.demand if r not in {d for d, _ in dropped}]
        try:
            p = magic_sets_once(p)
            p.demand_dropped = dropped
            return p
        except NotStratifiable as e:
            if e.neg not in p.demand: raise
            dropped.append((e.neg, "negative cycle through its demand"))
        except UnboundDemand as e:
            # Only a consequence of an earlier drop; otherwise the program asks for unbounded demand.
            if not dropped: raise
            dropped.append((e.rel, "a use with an unbound key once " + dropped[-1][0] + " is computed in full"))

def magic_sets_once(prog):
    """Restricts each `.demand` relation to the keys its consumers ask for (first column bound).

    Every rule for a demanded relation R gets `magic_R(key)` as its first literal. Every use of a demanded S with its
    key bound produces `magic_S(key) :- <the literals before it>`; a negated use too, because `!S(k)` needs S complete
    for k. The literals passed sideways are as many as stay stratifiable: every positive atom, bound comparison and
    walk, plus negations and atoms of the head's own recursive component ("extras"). Extras can close a cycle through
    negation (magic_S -> !Q -> magic_Q -> magic_S), so the transformed program is stratified and the extras of each
    extra is added only if the program stays stratifiable (greedily, starting from none); leaving a literal out only
    widens demand, which is always safe. `RDL_COARSE=1` drops every extra up front (the first version). A demanded atom whose key is not bound
    by its prefix is a compile error: the demand would be everything."""
    if not prog.demand:
        return prog
    sccs, _ = strata(prog)
    comp = {r: i for i, scc in enumerate(sccs) for r in scc}
    coarse = bool(os.environ.get("RDL_COARSE"))
    rules, magic = [], []   # magic: (head, [(literal, extra)])
    for h, body in prog.rules:
        demanded_head = h.rel in prog.demand
        seen, bound = [], set()
        if demanded_head:
            key = h.args[0]
            seen.append((Atom(f"magic_{h.rel}", [key]), False))
            bound |= term_vars(key)
        for l in body:
            if isinstance(l, Atom) and l.rel in prog.demand:
                key = l.args[0]
                if not (term_vars(key) <= bound or isinstance(key, Const)):
                    raise UnboundDemand(f"demand for {l.rel} is unbound in the rule for {h.rel}: {body}", l.rel)
                magic.append((Atom(f"magic_{l.rel}", [key]), list(seen)))
            if isinstance(l, Atom) and not l.neg:
                same = l.rel in prog.demand and comp.get(l.rel) == comp.get(h.rel)
                seen.append((l, same)); bound |= l.vars()
            elif isinstance(l, Atom) and l.neg and l.vars() <= bound:
                seen.append((l, True))
            elif isinstance(l, Nearest) and (l.n in bound or l.a in bound):
                seen.append((l, False)); bound |= l.vars()
            elif isinstance(l, Cmp) and l.vars() <= bound:
                seen.append((l, False))
        rules.append((h, ([Atom(f"magic_{h.rel}", [h.args[0]])] if demanded_head else []) + body))
    for r in prog.demand:
        prog.decls[f"magic_{r}"] = [("k", prog.decls[r][0][1])]
    prog.decls["magic_true"] = [("k", "int")]
    prog.facts["magic_true"].append([0])
    keep = [[not extra for _, extra in pre] for _, pre in magic]
    def build():
        out = []
        for (mh, pre), ks in zip(magic, keep):
            # A kept literal still needs its variables bound by the kept literals before it.
            bound, body = set(), []
            for (l, _), k in zip(pre, ks):
                if not k: continue
                if isinstance(l, Atom) and not l.neg: body.append(l); bound |= l.vars()
                elif isinstance(l, Nearest) and (l.n in bound or l.a in bound): body.append(l); bound |= l.vars()
                elif l.vars() <= bound: body.append(l)
            if not term_vars(mh.args[0]) <= bound and not isinstance(mh.args[0], Const):
                raise ValueError(f"demand for {mh.rel[6:]} is unbound: {[l for l, _ in pre]}")
            out.append((mh, body or [Atom("magic_true", [Const(0)])]))
        return out + rules
    # Greedy: start from no extras (always stratifiable) and keep each extra that leaves the program stratifiable.
    prog.rules = build()
    strata(prog)
    for i, (_, pre) in enumerate(magic):
        for j, (_, extra) in enumerate(pre):
            if not extra or coarse: continue
            keep[i][j] = True
            prog.rules = build()
            try: strata(prog)
            except NotStratifiable: keep[i][j] = False
    prog.rules = build()
    prog.magic_extras = sum(1 for (mh, pre), ks in zip(magic, keep) for (_, extra), k in zip(pre, ks) if extra and k)
    return prog

def magic_adorn(prog):
    """Soufflé's magic-set transformation (souffle-lang.github.io/magicset), applied to every derived relation: `.demand
    *` or `RDL_MAGIC=all`. Each output starts with every argument free. A rule for relation R under adornment `a` (one
    `b` or `f` per argument) has the variables at its `b` positions bound, and its body is visited in sideways order:
    the left-most atom with a bound variable, else the atom lowest in the strata (an input first), each filter as
    soon as it can run. A derived atom met there is adorned by what is bound (variables and constants), and if any
    argument is bound gets `magic_S_a(bound args) :- <guard>, <literals before it>`.

    Differences from Soufflé:
    - **One relation per predicate.** Every adornment of R guards a copy of R's rules into R itself, so its tuples are
      stored once. A relation demanded with every argument free is computed in full and its other adornments vanish.
    - **Sideways order.** A constant-only binding does not pick an atom first: Soufflé's leftmost-bound rule would put
      `pv($e, "fn", $f)` before `callee($c, $e)` and demand every function value instead of every callee's. A bound
      first argument (the key) comes before other bound arguments, an input before a derived atom, and a connected
      atom before an unconnected one.
    - **Key adornments.** An adornment whose first argument is free is all free (see `adorn`), unless
      the rules answer it (`answers`; `RDL_MAGIC_NONKEY=0`: never). One binding a superset of another's columns is
      answered through the other (`pv_bbb` through `pv_bbf`).
    - **Computed keys.** A head's computed term at a bound position (`qv(@cat($q, $x), ..)`) is guarded through a
      fresh variable and `$m = @cat($q, $x)`, which splits a bound `$m` at its dots (`cmp_binds`), so the rule
      reads `modname` by `$q` and `bv` by its key.
    - **Negation** is kept (Soufflé skips such relations); a relation that aggregates or is aggregated is computed in
      full. A derived atom or a negation joins a magic rule only if the program stays stratifiable, added greedily
      (`magic_sets_once`'s extras), or if it binds a key; the relations of a component in which demand still closes
      a negative cycle are computed in full, or with `RDL_GATE=1` (native program mode) the negation waits for its
      demand (`gate_negations`).
    - Views (one input atom) and lattice values are never adorned.
    `RDL_MAGIC_EXCLUDE=a,b`: relations computed in full. `RDL_MAGIC_KEY=1`: adorn the key column only.
    `RDL_MAGIC_TRACE=1`: why each relation is computed in full. `prog.magic_report` describes the result."""
    import copy
    derived = {h.rel for h, _ in prog.rules}
    by = defaultdict(list)
    for h, b in prog.rules: by[h.rel].append((h, b))
    view = {r for r, rs in by.items() if all(len(b) == 1 and isinstance(b[0], Atom) and not b[0].neg and b[0].rel not in derived for _, b in rs)}
    idb = derived - view
    full = {r for r in os.environ.get("RDL_MAGIC_EXCLUDE", "").split(",") if r}
    # A relation that aggregates is computed in full: guarded, its count would join its demand's recursive component.
    full |= {h.rel for h, b in prog.rules if any(isinstance(l, Agg) for l in b)}
    sccs, _ = strata(prog)
    level = {r: i for i, scc in enumerate(sccs) for r in scc}
    cyc = []
    while True:
        try:
            p = magic_adorn_once(copy.deepcopy(prog), by, idb, full, level)
            p.magic_report["full for negation"] = cyc
            return p
        except NotStratifiable as e:
            # Demand closed a cycle through negation: each relation negated inside the component is computed in full,
            # with every relation it depends on, so that its demand no longer comes from its consumers.
            new, todo = set(), [r for r in sorted(e.negs) if r in idb and r not in full]
            while todo:
                r = todo.pop()
                if r in new or r in full or r not in idb: continue
                new.add(r)
                todo += [(l.atom if isinstance(l, Agg) else l).rel for _, b in by[r] for l in b if isinstance(l.atom if isinstance(l, Agg) else l, Atom)]
            if not new: raise
            if os.environ.get("RDL_MAGIC_TRACE"): print(f"  negative cycle: negs {sorted(e.negs)}, full {sorted(new)}", flush=True)
            full |= new
            cyc.extend(sorted(set(e.negs) & new))

MAGIC_KEY = os.environ.get("RDL_MAGIC_KEY", "0") != "0"   # adorn the key column only
NONKEY = os.environ.get("RDL_MAGIC_NONKEY", "1") != "0"   # adorn non-key columns bound by a variable (`answers`)
INPUT_FIRST = os.environ.get("RDL_MAGIC_INPUT_FIRST", "1") != "0"   # sideways: a bound-key input before a derived atom

def magic_adorn_once(prog, by, idb, full, level):
    arity = {r: len(c) for r, c in prog.decls.items()}
    def adorn(a, bound):
        """`b` for each bound argument; all free unless the first (the key) is bound: demand on later columns alone
        (`res_fbf("fn")`: every node that names a function) restricts little and reverses the rules' direction, which
        is how it spreads (`lwalk_fbb`, under a `nearest` descent: every node of a scope paired with every name)."""
        keyed = prog.lattice.get(a.rel, arity[a.rel])   # a lattice's value is never a key
        ad = "".join("b" if i < keyed and (isinstance(t, Const) or (isinstance(t, Var) and t.n in bound)) else "f" for i, t in enumerate(a.args))
        if not ad.startswith("b"):
            # A non-key column bound by a variable (`sbinds` asked by statement), if the rules can answer it.
            var = any(x == "b" and isinstance(t, Var) for t, x in zip(a.args, ad))
            return ad if NONKEY and var and answers(a.rel, ad, ()) else "f" * len(ad)
        return "b" + "f" * (len(ad) - 1) if MAGIC_KEY else ad
    answered = {}
    def answers(rel, ad, stack):
        """Whether R's rules answer the non-key adornment `ad` without asking any derived relation in full, or
        through a cycle of non-key adornments: `sbinds` by statement reads `stmt` by its key, then `defn` by its
        key, while `base` by base class reads `pv` by value, which spreads (`pv_fbb` asks `qn` by name, `look` and
        `walk` by scope, every scope's). One that doesn't leaves its consumer reading R in full."""
        if (rel, ad) in answered: return answered[(rel, ad)]
        if rel in full or (rel, ad) in stack: return False
        ok = True
        for h, body in by[rel]:
            bound, extra = set(), []
            for t, x in zip(h.args, ad):
                if x != "b": continue
                if isinstance(t, Call): v = Var(f"_m{len(extra)}"); extra.append(Cmp(v, "=", t)); t = v
                if isinstance(t, Var): bound.add(t.n)
            try: order = sips(body + extra, bound)
            except ValueError: ok = False; break
            for l in order:
                a = l.atom if isinstance(l, Agg) else l
                if isinstance(l, Atom) and l.rel in idb and l.rel not in full:
                    ad2 = "".join("b" if i < prog.lattice.get(l.rel, arity[l.rel]) and (isinstance(t, Const) or (isinstance(t, Var) and t.n in bound))
                                  else "f" for i, t in enumerate(l.args))
                    if not ad2.startswith("b") and not (any(x == "b" and isinstance(t, Var) for t, x in zip(l.args, ad2))
                                                        and answers(l.rel, ad2, stack + ((rel, ad),))):
                        ok = False; break
                bound |= binds(l, bound) or set()
            if not ok: break
        if not stack: answered[(rel, ad)] = ok
        return ok
    def binds(l, bound):
        """The variables literal `l` binds once it runs, or None if it can't run yet (the planner's `ready`)."""
        if isinstance(l, Atom): return l.vars() if not l.neg else (set() if l.vars() <= bound else None)
        if isinstance(l, Cmp): return cmp_binds(l, bound)
        if isinstance(l, Agg): return {l.var} if (l.atom.vars() - {l.var}) <= bound else None
        if isinstance(l, Nearest): return l.vars() if (l.n in bound or l.a in bound) else None
    def sips(body, bound):
        """The body in sideways order: filters as soon as they can run; then the left-most positive atom whose first
        argument (a relation's key, by this dialect's convention) is bound, an input before a derived one; else one sharing a bound variable (an
        input or view first); else the lowest one in the strata (inputs first). Constants alone never pick an atom,
        and an atom sharing no bound variable comes last: `kind($f, "function_item")` taken while `$y` is bound
        demands every pair of `$y` and function."""
        rest, order, bound = list(body), [], set(bound)
        while rest:
            for l in rest:
                if not (isinstance(l, Atom) and not l.neg) and binds(l, bound) is not None:
                    order.append(l); bound |= binds(l, bound); rest.remove(l); break
            else:
                pos = [l for l in rest if isinstance(l, Atom) and not l.neg]
                if not pos: raise ValueError(f"no order binds {rest}")
                first = lambda a: isinstance(a.args[0], Const) or (isinstance(a.args[0], Var) and a.args[0].n in bound)
                joined = [a for a in pos if a.vars() & bound]
                # An input lookup on a bound key before a derived atom: `kind($t, $k), patkind($k)` filter `tgt($t)`'s
                # demand to pattern nodes, where demand first would climb every ancestor of every target.
                l = (next((a for a in pos if first(a) and a.rel not in idb and INPUT_FIRST), None) or next((a for a in pos if first(a)), None)
                     or min(joined, key=lambda a: a.rel in idb, default=None)
                     or min(pos, key=lambda a: level.get(a.rel, -1)))
                order.append(l); bound |= l.vars(); rest.remove(l)
        return order
    def closure(full, coarse):
      rules, magic, seen = [], [], set()   # magic: (head, [(literal, extra)])
      why.clear()
      answered.clear()
      todo = [(o, "f" * arity[o]) for o in prog.outputs if o in idb]
      while todo:
        rel, ad = todo.pop()
        if rel in full: ad = "f" * arity[rel]
        if (rel, ad) in seen: continue
        seen.add((rel, ad))
        for h, body in by[rel]:
            bound, pre = set(), []
            guard = None
            if "b" in ad:
                # A computed head term at a bound position is matched through a fresh variable: the guard binds it,
                # and `$m = @cat($q, $x)` tests it once the body binds the call's arguments (or computes it first,
                # making the guard a membership test).
                gargs, calls = [], []
                for t, x in zip(h.args, ad):
                    if x != "b": continue
                    if isinstance(t, Call):
                        calls.append(Cmp(Var(f"_m{len(calls)}"), "=", t)); t = calls[-1].l
                    gargs.append(t)
                guard = Atom(f"magic_{rel}_{ad}", gargs)
                body = body + calls
                pre.append((guard, False))
                bound |= guard.vars()
            for l in sips(body, bound):
                a = l.atom if isinstance(l, Agg) else l
                if isinstance(a, Atom) and a.rel in idb:
                    # An aggregated relation is computed in full: its demand would pull the count into its consumers'
                    # recursive component.
                    ad2 = "f" * arity[a.rel] if a.rel in full or isinstance(l, Agg) else adorn(a, bound)
                    ad2 = coarse.get((a.rel, ad2), ad2)
                    if "b" in ad2:
                        magic.append((Atom(f"magic_{a.rel}_{ad2}", [t for t, x in zip(a.args, ad2) if x == "b"]), list(pre)))
                    todo.append((a.rel, ad2))
                    if "b" not in ad2: why.setdefault(a.rel, (h, ad, l))
                # Sideways: input atoms and filters always; derived atoms and negations as extras, kept if the
                # program stays stratifiable (a derived atom can lead back to the demand: magic_S -> Q -> !S).
                # Leaving one out only widens the demand.
                if isinstance(l, Atom) and not l.neg:
                    pre.append((l, a.rel in idb))
                elif not isinstance(l, Agg):
                    pre.append((l, isinstance(l, Atom)))
                b2 = binds(l, bound)
                bound |= b2 or set()
            rules.append((h, ([guard] if guard else []) + body))
      return rules, magic, seen
    # A relation demanded with every argument free anywhere is computed in full: its other adornments would only add
    # guarded copies of its rules (and dependencies through their magic relations), deriving nothing new.
    # Likewise an adornment that binds a superset of another's columns is answered through the other: its demand,
    # projected, is the other's (`pv_bbb` through `pv_bbf`), and binding more columns multiplies the demand tuples
    # (every key with every value its consumers pass) where it gains little.
    full, coarse, why = set(full), {}, {}   # why: rel -> (head, its adornment, literal) that first demanded it free
    while True:
        rules, magic, seen = closure(full, coarse)
        free = {r for r, a in seen if "b" not in a} - full
        bset = lambda ad: {i for i, x in enumerate(ad) if x == "b"}
        more = {}
        for rel, ad in seen:
            under = [a2 for r2, a2 in seen if r2 == rel and a2 != ad and "b" in a2 and bset(a2) < bset(ad)]
            if under and "b" in ad: more[(rel, ad)] = min(under, key=lambda a2: (len(bset(a2)), a2))
        if os.environ.get("RDL_MAGIC_TRACE"):
            for r in sorted(free): print(f"  free {r}: {why[r][2]} in {why[r][0]} ({why[r][1]})" if r in why else f"  free {r}: an output", flush=True)
        if not free and not more: break
        full |= free
        coarse.update(more)
    for rel, ad in seen:
        if "b" in ad: prog.decls[f"magic_{rel}_{ad}"] = [(c, t) for (c, t), x in zip(prog.decls[rel], ad) if x == "b"]
    prog.decls["magic_true"] = [("k", "int")]
    prog.facts["magic_true"].append([0])
    keep = [[not extra for _, extra in pre] for _, pre in magic]
    views = [(h, b) for h, b in prog.rules if h.rel not in idb]   # unchanged
    def build():
        out = {}
        for (mh, pre), ks in zip(magic, keep):
            # A left-out literal that binds a key variable, or what such a literal needs, stays: the key must be
            # bound (stratification still decides, as for any extra).
            key = term_vars_all(mh.args)
            ks, need = list(ks), set(key)
            for i in reversed(range(len(pre))):
                l = pre[i][0]
                gives = l.vars() if (isinstance(l, Atom) and not l.neg) or isinstance(l, Nearest) else \
                        l.vars() if isinstance(l, Cmp) and l.op == "=" and isinstance(l.l, Var) and is_cat(l.r) else \
                        {l.l.n} if isinstance(l, Cmp) and l.op == "=" and isinstance(l.l, Var) else set()
                if gives & need:
                    ks[i] = True
                    # A split gives its parts from the whole, and the whole from its parts.
                    split = {l.l.n} if isinstance(l, Cmp) and is_cat(l.r) and term_vars(l.r) & need else set()
                    climb = isinstance(l, Nearest) and l.a in need   # else it descends to $n from $a
                    need -= gives
                    if isinstance(l, Nearest): need |= {l.n} if climb else {l.a}
                    elif isinstance(l, Cmp): need |= split or term_vars(l.r)
            bound, body = set(), []
            for (l, _), k in zip(pre, ks):
                if not k: continue
                if isinstance(l, Atom) and not l.neg: body.append(l); bound |= l.vars()
                elif binds(l, bound) is not None: body.append(l); bound |= binds(l, bound)
            if not key <= bound:
                raise ValueError(f"demand for {mh.rel} is unbound: {[l for l, _ in pre]}")
            out[repr((mh, body))] = (mh, body or [Atom("magic_true", [Const(0)])])
        return list(out.values()) + rules + views
    def neg_ad(a):
        """The adornment a negated atom (every variable bound) was demanded with."""
        ad = "f" * arity[a.rel] if a.rel in full else adorn(a, a.vars())
        return coarse.get((a.rel, ad), ad)
    def settle():
        """The rules from `build`, with the negations demand puts in their own component gated where it does."""
        prog.rules, prog.gates = build(), {}
        try: strata(prog)
        except NotStratifiable:
            if not GATE: raise
            gate_negations(prog, neg_ad)
            strata(prog)
    try: settle()
    except NotStratifiable as e:
        e.negs = {(l.atom if isinstance(l, Agg) else l).rel for h, b in prog.rules if h.rel in e.scc for l in b
                  if (isinstance(l, Atom) and l.neg or isinstance(l, Agg)) and (l.atom if isinstance(l, Agg) else l).rel in e.scc}
        raise
    for i, (_, pre) in enumerate(magic):
        for j, (_, extra) in enumerate(pre):
            if not extra: continue
            keep[i][j] = True
            try: settle()
            except NotStratifiable: keep[i][j] = False
    settle()
    prog.demand = []
    prog.magic_report = {"adornments": sorted(f"{r}_{a}" for r, a in seen if "b" in a), "magic rules": len(magic), "full": sorted(full & idb),
                         "extras kept": sum(1 for (_, pre), ks in zip(magic, keep) for (_, extra), k in zip(pre, ks) if extra and k),
                         "gates": len(prog.gates)}
    return prog

# RDL_GATE=1: gated negations, where a negative cycle through demand otherwise makes relations full. Only the native
# program mode orders a gated negation.
GATE = os.environ.get("RDL_GATE", "0") != "0" and "program" in NATIVE
SQL_REPLAN = os.environ.get("RDL_SQL_REPLAN", "0") != "0"

def gate_negations(prog, neg_ad):
    """Demand can put a negated relation N in the component of the rule that negates it: `!bound_in($s, $x)` in
    `walk`, whose demand comes from `pv`, which reads `walk`. N's own rules read only below the negation, except for
    their guard `magic_N` (its demand). So N(t) is final once the relations N reads are quiet with `magic_N(t)` among
    their demand, and the negation (or a count of N) can wait for that: `!N(t)` becomes `done_N(t), !N(t)`, and the native engine
    copies `magic_N` into `done_N` (a gate) only in a wave after which nothing N reads changed. The negating rule then
    reruns from `done_N`'s delta.

    Within each such component the edges into guards (R -> magic_R) are dropped: what is left must be stratified, and
    its strata are the gates' levels (lower gates first). A gate's `quiet` set is what N reads there, with their
    magic relations; `deps` are the gates those read. A negated relation computed in full in the component (no
    `magic_N`) has a unit gate, `done_N(0)`, as has an aggregated one."""
    derived = {h.rel for h, _ in prog.rules}
    edges = defaultdict(set)
    for h, body in prog.rules:
        for l in body:
            a = l.atom if isinstance(l, Agg) else l
            if isinstance(a, Atom) and a.rel in derived: edges[h.rel].add((a.rel, a.neg or isinstance(l, Agg)))
    sccs = tarjan(derived, edges)
    comp = {r: i for i, scc in enumerate(sccs) for r in scc}
    bad = {comp[v] for v in derived for w, neg in edges[v] if neg and comp[w] == comp[v]}
    guard = lambda v, w: w.startswith("magic_") and not v.startswith("magic_")
    red, level = {}, {}
    for i in bad:
        scc = sccs[i]
        e = {v: {(w, n) for w, n in edges[v] if w in scc and not guard(v, w)} for v in scc}
        red.update(e)
        for k, s in enumerate(tarjan(scc, e)):
            for v in s: level[v] = k
        for v in scc:
            for w, n in e[v]:
                if n and level[w] >= level[v]:
                    raise NotStratifiable(f"not stratifiable: {v} depends negatively on {w} without its demand", scc, w)
    rules, gates = [], {}
    for h, body in prog.rules:
        nb = []
        for l in body:
            a = l.atom if isinstance(l, Agg) else l
            inside = isinstance(a, Atom) and a.rel in comp and comp[a.rel] == comp.get(h.rel) and comp[a.rel] in bad
            if inside and (isinstance(l, Agg) or l.neg):
                ad = "f" * len(a.args) if isinstance(l, Agg) else neg_ad(l)   # an aggregated relation is full
                src = f"magic_{a.rel}_{ad}"
                if "b" in ad and comp.get(src) == comp[a.rel]:
                    g = Atom(f"done_{a.rel}_{ad}", [t for t, x in zip(a.args, ad) if x == "b"])
                else:
                    g, src = Atom(f"done_{a.rel}", [Const(0)]), None
                gates.setdefault(g.rel, {"src": src, "rel": a.rel, "level": level[a.rel]})
                n = Agg(l.var, l.atom) if isinstance(l, Agg) else Atom(l.rel, l.args, neg=True)
                n.gated = True
                nb += [g, n]
            else: nb.append(l)
        rules.append((h, nb))
    for done, g in gates.items():
        if g["src"]:
            prog.decls[done] = prog.decls[g["src"]]
            vs = [Var(f"_g{i}") for i in range(len(prog.decls[done]))]
            rules.append((Atom(done, vs), [Atom(g["src"], vs)]))   # only places done_N in N's component
        else:
            prog.decls[done] = [("k", "int")]
            rules.append((Atom(done, [Const(0)]), [Atom(g["rel"], [Wild() for _ in prog.decls[g["rel"]]])]))
        reach, todo = set(), [g["rel"]]
        while todo:
            v = todo.pop()
            if v in reach: continue
            reach.add(v)
            todo += [w for w, _ in red.get(v, ())]
        scc = sccs[comp[g["rel"]]]
        g["quiet"] = sorted(reach | {m for m in scc if m.startswith("magic_") and m[6:].rsplit("_", 1)[0] in reach})
        g["deps"] = sorted({l.rel for h, b in rules if h.rel in reach for l in b if isinstance(l, Atom) and l.rel in gates and l.rel != done})
    prog.rules, prog.gates = rules, gates

def term_vars_all(ts): return set().union(*map(term_vars, ts)) if ts else set()

def cmp_binds(l, bound):
    """The variables comparison `l` binds once it runs, or None if it can't run yet. `$x = e` assigns an unbound $x
    once `e`'s variables are bound; `$m = @cat($q, $x)` with $m bound splits it, binding $q and $x where unbound (at
    every ".", so a bound $q leaves at most one $x); otherwise every variable must be bound."""
    if l.op == "=" and isinstance(l.l, Var) and l.l.n not in bound: return {l.l.n} if term_vars(l.r) <= bound else None
    if l.vars() <= bound: return set()
    if l.op == "=" and isinstance(l.l, Var) and is_cat(l.r): return {t.n for t in l.r.args} - bound
    return None

def is_cat(t): return isinstance(t, Call) and t.f == "cat" and all(isinstance(x, Var) for x in t.args)

# ---- stratification ----------------------------------------------------------------------------------------------

def strata(prog):
    """The program's components, dependencies first, and its dependency edges (head -> {(dep, negative)}). A gated
    negation (`gate_negations`) is not a negative edge: the native engine orders it inside its component."""
    derived = {h.rel for h, _ in prog.rules}
    edges = defaultdict(set)  # head -> (dep, negative)
    for h, body in prog.rules:
        for l in body:
            a = l.atom if isinstance(l, Agg) else l
            if isinstance(a, Atom) and a.rel in derived:
                edges[h.rel].add((a.rel, (a.neg or isinstance(l, Agg)) and not getattr(l, "gated", False)))
    sccs = tarjan(derived, edges)
    for scc in sccs:
        for v in sorted(scc):
            for w, neg in sorted(edges[v]):
                if neg and w in scc:
                    raise NotStratifiable(f"not stratifiable: {v} depends negatively on {w} in its own component", scc, w)
    # Tarjan emits SCCs in reverse topological order of dependencies already (dependencies first).
    return sccs, edges

def tarjan(nodes, edges):
    """Strongly connected components of `nodes` under `edges` (v -> {(w, _)}), dependencies first."""
    index, low, onstack, stack, sccs, counter = {}, {}, set(), [], [], [0]
    def visit(v):
        index[v] = low[v] = counter[0]; counter[0] += 1; stack.append(v); onstack.add(v)
        for w, _ in sorted(edges[v]):   # sorted: the same components in the same order in every process
            if w not in nodes: continue
            if w not in index: visit(w); low[v] = min(low[v], low[w])
            elif w in onstack: low[v] = min(low[v], index[w])
        if low[v] == index[v]:
            scc = set()
            while True:
                w = stack.pop(); onstack.discard(w); scc.add(w)
                if w == v: break
            sccs.append(scc)
    sys.setrecursionlimit(10000)
    for v in sorted(nodes):
        if v not in index: visit(v)
    return sccs

# ---- statistics (read from SQL) ----------------------------------------------------------------------------------

class Stats:
    """The corpus statistics the planner prices plans with, read from the tables stats.py builds at ingestion
    (`stat_*` in the facts database) in one statement. Nothing is counted at plan time: a GROUP BY over every input
    column on every run was 0.7 s of planning here, and ast-index-spike found that statistics held only by the
    process plan differently in each process (S174, S178). Statistics whose fingerprint (row counts) doesn't match
    the facts are refused, so a plan is never priced on statistics of other data."""
    def __init__(self, db, prog):
        consts = sorted({t.v for _, b in prog.rules for l in b if isinstance(l, Atom) for t in l.args if isinstance(t, Const)}, key=repr)
        holes = ", ".join("?" * len(consts)) or "NULL"
        try:
            rows = db.execute(
                "SELECT 'meta', k, NULL, v, NULL, NULL, NULL FROM f.stat_meta"
                " UNION ALL SELECT 'col', tbl, col, rows, ndv, sq, sqd FROM f.stat_col"
                " UNION ALL SELECT 'kind', kind, NULL, n, size, NULL, NULL FROM f.stat_kind"
                " UNION ALL SELECT 'edge', pkind, ckind, n, field, NULL, NULL FROM f.stat_edge"
                f" UNION ALL SELECT 'val', tbl, col, n, val, docs, sqd FROM f.stat_val WHERE val IN ({holes})"
                + "".join(f" UNION ALL SELECT 'rows', '{t}', NULL, count(*), NULL, NULL, NULL FROM f.{t}" for t in prog.inputs),
                consts).fetchall()
        except sqlite3.OperationalError as e:
            raise SystemExit(f"the facts have no planner statistics ({e}); build them at ingestion: python3 -I stats.py build <facts.sqlite>")
        meta = {r[1]: r[3] for r in rows if r[0] == "meta"}
        actual = {r[1]: r[3] for r in rows if r[0] == "rows"}
        stale = [t for t, n in actual.items() if meta.get(f"rows.{t}") != n]
        if stale:
            raise SystemExit(f"planner statistics do not match the facts ({stale}); rebuild them: python3 -I stats.py build <facts.sqlite>")
        self.col = {(r[1], r[2]): r[3:] for r in rows if r[0] == "col"}   # (table, column) -> (rows, ndv, sq, sqd)
        self.kfreq = {r[1]: r[3] for r in rows if r[0] == "kind"}
        self.size = {r[1]: r[4] / r[3] for r in rows if r[0] == "kind" and r[3]}
        self.nodes = sum(self.kfreq.values())
        parents = defaultdict(lambda: defaultdict(int))
        for r in rows:
            if r[0] == "edge": parents[r[2]][r[1]] += r[3]
        self.parents = {c: {p: n / sum(ps.values()) for p, n in ps.items()} for c, ps in parents.items()}
        self.card = {}
        self.fan = {}        # (rel, col) -> posting-weighted fanout of an equality probe: sq / rows
        self.dfreq = {}      # (derived rel, col) -> {value: postings}, for a column of few values (native stats)
        self.obs = {}        # atom_sig -> [bindings in, bindings out]: fanouts native joins observed (feedback)
        self.solving = set() # the recursive component a native stratum is planning (`atom_rows`)
        self.prog = prog
        self.indexes = {}    # input rel -> index column lists
        self.docs = meta.get("docs")   # documents are node-id intervals (f.stat_docrange), checked at ingestion
        self.db = db
        # Exact postings of the program's constants, ("const", table, column, value) -> n. A constant with no row
        # does not occur in that column.
        self.memo = {("const", r[1], r[2], r[4]): r[3] for r in rows if r[0] == "val"}
        self.steps_cache = {}
        self.kindsets = prog.kindsets
    def const_freq(self, rel, cname, v):
        """Exact postings for a constant probe on an input column, from stat_val; None if there are no statistics
        for the column (replanning from a plan file that lacks them)."""
        key = ("const", rel, cname, v)
        if key in self.memo: return self.memo[key]
        return 0 if self.db is not None and (rel, cname) in self.col else None
    def fan_doc(self, rel, cname, rname):
        """Posting-weighted fanout of an equality probe on `cname` confined to one document (any range column
        locates the same document): sqd / rows."""
        rows, _, _, sqd = self.col[(rel, cname)]
        return sqd / rows if rows else 1.0
    def steps(self, kind, kset):
        """Expected parent reads to climb from `kind` to the nearest ancestor in `kset` (absorbing Markov chain)."""
        key = (kind, kset)
        if key not in self.steps_cache:
            members = set(self.kindsets[kset])
            est = defaultdict(float)
            for _ in range(40):  # tree depth bound; converges well before
                nxt = {}
                for c, ps in self.parents.items():
                    nxt[c] = 1 + sum(pr * (0 if p in members else est[p]) for p, pr in ps.items())
                est = defaultdict(float, nxt)
            self.steps_cache[key] = est[kind] if kind in est else 1 + sum(est.values()) / max(1, len(est))
            if kind is None:
                total = sum(self.kfreq.values())
                self.steps_cache[key] = sum(self.kfreq[k] / total * est[k] for k in self.kfreq if k in est)
        return self.steps_cache[key]

# ---- planner -----------------------------------------------------------------------------------------------------

OBS_MIN = 32   # bindings a step must have seen before its observed fanout replaces the estimate

def atom_sig(a, bound):
    """An atom's binding pattern: its relation, and which columns are bound, constants by value."""
    return (a.rel, tuple((c, t.v if isinstance(t, Const) else "*") for c, t in enumerate(a.args)
                         if isinstance(t, Const) or (isinstance(t, Var) and t.n in bound)))

def observe(stats, order, line):
    """Adds a native join's step counts ("in:out" per step, `set_fixpoint_steps`) to the observed fanouts of the
    binding patterns its positive atoms were read with. The first step (the delta, or a seed's anchor) is skipped."""
    if not line: return
    counts = [tuple(map(int, x.split(":"))) for x in line.split(",")]
    bound = set()
    for i, ((l, _), (n_in, n_out)) in enumerate(zip(order, counts)):
        if i > 0 and isinstance(l, Atom) and not l.neg and n_in:
            o = stats.obs.setdefault(atom_sig(l, bound), [0, 0])
            o[0] += n_in; o[1] += n_out
        if not (isinstance(l, Atom) and l.neg): bound |= l.vars()

def plan_rule(prog, stats, head, body, ndv, first=None):
    """Orders a rule body. Tries every positive atom as the anchor, extends greedily by estimated work, and keeps the
    cheapest. Work counts rows produced per step plus tape reads for `nearest` (climb: steps per row; descent: mean
    subtree size per row). Negations, comparisons and aggregates run as soon as their variables are bound. With
    `first`, only orders anchored on that atom (a semi-naive delta) are considered."""
    kinds = {}  # var -> constant kind, from kind($v, "k") atoms
    for l in body:
        if isinstance(l, Atom) and not l.neg and l.rel == "kind" and isinstance(l.args[0], Var) and isinstance(l.args[1], Const):
            kinds[l.args[0].n] = l.args[1].v

    def card(rel):
        if rel == "node": return stats.nodes
        return stats.card.get(rel, 1000)

    def atom_rows(a, bound):
        rows = atom_rows_(a, bound)
        # No variable bound on a relation of the component being solved: a scan of it per binding, priced as a pass
        # over the corpus (shared among its constants' values) at least. Its estimate comes from seeds or early waves
        # (tiny while it is still growing, or when demand-guarded), and a probe that scans it per delta row is
        # quadratic in it.
        if a.rel in stats.solving and not any(isinstance(t, Var) and t.n in bound for t in a.args):
            floor = float(stats.nodes)
            for c, t in enumerate(a.args):
                if isinstance(t, Const): floor /= max(1.0, ndv.get((a.rel, c), 1.0))
            rows = max(rows, floor)
        return rows

    def atom_rows_(a, bound):
        seen = stats.obs.get(atom_sig(a, bound))
        if seen and seen[0] >= OBS_MIN: return max(seen[1] / seen[0], 0.01)   # observed, not estimated
        if a.rel == "kind" and isinstance(a.args[1], Const):
            if isinstance(a.args[0], Var) and a.args[0].n in bound: return 1.0
            return float(stats.kfreq.get(a.args[1].v, 0)) or 0.5
        cols = [c for c, t in enumerate(a.args) if isinstance(t, Const) or (isinstance(t, Var) and t.n in bound)]
        if not cols: return float(card(a.rel))
        if PLAN0:
            rows = float(card(a.rel))
            for c in cols: rows /= max(1.0, ndv.get((a.rel, c), 1.0))
            return max(rows, 0.01)
        # The most selective bound column gives its expected postings (exact for a constant on an input); the other
        # bound columns divide by their NDV.
        def postings(c):
            t = a.args[c]
            if isinstance(t, Const) and a.rel in prog.inputs:
                f = stats.const_freq(a.rel, sql_col(a.rel, c, prog), t.v)
                if f is not None: return float(f)
            if isinstance(t, Const) and (a.rel, c) in stats.dfreq:   # a derived relation's tag column (native stats)
                return float(stats.dfreq[(a.rel, c)].get(t.v, 0))
            return stats.fan.get((a.rel, c), card(a.rel) / max(1.0, ndv.get((a.rel, c), 1.0)))
        first = min(cols, key=postings)
        rows = postings(first)
        for c in cols:
            if c != first: rows /= max(1.0, ndv.get((a.rel, c), 1.0))
        # Every column bound: a membership test, at most one row of a set (a magic guard, before its relation has
        # statistics, would otherwise be priced at the relation's whole cardinality).
        if len(cols) == len(a.args) and a.rel not in prog.lattice: rows = min(rows, 1.0)
        return max(rows, 0.01)

    def doc_range(a, bound):
        """(column, rows) if `a` can be read as a range inside one document: a body literal `docof($v, $d)` (any
        relation that is node's (n, d) projection) has $d bound and $v unbound, $v is `a`'s column, and an input index
        on `a` is the bound columns then that column. Rows are the per-document posting-weighted fanout."""
        if PLAN0 or stats.docs is None or a.rel not in prog.inputs: return None
        for d in body:
            if not (isinstance(d, Atom) and not d.neg and d.rel in prog.docof): continue
            nc, dc = prog.docof[d.rel]
            v, dv = d.args[nc], d.args[dc]
            if not (isinstance(v, Var) and v.n not in bound and (isinstance(dv, Const) or (isinstance(dv, Var) and dv.n in bound))): continue
            rcols = [c for c, t in enumerate(a.args) if isinstance(t, Var) and t.n == v.n]
            bcols = [c for c, t in enumerate(a.args) if isinstance(t, Const) or (isinstance(t, Var) and t.n in bound)]
            if len(rcols) != 1 or len(bcols) != 1: continue
            rname, bname = sql_col(a.rel, rcols[0], prog), sql_col(a.rel, bcols[0], prog)
            if not any(ix[:2] == [bname, rname] for ix in stats.indexes.get(a.rel, [])): continue
            t = a.args[bcols[0]]
            rows = stats.fan_doc(a.rel, bname, rname)
            if isinstance(t, Const): rows = min(rows, float(stats.const_freq(a.rel, bname, t.v) or rows))
            return rcols[0], rows
        return None

    # `$x = e` assigns $x if it is unbound when `e`'s variables are (SQL and the native `cat`/`eq` steps bind an
    # unbound left side); otherwise it is a test.
    def ready(l, bound):
        if isinstance(l, Atom) and l.neg: return l.vars() <= bound
        if isinstance(l, Cmp): return cmp_binds(l, bound) is not None
        if isinstance(l, Agg): return (l.atom.vars() - {l.var}) <= bound
        if isinstance(l, Nearest): return l.n in bound or l.a in bound
        return True

    def is_join(l): return (isinstance(l, Atom) and not l.neg) or isinstance(l, Nearest)

    def step(l, rows, bound):
        """(work, rows out, mode) of adding join literal `l`, or None if it can't run yet."""
        if isinstance(l, Atom):
            out = rows * atom_rows(l, bound)
            rng = doc_range(l, bound)
            if rng and rows * rng[1] < out:
                out = rows * rng[1]
                return out + rows, out, "range"
            return out + rows, out, None
        if not ready(l, bound): return None
        if l.n in bound:   # climb from $n: one parent read per expected step; a bound $a just filters
            return rows * stats.steps(kinds.get(l.n), l.kset), rows, "climb"
        size = stats.size.get(kinds.get(l.a), stats.nodes / 1000)   # descend from $a: every node walked
        return rows * size, rows * size, "descend"

    def close(order, rows, bound, rest):
        """Applies every filter (negation, comparison, aggregate) whose variables are bound."""
        progressed = True
        while progressed:
            progressed = False
            for l in list(rest):
                if not is_join(l) and ready(l, bound):
                    order.append((l, None)); rest.remove(l); bound |= l.vars(); progressed = True
                    if not (isinstance(l, Cmp) and l.op == "=") and not isinstance(l, Agg):
                        rows *= 0.5
        return rows

    # Dynamic programming over subsets of the join literals (Selinger): the cheapest order of each subset, with the
    # filters applied as early as they can run. Bodies here have at most ten join literals.
    joins = [l for l in body if is_join(l)]
    if not any(isinstance(l, Atom) for l in joins):
        raise ValueError(f"rule for {head} has no positive atom")
    from itertools import combinations
    full = frozenset(range(len(joins)))
    best_of = {frozenset(): (0.0, 1.0, [], set(), [l for l in body if not is_join(l)])}
    for size_ in range(1, len(joins) + 1):
        for subset in map(frozenset, combinations(range(len(joins)), size_)):
            best = None
            for i in subset:
                prev = best_of.get(subset - {i})
                if prev is None: continue
                work, rows, order, bound, rest = prev
                if not order and not isinstance(joins[i], Atom): continue   # an anchor is an atom
                if not order and first is not None and joins[i] is not first: continue   # semi-naive: the delta first
                st = step(joins[i], rows, bound)
                if st is None: continue
                w, out, mode = st
                order2, bound2, rest2 = order + [(joins[i], mode)], bound | joins[i].vars(), list(rest)
                out = close(order2, out, bound2, rest2)
                cand = (work + w, out, order2, bound2, rest2)
                if best is None or cand[0] < best[0]: best = cand
            if best is not None: best_of[subset] = best
    if full not in best_of:
        raise ValueError(f"cannot order the body of {head}")
    work, rows, order, bound, rest = best_of[full]
    if rest: raise ValueError(f"unplaced literals {rest} in {head}")
    return order, work, rows

def make_plan(prog, db):
    stats = Stats(db, prog)
    ndv = {}
    for rel in prog.inputs:
        for col in range(len(prog.decls[rel])):
            rows, n, sq, _ = stats.col[(rel, sql_col(rel, col, prog))]
            stats.card[rel] = rows
            ndv[(rel, col)] = n or 1
            stats.fan[(rel, col)] = sq / rows if rows else 1.0
    # Input indexes as column lists, each ending in the rowid column if the table has one (SQLite appends it).
    # Schema, not statistics.
    for rel in prog.inputs:
        cols = {r[1]: r for r in db.execute(f"PRAGMA f.table_info({rel})")}
        rowid = [c for c, r in cols.items() if r[5] == 1 and r[2].upper() == "INTEGER"]
        stats.indexes[rel] = [rowid] if rowid else []
        for _, name, *_ in db.execute(f"PRAGMA f.index_list({rel})"):
            stats.indexes[rel].append([r[2] for r in db.execute(f"PRAGMA f.index_info({name})")] + rowid)
    return stats, ndv

def view_stats(prog, stats, ndv, rel, h, a):
    """A view over one input atom takes its statistics from the input's: a constant filters by its exact postings
    fraction, and each column keeps the input column's NDV (at most the view's rows) and fanout (scaled by the
    filter, assuming independence)."""
    base = stats.card[a.rel]
    rows = float(base)
    for c, t in enumerate(a.args):
        if isinstance(t, Const):
            rows *= (stats.const_freq(a.rel, sql_col(a.rel, c, prog), t.v) or 0) / max(1, base)
    stats.card[rel] = max(1, round(rows))
    src = {t.n: c for c, t in reversed(list(enumerate(a.args))) if isinstance(t, Var)}
    for c, t in enumerate(h.args):
        bc = src.get(t.n) if isinstance(t, Var) else None
        if bc is None: continue
        ndv[(rel, c)] = max(1, min(ndv[(a.rel, bc)], stats.card[rel]))
        stats.fan[(rel, c)] = max(1.0, stats.fan[(a.rel, bc)] * rows / max(1, base))

def sql_col(rel, col, prog):
    names = {"node": ["d", "n", "kind", "start", "end_"], "doc": ["d", "path", "module"], "child": ["p", "c", "idx"],
             "field": ["p", "f", "c"], "text": ["n", "s"], "intval": ["n", "v"], "op": ["n", "s"], "pkg": ["d", "p"]}
    if rel in names: return names[rel][col]
    return f"c{col}"

# ---- SQL backend -------------------------------------------------------------------------------------------------

def sql_term(t, env):
    if isinstance(t, Var): return env[t.n]
    if isinstance(t, Const): return repr(t.v) if isinstance(t.v, int) else "'" + t.v.replace("'", "''") + "'"
    if isinstance(t, TopT): return str(TOP)
    if isinstance(t, Call):
        a = [sql_term(x, env) for x in t.args]
        if t.f == "cat": return f"({a[0]} || '.' || {a[1]})"
        if t.f == "flat_bin":
            o, x, y = a
            big = 1 << 40
            return (f"(CASE WHEN {x} = {TOP} OR {y} = {TOP} THEN {TOP} WHEN {o} = '+' THEN {x} + {y} WHEN {o} = '-' THEN {x} - {y} "
                    f"WHEN {o} = '*' AND abs({x}) < {big} AND abs({y}) < {big} THEN {x} * {y} ELSE {TOP} END)")
    raise ValueError(t)

def table(prog, rel, which=""):
    if rel in prog.inputs: return f"f.{rel}"
    return f"{which}{rel}"

def sql_select(prog, head, order, delta_at=None, rec=set(), tables=None):
    """One SELECT for a planned rule body. `delta_at` is the index of the recursive atom that reads its delta table.
    `tables` maps a relation to the (full, delta) tables to read it from instead (the native set fixpoint's vtabs)."""
    env, frm, where, ctes = {}, [], [], []
    alias = [0]
    def a_name():
        alias[0] += 1; return f"t{alias[0]}"
    for idx, (l, mode) in enumerate(order):
        if isinstance(l, Atom) and not l.neg:
            al = a_name()
            src = (tables[l.rel][1 if idx == delta_at else 0] if tables and l.rel in tables
                   else table(prog, l.rel, "delta_" if idx == delta_at else ""))
            conds = []
            if mode == "range":   # docof($v, $d) with $d bound: $v lies in $d's node-id interval
                d = next(d for d, _ in order if isinstance(d, Atom) and d.rel in prog.docof
                         and isinstance(d.args[prog.docof[d.rel][0]], Var) and d.args[prog.docof[d.rel][0]].n in l.vars()
                         and (isinstance(d.args[prog.docof[d.rel][1]], Const) or d.args[prog.docof[d.rel][1]].n in env))
                v, dv = d.args[prog.docof[d.rel][0]], d.args[prog.docof[d.rel][1]]
                ra = a_name()
                frm.append((f"f.stat_docrange {ra}", [f"{ra}.d = {sql_term(dv, env)}"]))
                rc = next(c for c, t in enumerate(l.args) if isinstance(t, Var) and t.n == v.n)
                conds.append(f"{al}.{sql_col(l.rel, rc, prog)} BETWEEN {ra}.lo AND {ra}.hi")
            for col, t in enumerate(l.args):
                c = f"{al}.{sql_col(l.rel, col, prog)}"
                if isinstance(t, Var):
                    if t.n in env: conds.append(f"{c} = {env[t.n]}")
                    else: env[t.n] = c
                elif isinstance(t, (Const, TopT)):
                    conds.append(f"{c} = {sql_term(t, env)}")
            frm.append((f"{src} {al}", conds))
        elif isinstance(l, Atom) and l.neg:
            al = a_name()
            conds = [f"{al}.{sql_col(l.rel, col, prog)} = {sql_term(t, env)}" for col, t in enumerate(l.args) if not isinstance(t, Wild)]
            where.append(f"NOT EXISTS (SELECT 1 FROM {table(prog, l.rel)} {al} WHERE {' AND '.join(conds) or '1'})")
        elif isinstance(l, Cmp):
            if l.op == "=" and isinstance(l.l, Var) and l.l.n not in env:
                env[l.l.n] = sql_term(l.r, env)
            elif l.op == "=" and isinstance(l.l, Var) and is_cat(l.r) and not term_vars(l.r) <= set(env):
                # A split of the bound whole: at the "." a bound part places, else at every "." (`rdl_int` counts).
                w, (q, x) = env[l.l.n], l.r.args
                if q.n in env:
                    where.append(f"substr({w}, 1, length({env[q.n]}) + 1) = {env[q.n]} || '.'")
                    env[x.n] = f"substr({w}, length({env[q.n]}) + 2)"
                elif x.n in env:
                    where.append(f"substr({w}, -length({env[x.n]}) - 1) = '.' || {env[x.n]} AND length({w}) > length({env[x.n]})")
                    env[q.n] = f"substr({w}, 1, length({w}) - length({env[x.n]}) - 1)"
                else:
                    al = a_name()
                    frm.append((f"rdl_int {al}", [f"{al}.i <= length({w})", f"substr({w}, {al}.i, 1) = '.'"]))
                    env[q.n], env[x.n] = f"substr({w}, 1, {al}.i - 1)", f"substr({w}, {al}.i + 1)"
            else:
                op = {"=": "=", "!=": "<>"}.get(l.op, l.op)
                where.append(f"{sql_term(l.l, env)} {op} {sql_term(l.r, env)}")
        elif isinstance(l, Agg):
            al = a_name()
            conds = [f"{al}.{sql_col(l.atom.rel, col, prog)} = {sql_term(t, env)}" for col, t in enumerate(l.atom.args) if not isinstance(t, Wild)]
            env[l.var] = f"(SELECT count(*) FROM {table(prog, l.atom.rel)} {al} WHERE {' AND '.join(conds) or '1'})"
        elif isinstance(l, Nearest) and mode == "climb" and "nearest" in NATIVE:
            # The climb is a native function over the tape: no CTE, and its result is just a column expression.
            near = f"tape_nearest({env[l.n]}, '{','.join(prog.kindsets[l.kset])}')"
            if l.a in env: where.append(f"{near} = {env[l.a]}")
            else: env[l.a] = near; where.append(f"{near} IS NOT NULL")
        elif isinstance(l, Nearest):
            ks = ", ".join("'" + k + "'" for k in prog.kindsets[l.kset])
            # Everything joined so far becomes the seed CTE; the walk is a recursive CTE seeded only from it.
            k = len(ctes)
            seed_cols = sorted(env)
            seed_sel = ", ".join(f"{env[v]} AS {v}" for v in seed_cols)
            seed_from = " JOIN ".join(f"{f} ON {' AND '.join(c) or '1'}" if i else f for i, (f, c) in enumerate(frm))
            first_conds = frm[0][1] if frm else []
            seed_where = " AND ".join(first_conds + where) or "1"
            # rewrite: first FROM entry's conditions go to WHERE
            seed_from = frm[0][0] + "".join(f" CROSS JOIN {f}" for f, c in frm[1:]) if frm else ""
            seed_where = " AND ".join(first_conds + [x for _, c in frm[1:] for x in c] + where) or "1"
            ctes.append(f"seed{k} AS (SELECT DISTINCT {seed_sel} FROM {seed_from} WHERE {seed_where})")
            # The seed's columns are threaded through the walk (ast-index-spike S050 `_path_threaded`), so the walk's
            # output is the extended row and nothing joins back to the seed.
            cols = ", ".join(seed_cols)
            wcols = ", ".join(f"w.{v}" for v in seed_cols)
            scols = ", ".join(f"s.{v}" for v in seed_cols)
            if mode == "climb":
                ctes.append(f"walk{k}({cols}, cur) AS (SELECT {scols}, t.par FROM seed{k} s JOIN f.tape t ON t.n = s.{l.n} WHERE t.par >= 0 "
                            f"UNION ALL SELECT {wcols}, t.par FROM walk{k} w JOIN f.node k ON k.n = w.cur AND k.kind NOT IN ({ks}) "
                            f"JOIN f.tape t ON t.n = w.cur WHERE t.par >= 0)")
                cond = f" AND w.cur = w.{l.a}" if l.a in env else ""
                ctes.append(f"near{k} AS (SELECT {wcols}, w.cur AS near_out FROM walk{k} w CROSS JOIN f.node k WHERE k.n = w.cur AND k.kind IN ({ks}){cond})")
                out_var = l.a
            else:
                ctes.append(f"walk{k}({cols}, cur, stop) AS (SELECT {scols}, s.{l.a} + 1, s.{l.a} + t.size FROM seed{k} s JOIN f.tape t ON t.n = s.{l.a} WHERE t.size > 0 "
                            f"UNION ALL SELECT {wcols}, CASE WHEN k.kind IN ({ks}) THEN w.cur + t.size + 1 ELSE w.cur + 1 END, w.stop "
                            f"FROM walk{k} w JOIN f.node k ON k.n = w.cur JOIN f.tape t ON t.n = w.cur WHERE w.cur < w.stop)")
                cond = f" AND w.cur = w.{l.n}" if l.n in env else ""
                ctes.append(f"near{k} AS (SELECT {wcols}, w.cur AS near_out FROM walk{k} w WHERE w.cur <= w.stop{cond})")
                out_var = l.n
            env = {v: f"near{k}.{v}" for v in seed_cols}
            if out_var not in env: env[out_var] = f"near{k}.near_out"
            frm = [(f"near{k}", [])]
            where = []
            continue
    sel = ", ".join(f"{sql_term(t, env)} AS c{i}" for i, t in enumerate(head.args))
    from_sql = frm[0][0] + "".join(f" CROSS JOIN {f}" for f, c in frm[1:])
    w = " AND ".join(frm[0][1] + [x for _, c in frm[1:] for x in c] + where) or "1"
    with_sql = ("WITH RECURSIVE " + ", ".join(ctes) + " ") if ctes else ""
    return f"{with_sql}SELECT DISTINCT {sel} FROM {from_sql} WHERE {w}"

def run_sql(prog, facts, out, plan_path=None):
    db = sqlite3.connect(":memory:")
    db.execute(f"ATTACH 'file:{facts}?mode=ro' AS f")
    # RDL_MMAP: map the facts file and give it a page cache that holds it, so probes of the input relations' indexes
    # are memory reads rather than a pread per page miss (SQLite's default cache is 2 MB).
    if os.environ.get("RDL_MMAP", "1") != "0":
        db.execute("PRAGMA f.mmap_size = 4294967296")
        db.execute("PRAGMA f.cache_size = -1048576")
        db.execute("PRAGMA main.cache_size = -1048576")
        db.execute("PRAGMA temp.cache_size = -1048576")
        # Ephemeral b-trees (DISTINCT, sorts, materialized subqueries) have their own 2 MB cache and spill to a
        # temp file when it fills: keep them in memory.
        if os.environ.get("RDL_TEMPMEM", "1") != "0": db.execute("PRAGMA temp_store = MEMORY")
    # Positions in a symbol, for splitting one at its dots (`$m = @cat($q, $x)` with only $m bound).
    db.execute("CREATE TEMP TABLE rdl_int(i INTEGER PRIMARY KEY)")
    db.executemany("INSERT INTO rdl_int VALUES (?)", ((i,) for i in range(1, 4097)))
    db.commit()
    if NATIVE & {"nearest", "fixpoint", "kseed", "direct", "vtab", "setfix"}:
        db.enable_load_extension(True)
        db.load_extension(os.environ["RDL_EXT"])
        if "nearest" in NATIVE: db.execute("SELECT tape_load('f')")
    t0, c0 = time.time(), time.process_time()
    stats, ndv = make_plan(prog, db)
    prog.stats = stats
    t_stats = time.time() - t0
    sccs, edges = strata(prog)
    rules_by = defaultdict(list)
    for h, b in prog.rules: rules_by[h.rel].append((h, b))
    views = set()
    for rel in list(rules_by):
        rs = rules_by[rel]
        if len(rs) == 1 and rel not in prog.lattice and rel not in prog.facts:
            h, b = rs[0]
            if len(b) == 1 and isinstance(b[0], Atom) and not b[0].neg and b[0].rel in prog.inputs:
                order = [(b[0], None)]
                db.execute(f"CREATE TEMP VIEW {rel} AS {sql_select(prog, h, order).replace('SELECT DISTINCT', 'SELECT', 1)}")
                views.add(rel)
                a = b[0]
                view_stats(prog, stats, ndv, rel, h, a)
                src = {t.n: sql_col(a.rel, c, prog) for c, t in reversed(list(enumerate(a.args))) if isinstance(t, Var)}
                prog.view_src[rel] = (a.rel, [src.get(t.n) if isinstance(t, Var) else None for t in h.args])
                if (a.rel == "node" and len(h.args) == 2 and all(isinstance(t, Var) for t in h.args)
                        and isinstance(a.args[0], Var) and isinstance(a.args[1], Var) and all(isinstance(t, Wild) for t in a.args[2:])):
                    names = [t.n for t in h.args]
                    if sorted(names) == sorted([a.args[0].n, a.args[1].n]) and a.args[0].n != a.args[1].n:
                        prog.docof[rel] = (names.index(a.args[1].n), names.index(a.args[0].n))
    for rel, cols in prog.decls.items():
        if rel in prog.inputs or rel in views: continue
        # Typed columns: an untyped column compared with an INTEGER one gets numeric affinity applied to it, and
        # SQLite then can't use its index (reproduced in isolation; lessons ledger L11, narrow typed tables).
        sqlty = {"node": "INTEGER", "int": "INTEGER", "flat": "INTEGER", "sym": "TEXT"}
        names = ", ".join(f"c{i} {sqlty[t]}" for i, (_, t) in enumerate(cols))
        # RDL_CLUSTER: clustered on the key (WITHOUT ROWID), so an insert writes one b-tree, not a rowid table and
        # its unique index, and every secondary index carries the whole row (a probe never looks the row up).
        clus = " WITHOUT ROWID" if os.environ.get("RDL_CLUSTER", "1") != "0" else ""
        if rel in prog.lattice:
            keys = ", ".join(f"c{i}" for i in range(len(cols) - 1))
            db.execute(f"CREATE TABLE {rel}({names}, PRIMARY KEY ({keys})){clus}")
        elif clus:
            db.execute(f"CREATE TABLE {rel}({names}, PRIMARY KEY ({', '.join(f'c{i}' for i in range(len(cols)))})){clus}")
        else:
            db.execute(f"CREATE TABLE {rel}({names}, UNIQUE ({', '.join(f'c{i}' for i in range(len(cols)))}))")
        for row in prog.facts.get(rel, []):
            db.execute(f"INSERT OR IGNORE INTO {rel} VALUES ({', '.join('?' * len(row))})", row)
    plans, timing, iters, chosen, made = {}, {}, {}, [], set()
    native_rels = set()   # relations held only by the native set engine (RDL_NATIVE=program)
    def materialize(rels):
        for r in sorted(set(rels) & native_rels):
            db.execute("SELECT set_fixpoint_materialize(?)", (r,))
            native_rels.discard(r)
    t1, c1 = time.time(), time.process_time()
    for scc in sccs:
        ts = time.process_time()   # per-stratum timings are CPU seconds
        rules = [(h, b) for rel in sorted(scc) for (h, b) in rules_by[rel]]
        if "program" in NATIVE and not scc <= views:
            tp = time.process_time()
            if native_stratum(prog, db, scc, rules, stats, ndv, native_rels):
                iters["/".join(sorted(scc))] = "native"
                STAGE["native strata"] += time.process_time() - tp
                timing["/".join(sorted(scc))] = time.process_time() - ts
                if os.environ.get("RDL_TRACE"): print(f"  {'/'.join(sorted(scc))}: {time.process_time() - ts:.2f}s native stratum", flush=True)
                continue
            if scc & set(getattr(prog, "gates", {})): raise ValueError(f"gated negations in {sorted(scc)} need native steps for every rule")
            # This stratum runs in SQL, which reads the native relations it joins from their tables.
            materialize(l.atom.rel if isinstance(l, Agg) else l.rel for _, b in rules for l in b if isinstance(l, (Atom, Agg)))
        planned = []
        for h, b in rules:
            tp = time.process_time()
            order, work, _ = plan_rule(prog, stats, h, b, ndv)
            planned.append((h, order))
            STAGE["plan"] += time.process_time() - tp; tp = time.process_time()
            ensure_indexes(prog, db, order, views, made)
            STAGE["indexes"] += time.process_time() - tp
            anchor = order[0][0]
            walks = [(repr(l), m) for l, m in order if m]
            chosen.append({"head": repr(h), "anchor": repr(anchor), "walks": walks, "work": round(work)})
        if scc <= views:
            continue
        recursive = len(scc) > 1 or any(isinstance(l, Atom) and l.rel in scc and not l.neg for _, o in planned for l, _ in o)
        lattice_rels = [r for r in scc if r in prog.lattice]
        tp = time.process_time()
        if not recursive:
            for h, order in planned:
                sql = sql_select(prog, h, order)
                tr = time.time()
                if os.environ.get("RDL_EXPLAIN") == h.rel:
                    print(sql)
                    for r in db.execute("EXPLAIN QUERY PLAN " + sql): print("   ", r)
                insert_rows(prog, db, h, sql)
                if os.environ.get("RDL_EXPLAIN") == h.rel:
                    print(f"  rule took {time.time() - tr:.2f}s", flush=True)
        else:
            for r in scc:
                db.execute(f"CREATE TABLE delta_{r} AS SELECT * FROM {r} WHERE 0")
            if "fixpoint" in NATIVE and native_fixpoint(prog, db, scc, rules, stats, ndv, views, made):
                iters["/".join(sorted(scc))] = "native"
                for r in scc:
                    stats.card[r] = db.execute(f"SELECT count(*) FROM {r}").fetchone()[0]
                STAGE["recursive"] += time.process_time() - tp
                timing["/".join(sorted(scc))] = time.process_time() - ts
                if os.environ.get("RDL_TRACE"): print(f"  {'/'.join(sorted(scc))}: {time.process_time() - ts:.2f}s native", flush=True)
                continue
            if "setfix" in NATIVE and native_set_fixpoint(prog, db, scc, rules, stats, ndv, views, made):
                iters["/".join(sorted(scc))] = "native"
            else:
                # First round: every rule over the full (initially empty or seeded) relations.
                changed = semi_naive_round(prog, db, scc, planned, first=True)
                n = 1
                # Each delta variant is planned with its delta first, from the first round's counts: one order for
                # every variant put `bv` and `base` (unknown, so tiny) before `pv`'s delta, a 3.7 s product a round.
                for r in scc: stats.card[r] = max(1, db.execute(f"SELECT count(*) FROM {r}").fetchone()[0])
                # A still-growing relation scanned unjoined is priced as the native planner prices it (`atom_rows`).
                # RDL_SQL_REPLAN=1: replanned whenever a relation of the component has doubled since. Plans from the
                # first round's counts put a guard (`magic_pv_bff($c)`) after the joins it should restrict (no-demand
                # Python with magic sets: over 280 s, 117 s replanned), but replanning took the written programs from
                # 17 s to 71 s.
                def plan_deltas():
                    for r in scc if SQL_REPLAN else ():
                        ndv.update({(r, c): db.execute(f"SELECT count(DISTINCT c{c}) FROM {r}").fetchone()[0] or 1 for c in range(len(prog.decls[r]))})
                        for c in range(len(prog.decls[r])): stats.fan.pop((r, c), None)
                    out, stats.solving = [], set(scc)
                    for h, b in rules:
                        for a in [l for l in b if isinstance(l, Atom) and not l.neg and l.rel in scc]:
                            order, _, _ = plan_rule(prog, stats, h, b, ndv, first=a)
                            out.append((h, order, next(i for i, (l, _) in enumerate(order) if l is a)))
                    stats.solving = set()
                    return out, {r: stats.card[r] for r in scc}
                deltas, planned_at = plan_deltas()
                while changed:
                    changed = semi_naive_round(prog, db, scc, planned, first=False, deltas=deltas)
                    n += 1
                    if SQL_REPLAN:
                        for r in scc: stats.card[r] = max(1, db.execute(f"SELECT count(*) FROM {r}").fetchone()[0])
                    if os.environ.get("RDL_TRACE") == "2":
                        print(f"    round {n}: {changed} changed, {time.process_time() - ts:.1f}s", flush=True)
                    if SQL_REPLAN and any(stats.card[r] >= 2 * planned_at[r] for r in scc): deltas, planned_at = plan_deltas()
                iters["/".join(sorted(scc))] = n
        STAGE["recursive" if recursive else "non-recursive"] += time.process_time() - tp; tp = time.process_time()
        for r in scc:
            stats.card[r] = db.execute(f"SELECT count(*) FROM {r}").fetchone()[0]
            ndv.update({(r, c): db.execute(f"SELECT count(DISTINCT c{c}) FROM {r}").fetchone()[0] or 1 for c in range(len(prog.decls[r]))})
        STAGE["post-stratum stats"] += time.process_time() - tp
        timing["/".join(sorted(scc))] = time.process_time() - ts
        if os.environ.get("RDL_TRACE"):
            print(f"  {"/".join(sorted(scc))}: {time.process_time() - ts:.2f}s rows {[stats.card[r] for r in scc]}", flush=True)
            for c in chosen[-len(rules):]: print("     ", c, flush=True)
    t_eval, c_eval, c_stats = time.time() - t1, time.process_time() - c1, c1 - c0
    # Native joins read base facts into hash indexes: construction (the tape and anchor index), not execution.
    c_build = 0.0
    if "join" in NATIVE:
        bms, brows, dms, drows = db.execute("SELECT set_fixpoint_loads()").fetchone()[0].split()
        c_build = float(bms) / 1e3
        print(f"native input loads: base facts {brows} rows {float(bms) / 1e3:.2f}s cpu (construction), "
              f"lower strata {drows} rows {float(dms) / 1e3:.2f}s cpu (execution)")
    tp = time.process_time()
    materialize(prog.outputs)
    STAGE["materialize outputs"] += time.process_time() - tp
    c_eval = time.process_time() - c1
    os.makedirs(out, exist_ok=True)
    def dump(name, sql):
        rows = sorted("\t".join(map(str, r)) for r in db.execute(sql))
        open(f"{out}/{name}.csv", "w").write("".join(r + "\n" for r in rows))
        return len(rows)
    counts = {}
    for rel in prog.outputs:
        if rel == "finding":
            counts[rel] = dump("finding_at", "SELECT DISTINCT x.c0, d.path, k.start FROM finding x JOIN f.node k ON k.n = x.c1 JOIN f.doc d ON d.d = k.d")
        else:
            counts[rel] = dump(rel, f"SELECT * FROM {rel}")
    slow = sorted(timing.items(), key=lambda kv: -kv[1])[:6]
    sizes = {r: stats.card.get(r) for r in ("lookup", "type_of", "returns", "cv", "has_def", "fn_ref", "class_ref") + tuple(f"magic_{d}" for d in prog.demand) if r in stats.card}
    print(f"stats {t_stats:.2f}s eval {t_eval:.2f}s cpu stats {c_stats:.2f}s eval {c_eval:.2f}s exec {c_eval - c_build:.2f}s outputs {counts} iterations {iters}")
    if getattr(prog, "demand_dropped", None): print("demand dropped:", prog.demand_dropped)
    print("derived rows:", sizes)
    print("slowest strata:", ", ".join(f"{k} {v:.2f}s" for k, v in slow))
    for (r, w), v in sorted(PROFILE_RULES.items(), key=lambda kv: -kv[1])[:25]: print(f"  {v:6.3f}s {w}: {r}")
    if os.environ.get("RDL_PROFILE") and JOIN_MISS: print("no native join step:", dict(JOIN_MISS))
    if os.environ.get("RDL_PROFILE"): print("cpu by stage:", {k: round(v, 2) for k, v in STAGE.items()}, "strata", len(sccs))
    if os.environ.get("RDL_PROFILE"): print("lattice merge phases:", {k: round(v, 2) for k, v in PROFILE.items()})
    json.dump({"rules": chosen, "card": stats.card, "ndv": {f"{r}/{c}": v for (r, c), v in ndv.items()},
               "fan": {f"{r}/{c}": v for (r, c), v in stats.fan.items()},
               "const": {json.dumps(k[1:]): v for k, v in stats.memo.items() if k[0] == "const"},
               "kfreq": stats.kfreq, "size": stats.size, "parents": stats.parents, "nodes": stats.nodes},
              open(plan_path or f"{out}/plan.json", "w"), indent=1)

def native_fixpoint(prog, db, scc, rules, stats, ndv, views, made):
    """Runs a one-relation flat-lattice component natively (sqlite-ext `lattice_fixpoint`) if every rule is a seed
    (no recursive atom), a copy (`R($n, $v) :- ..., R($x, $v)`) or a binary transfer
    (`R($n, @flat_bin($o, $a, $b)) :- ..., R($l, $a), R($r, $b)`). The rest of each body is non-recursive, so SQL
    evaluates it once, planned as usual, into the component's seed and edge tables. Returns False to fall back."""
    if len(scc) != 1: return False
    rel = next(iter(scc))
    if rel not in prog.lattice or len(prog.decls[rel]) != 2: return False
    parts = []
    for h, body in rules:
        rec = [l for l in body if isinstance(l, Atom) and l.rel == rel]
        rest = [l for l in body if not (isinstance(l, Atom) and l.rel == rel)]
        if any(l.neg for l in rec): return False
        rvals = {l.args[1].n for l in rec if isinstance(l.args[1], Var)}
        if len(rvals) != len(rec) or any(rvals & l.vars() for l in rest if hasattr(l, "vars")): return False
        key, val = h.args
        kinds = [l for l in body if isinstance(l, Atom) and not l.neg and l.rel == "kind" and isinstance(key, Var)
                 and isinstance(l.args[0], Var) and l.args[0].n == key.n]
        if (not rec and "kseed" in NATIVE and len(kinds) == 1 and isinstance(kinds[0].args[1], Var)
                and all(key.n not in l.vars() for l in body if l is not kinds[0]) and key.n not in term_vars(val)):
            # A per-kind default (`R($n, v) :- kind($n, $k), <tests of $k>`): evaluated over the distinct kinds, and
            # the native side seeds every node of those kinds from the tape. No row per node.
            k = kinds[0].args[1]
            parts.append(("kseed", Atom(f"lfx_{rel}_kseed", [k, val]), [Atom("rdl_kinds", [k])] + [l for l in body if l is not kinds[0]]))
        elif not rec:
            parts.append(("seed", Atom(f"lfx_{rel}_seed", [key, val]), body))
        elif len(rec) == 1 and isinstance(val, Var) and val.n == rec[0].args[1].n:
            parts.append(("copy", Atom(f"lfx_{rel}_copy", [key, rec[0].args[0]]), rest))
        elif (len(rec) == 2 and isinstance(val, Call) and val.f == "flat_bin" and isinstance(val.args[1], Var)
              and isinstance(val.args[2], Var) and val.args[1].n == rec[0].args[1].n and val.args[2].n == rec[1].args[1].n):
            parts.append(("bin", Atom(f"lfx_{rel}_bin", [key, val.args[0], rec[0].args[0], rec[1].args[0]]), rest))
        else:
            return False
    if any(kind == "kseed" for kind, _, _ in parts) and "rdl_kinds" not in prog.decls:
        db.execute("CREATE TEMP VIEW rdl_kinds AS SELECT kind AS c0 FROM f.stat_kind")
        prog.decls["rdl_kinds"] = [("k", "sym")]
        views.add("rdl_kinds")
        stats.card["rdl_kinds"] = ndv[("rdl_kinds", 0)] = len(stats.kfreq)
        if "nearest" not in NATIVE: db.execute("SELECT tape_load('f')")
    for kind, cols in (("seed", 2), ("copy", 2), ("bin", 4), ("kseed", 2)):
        db.execute(f"DROP TABLE IF EXISTS lfx_{rel}_{kind}")
        if kind != "kseed" or any(k == "kseed" for k, _, _ in parts):
            db.execute(f"CREATE TEMP TABLE lfx_{rel}_{kind}({', '.join(f'c{i}' for i in range(cols))})")
    t = time.time()
    for kind, head, body in parts:
        order, _, _ = plan_rule(prog, stats, head, body, ndv)
        ensure_indexes(prog, db, order, views, made)
        db.execute(f"INSERT INTO {head.rel} {sql_select(prog, head, order)}")
    t_sql = time.time() - t
    t = time.time()
    if "vtab" in NATIVE:   # kept in memory; the relation becomes a virtual table over it (no rows written)
        out = db.execute(f"SELECT lattice_fixpoint('lfx_{rel}', '@{rel}')").fetchone()[0]
        db.execute(f"DROP TABLE {rel}")
        db.execute(f"CREATE VIRTUAL TABLE temp.{rel} USING lattice_result({rel})")
        views.add(rel)
    elif "direct" in NATIVE:   # written straight into the relation: no output table to copy back
        out = db.execute(f"SELECT lattice_fixpoint('lfx_{rel}', '{rel}')").fetchone()[0]
    else:
        out = db.execute(f"SELECT lattice_fixpoint('lfx_{rel}')").fetchone()[0]
        t = time.time()
        db.execute(f"INSERT INTO {rel} SELECT c0, c1 FROM lfx_{rel}_out")
    if os.environ.get("RDL_PROFILE"):
        rows, waves, visits, solve, total = out.split()
        sizes = [db.execute(f"SELECT count(*) FROM lfx_{rel}_{k}").fetchone()[0] for k in ("seed", "copy", "bin", "kseed")
                 if db.execute(f"SELECT 1 FROM sqlite_temp_master WHERE name = 'lfx_{rel}_{k}'").fetchone()]
        print(f"    native {rel}: edge/seed SQL {t_sql:.2f}s (seed/copy/bin rows {sizes}), fixpoint {solve} ms solve "
              f"{total} ms with reads and writes, {waves} waves, {visits} visits, {rows} rows; copy back {time.time() - t:.2f}s", flush=True)
    return True

def live_stats(stats, ndv, r, line):
    """A relation's statistics from the native store (`set_fixpoint_stats`): rows, then per column the distinct
    count and the sum of squared postings, whose ratio to rows is the skew-weighted fanout ingestion gives inputs."""
    nums, *freqs = line.split(" |")
    n, *rest = map(int, nums.split())
    k = len(rest) // 2
    stats.card[r] = max(1.0, n)
    for c in range(k):
        ndv[(r, c)] = max(1, rest[c])
        stats.fan[(r, c)] = rest[k + c] / n if n else 1.0
        stats.dfreq.pop((r, c), None)
    for f in freqs:
        c, vals = f.split(":", 1)
        typ = prog_types(stats, r)[int(c)]
        stats.dfreq[(r, int(c))] = {(bytes.fromhex(v).decode() if typ == "sym" else int(bytes.fromhex(v).decode())): int(m)
                                    for v, m in (x.split("=") for x in vals.split(",") if x)}

def prog_types(stats, r): return [t for _, t in stats.prog.decls[r]]

def join_spec(prog, h, order, at, ri, scc, native_rels=frozenset()):
    """A probe's planned body as a native join spec (sqlite-ext `setfix.rs`, "native join probes"), or None where a
    literal has no native step (an aggregate, a descent, a document range, top): that probe stays SQL."""
    slots, lines = {}, []
    def typ(t):
        if isinstance(t, Const): return "int" if isinstance(t.v, int) else "sym"
        return next((prog.decls[l.rel][c][1] for l, _ in order if isinstance(l, Atom) and not l.neg
                     for c, x in enumerate(l.args) if isinstance(x, Var) and x.n == t.n), None)
    def tok(t):
        if isinstance(t, Wild): return "_"
        if isinstance(t, Var): return f"v{slots.setdefault(t.n, len(slots))}"
        if isinstance(t, Const): return f"i:{t.v}" if isinstance(t.v, int) else "x:" + t.v.encode().hex()
        raise ValueError(t)
    def atom(l, idx):
        """The atom's source and tokens. An input relation is loaded with its constant columns as a filter and
        projected away (`field($n, "path", $p)` loads the `path` edges only), so it is read in the size it is used."""
        if l.rel in scc: return ("d" if idx == at else "c") + str(ri[l.rel]), [tok(t) for t in l.args]
        if l.rel in native_rels: return "r:" + l.rel.encode().hex(), [tok(t) for t in l.args]
        keep = [c for c, t in enumerate(l.args) if not isinstance(t, Const)]
        cols = ", ".join(sql_col(l.rel, c, prog) for c in keep) or "1"
        where = " AND ".join(f"{sql_col(l.rel, c, prog)} = {sql_term(t, {})}" for c, t in enumerate(l.args) if isinstance(t, Const))
        types = "".join("s" if prog.decls[l.rel][c][1] == "sym" else "i" for c in keep) or "i"
        sel = table(prog, l.rel) + (f" WHERE {where}" if where else "")
        toks = [tok(l.args[c]) for c in keep] or ["_"]
        base = "b" if l.rel in prog.inputs or l.rel in prog.view_src else "d"
        return f"t:{base}:{sel.encode().hex()}:{cols.encode().hex()}:{types}", toks
    try:
        for idx, (l, mode) in enumerate(order):
            if isinstance(l, Atom):
                if mode == "range": JOIN_MISS[f"document range {l}"] += 1; return None
                sc, toks = atom(l, idx)
                lines.append(f"{'neg' if l.neg else 'pos'} {sc} {' '.join(toks)}")
            elif isinstance(l, Cmp) and l.op in ("=", "!="):
                if isinstance(l.r, Call):
                    if l.op != "=" or l.r.f != "cat" or not isinstance(l.l, Var): return None
                    a, b = l.r.args
                    lines.append(f"cat {tok(l.l)} {tok(a)} {tok(b)}")
                else:
                    a, b = l.l, l.r
                    if isinstance(b, Var) and b.n not in slots: a, b = b, a   # the bound side goes right
                    if isinstance(b, Var) and b.n not in slots: return None
                    lines.append(f"{'eq' if l.op == '=' else 'ne'} {tok(a)} {tok(b)}")
            elif isinstance(l, Cmp) and l.op in ("<", "<=", ">", ">="):
                # Integers (and nodes) only: symbols are numbered in the order they are met.
                if any(isinstance(t, Call) or (isinstance(t, Var) and t.n not in slots) or typ(t) in (None, "sym") for t in (l.l, l.r)): return None
                a, b = (l.l, l.r) if l.op in ("<", "<=") else (l.r, l.l)
                lines.append(f"{'lt' if l.op in ('<', '>') else 'le'} {tok(a)} {tok(b)}")
            elif isinstance(l, Agg):
                if any(isinstance(t, Var) and t.n not in slots for t in l.atom.args): return None
                sc, toks = atom(l.atom, -1)
                lines.append(f"count {sc} {' '.join(toks)} {tok(Var(l.var))}")
            elif isinstance(l, Nearest) and mode == "climb":
                lines.append(f"near {tok(Var(l.n))} {tok(Var(l.a))} {','.join(prog.kindsets[l.kset]).encode().hex()}")
            elif isinstance(l, Nearest) and l.a in slots and l.n not in slots:
                lines.append(f"down {tok(Var(l.n))} {tok(Var(l.a))} {','.join(prog.kindsets[l.kset]).encode().hex()}")
            else:
                JOIN_MISS[f"{type(l).__name__} {mode or ''}"] += 1
                return None
        head = []
        for t in h.args:
            if isinstance(t, Call) and t.f == "cat" and all(isinstance(x, Var) for x in t.args):
                head.append(f"cat:{tok(t.args[0])}:{tok(t.args[1])}")
            else: head.append(tok(t))
        lines.append("head " + " ".join(head))
    except ValueError:
        return None
    return "\n".join(lines)

def timed_plan(*a, **k):
    t = time.process_time()
    try: return plan_rule(*a, **k)
    finally: STAGE["native: planning (python)"] += time.process_time() - t

WHOLE = os.environ.get("RDL_WHOLE", "1") != "0"   # native probes get a plan for their delta's first, whole wave

def native_stratum(prog, db, scc, rules, stats, ndv, native_rels):
    stats.solving = set(scc) if any(l.rel in scc for _, b in rules for l in b if isinstance(l, Atom) and not l.neg) else set()
    try: return native_stratum_(prog, db, scc, rules, stats, ndv, native_rels)
    finally: stats.solving = set()

def native_stratum_(prog, db, scc, rules, stats, ndv, native_rels):
    """One stratum on the native set engine, as a component of seed joins (rules without a recursive atom, run once)
    and join probes (one per recursive atom, planned with its delta first), all planned by `plan_rule` and replanned
    from live counts between chunks of waves. Lower strata it reads natively come from the native store (`r:`),
    others from their tables or the facts. Nothing is written back: the relations join `native_rels`. False (and
    nothing run) where a rule has a literal with no native step."""
    if any(r in prog.lattice for r in scc): return False
    rels = sorted(scc)
    ri = {r: i for i, r in enumerate(rels)}
    name = f"prog_{rels[0]}"
    gates = sorted((g for g in getattr(prog, "gates", {}) if g in scc), key=lambda g: (prog.gates[g]["level"], g))
    rules = [(h, b) for h, b in rules if h.rel not in gates]   # a gate's rule only placed it here
    rec = lambda b: [l for l in b if isinstance(l, Atom) and not l.neg and l.rel in scc]
    seeds = [(h, b) for h, b in rules if not rec(b)]
    lines = [f"name {name}"] + [f"rel {r} {''.join('s' if t == 'sym' else 'i' for _, t in prog.decls[r])}" for r in rels] + ["nowrite"]
    est, shown = defaultdict(float), []
    for h, b in seeds:
        order, _, rows = timed_plan(prog, stats, h, b, ndv)
        spec = join_spec(prog, h, order, None, ri, scc, native_rels)
        if spec is None: return False
        est[h.rel] += rows
        shown.append((h, "seed", order))
        if os.environ.get("RDL_TRACE") == "2": print(f"    seed {len(shown) - 1} {h} ({rows:.0f} est.): {[str(l) for l, _ in order]}", flush=True)
        lines.append(f"seed {ri[h.rel]} {spec.encode().hex()}")
    for r in rels: stats.card[r] = max(1.0, est[r])
    probes = []
    for h, b in rules:
        for a in rec(b):
            order, _, _ = timed_plan(prog, stats, h, b, ndv, first=a)
            at = next(i for i, (l, _) in enumerate(order) if l is a)
            spec = join_spec(prog, h, order, at, ri, scc, native_rels)
            if spec is None: return False
            probes.append((h, b, a, order))
            # The wave whose delta is all of the relation (it was empty before): any order, the atom read in full.
            whole, _, _ = timed_plan(prog, stats, h, b, ndv)
            wspec = join_spec(prog, h, whole, None, ri, scc, native_rels) if whole[0][0] is not a and WHOLE else None
            lines.append(f"join {ri[h.rel]} {ri[a.rel]} {spec.encode().hex()}" + (f" {wspec.encode().hex()}" if wspec else ""))
            if os.environ.get("RDL_TRACE") == "2":
                print(f"    probe {len(seeds) + len(probes) - 1} {h} delta {a}: {[str(l) for l, _ in order]}", flush=True)
                if os.environ.get("RDL_SPEC"): print("      " + spec.replace("\n", " | "), flush=True)
    for g in gates:
        d = prog.gates[g]
        idx = lambda rs: ",".join(str(ri[r]) for r in rs if r in ri) or "-"
        lines.append(f"gate {ri[g]} {ri[d['src']] if d['src'] else '-'} {idx(d['quiet'])} {','.join(str(gates.index(x)) for x in d['deps'] if x in gates) or '-'}")
        if os.environ.get("RDL_TRACE") == "2": print(f"    gate {g}: from {d['src'] or 'unit'}, level {d['level']}, quiet {d['quiet']}, after {d['deps']}", flush=True)
    to = time.process_time()
    db.execute("SELECT set_fixpoint_open(?)", ("\n".join(lines),))
    STAGE["native: open (loads, indexes)"] += time.process_time() - to
    chunk, replanned, waves = int(os.environ.get("RDL_REPLAN", "2")), 0, 0
    per = defaultdict(lambda: [0.0, 0])
    seed_orders = [o for _, _, o in shown]
    while True:
        tr = time.process_time()
        f = db.execute("SELECT set_fixpoint_run(?, ?)", (name, chunk)).fetchone()[0].split()
        STAGE["native: run"] += time.process_time() - tr
        waves += int(f[1])
        for o, line in zip(seed_orders + [p[3] for p in probes], db.execute("SELECT set_fixpoint_steps(?)", (name,)).fetchone()[0].split("\n")):
            observe(stats, o, line)
        for k, x in enumerate(f[7].split(",") if len(f) > 8 else []):
            ms, n = x.split(":"); per[k][0] += float(ms); per[k][1] += int(n)
            if os.environ.get("RDL_TRACE") == "2" and float(ms) > 200:
                o = (seed_orders + [p[3] for p in probes])[k]
                print(f"      chunk to wave {waves}: {float(ms):.0f} ms, {n} derived: {[str(l) for l, _ in o]}", flush=True)
        live = db.execute("SELECT set_fixpoint_stats(?)", (name,)).fetchone()[0].split("\n")
        for r, line in zip(rels, live): live_stats(stats, ndv, r, line)
        if f[-1] == "1" or chunk == 0: break
        for k, (h, b, a, order) in enumerate(probes):
            new, _, _ = timed_plan(prog, stats, h, b, ndv, first=a)
            if [str(l) for l, _ in new] == [str(l) for l, _ in order]: continue
            spec = join_spec(prog, h, new, next(i for i, (l, _) in enumerate(new) if l is a), ri, scc, native_rels)
            db.execute("SELECT set_fixpoint_reprobe(?, ?, 'join', ?)", (name, len(seeds) + k, spec.encode().hex()))
            probes[k] = (h, b, a, new); replanned += 1
            if os.environ.get("RDL_TRACE") == "2":
                print(f"    replan {len(seeds) + k} at wave {waves}: {[str(l) for l, _ in new]}", flush=True)
                if os.environ.get("RDL_SPEC"): print("      " + spec.replace("\n", " | "), flush=True)
        chunk *= 2
    db.execute("SELECT set_fixpoint_close(?)", (name,))
    native_rels.update(scc)
    if os.environ.get("RDL_PROFILE") == "2":
        print(f"    native stratum {'/'.join(rels)}: {len(seeds)} seeds, {len(probes)} probes, {waves} waves, {replanned} replanned, "
              f"rows {[int(stats.card[r]) for r in rels]}", flush=True)
        shown += [(h, f"delta {a.rel}", order) for h, b, a, order in probes]
        for k, (h, what, order) in sorted(enumerate(shown), key=lambda x: -per[x[0]][0])[:3]:
            if per[k][0] >= 20:
                print(f"      {per[k][0]:.1f} ms, {per[k][1]} derived: {h} {what} order {[str(l) for l, _ in order]}", flush=True)
    return True

def native_set_fixpoint(prog, db, scc, rules, stats, ndv, views, made):
    """Runs a component of set-valued relations natively (sqlite-ext `set_fixpoint_*`, `set_rel`). Seeds (rules
    with no recursive atom) run once in SQL. Each recursive rule is then one of two kinds, chosen by cost:

    - **Edge.** A linear rule (one recursive atom) whose non-recursive rest SQL evaluates once into an edge table:
      the rest's join with the atom's variables free, projected onto the variables it shares with the atom (the key)
      and those the head needs. A wave joins the atom's delta with the edges through a hash index.
      Chosen when that is cheaper than one delta pass at the estimated seed sizes, which is a lower bound on
      evaluating the rule by delta: `type_of`'s copy graph qualifies, while `resolves` (71k attribute edges for 12k
      rows) does not. A rest that costs more than one pass over the tape on its own is never an edge table:
      `mirror`'s rest is a product of children on the index, and a demand-guarded rest is the undemanded relation.
    - **Probe.** Every other rule, including those with several recursive atoms. Its SQL runs once per wave for
      each recursive atom whose delta is non-empty, planned with that atom first and reading it from a `set_rel`
      delta vtab, and the other recursive atoms from full ones.

    The native side keeps the tuples, deduplicates them and schedules the waves, so there is no DDL, EXCEPT or count
    per round, and a probe whose delta is empty does not run."""
    if any(r in prog.lattice for r in scc): return False
    rels = sorted(scc)
    ri = {r: i for i, r in enumerate(rels)}
    name = f"sfx{len(made)}_{rels[0]}"
    seeds = [(h, b) for h, b in rules if not any(isinstance(l, Atom) and not l.neg and l.rel in scc for l in b)]
    seed_plans = [(h, plan_rule(prog, stats, h, b, ndv)) for h, b in seeds]
    for r in rels: stats.card[r] = max(1.0, sum(rows for h, (_, _, rows) in seed_plans if h.rel == r))
    def var_type(v, body):
        for l in body:
            if isinstance(l, Atom) and not l.neg:
                for c, t in enumerate(l.args):
                    if isinstance(t, Var) and t.n == v: return "s" if prog.decls[l.rel][c][1] == "sym" else "i"
        return None
    def const_tok(v): return f"i:{v}" if isinstance(v, int) else "x:" + v.encode().hex()
    def edge_rule(j, h, body, a, rest):
        """The spec line and edge plan of a linear rule, or the reason it is a probe."""
        restvars = set().union(*(l.vars() for l in rest)) if rest else set()
        hvars = set().union(*(term_vars(t) for t in h.args))
        key = sorted(a.vars() & restvars)
        ecols = sorted((hvars & restvars) - a.vars())
        if hvars - a.vars() - restvars: return "head variable from nowhere"
        slot = {v: i for i, v in enumerate(sorted(a.vars()) + ecols)}
        pat, head = [], []
        for t in a.args:
            if isinstance(t, Wild): pat.append("_")
            elif isinstance(t, Var): pat.append(f"v{slot[t.n]}")
            elif isinstance(t, Const): pat.append(const_tok(t.v))
            else: return "pattern term"
        for t in h.args:
            if isinstance(t, Var): head.append(f"v{slot[t.n]}")
            elif isinstance(t, Const): head.append(const_tok(t.v))
            elif isinstance(t, Call) and t.f == "cat" and all(isinstance(x, Var) for x in t.args):
                head.append(f"cat:v{slot[t.args[0].n]}:v{slot[t.args[1].n]}")
            else: return "head term"
        edge = None
        if rest:
            if not any(isinstance(l, Atom) and not l.neg for l in rest): return "rest has no atom"
            ehead = Atom(f"sfx_{h.rel}_{j}", [Var(v) for v in key + ecols])
            try: order, work, _ = plan_rule(prog, stats, ehead, rest, ndv)
            except ValueError: return "rest cannot be planned alone"
            if work > stats.nodes: return f"rest costs {work:.0g} > {stats.nodes} nodes alone"
            delta = plan_rule(prog, stats, h, body, ndv, first=a)[1]
            if work > delta: return f"edges cost {work:.0f} > one delta pass {delta:.0f}"
            edge = (ehead, order)
            src = f"{ehead.rel} {''.join(var_type(v, rest) or 'i' for v in key + ecols) or '-'}"
        else:
            src = "- -"
        line = (f"rule {ri[h.rel]} {ri[a.rel]} {src} key {' '.join(f'v{slot[v]}' for v in key)} ; "
                f"ecols {' '.join(f'v{slot[v]}' for v in ecols)} ; pat {' '.join(pat)} ; head {' '.join(head)}")
        return line, edge
    tables = {r: (f"sfx_f_{r}", f"sfx_d_{r}") for r in rels}
    lines = [f"name {name}"] + [f"rel {r} {''.join('s' if t == 'sym' else 'i' for _, t in prog.decls[r])}" for r in rels]
    edges, probes, kinds, pbody = [], [], [], []
    for j, (h, body) in enumerate(rules):
        rec = [l for l in body if isinstance(l, Atom) and not l.neg and l.rel in scc]
        if not rec: continue
        why = "several recursive atoms"
        if len(rec) == 1:
            got = edge_rule(j, h, body, rec[0], [l for l in body if l is not rec[0]])
            if isinstance(got, tuple):
                lines.append(got[0])
                if got[1]: edges.append(got[1])
                kinds.append("edge"); continue
            why = got
        for a in rec:   # a probe per recursive atom, planned with that atom's delta first
            order, _, _ = plan_rule(prog, stats, h, body, ndv, first=a)
            probes.append((h, order, next(i for i, (l, _) in enumerate(order) if l is a), a.rel))
            pbody.append((body, a))
        kinds.append("probe")
        if os.environ.get("RDL_TRACE"): print(f"  set fixpoint: probe {h} ({why})", flush=True)
    t = time.time()
    for h, (order, _, _) in seed_plans:
        ensure_indexes(prog, db, order, views, made)
        insert_rows(prog, db, h, sql_select(prog, h, order))
    for ehead, order in edges:
        ensure_indexes(prog, db, order, views, made)
        db.execute(f"DROP TABLE IF EXISTS {ehead.rel}")
        db.execute(f"CREATE TEMP TABLE {ehead.rel}({', '.join(f'c{i}' for i in range(len(ehead.args)))})")
        if ehead.args: db.execute(f"INSERT INTO {ehead.rel} {sql_select(prog, ehead, order)}")
    njoin = 0
    for h, order, at, rel in probes:
        spec = join_spec(prog, h, order, at, ri, scc) if "join" in NATIVE else None
        if spec is not None:
            lines.append(f"join {ri[h.rel]} {ri[rel]} {spec.encode().hex()}"); njoin += 1; continue
        ensure_indexes(prog, db, order, views, made)
        sql = sql_select(prog, h, order, delta_at=at, tables=tables)
        lines.append(f"probe {ri[h.rel]} {ri[rel]} {sql.encode().hex()}")
    t_sql = time.time() - t
    db.execute("SELECT set_fixpoint_open(?)", ("\n".join(lines),))
    for r in rels:
        for which, vt in zip(("full", "delta"), tables[r]):
            db.execute(f"CREATE VIRTUAL TABLE temp.{vt} USING set_rel({name}, {ri[r]}, {which})")
    # The probes were planned from seed estimates, and a derived relation's columns have no distinct counts then
    # (each is taken as 1, so a join on one looks unselective). Run a few waves, take the live counts from the native
    # store's hash indexes, replan the probes and resume, doubling the chunk until the fixpoint.
    chunk = int(os.environ.get("RDL_REPLAN", "2"))
    acc, waves_n, replanned, per_acc = [0, 0, 0, 0, 0.0], 0, 0, [[0.0, 0] for _ in probes]
    while True:
        out = db.execute("SELECT set_fixpoint_run(?, ?)", (name, chunk)).fetchone()[0]
        f = out.split()
        for k, (ms, n) in enumerate(x.split(":") for x in (f[7] if len(f) > 8 else "").split(",") if x):
            per_acc[k][0] += float(ms); per_acc[k][1] += int(n)
        acc = [acc[0] + int(f[0]), acc[1] + int(f[1]), acc[2] + int(f[2]), acc[3] + int(f[3]), acc[4] + float(f[5])]
        if f[-1] == "1" or chunk == 0: break
        live = db.execute("SELECT set_fixpoint_stats(?)", (name,)).fetchone()[0].split("\n")
        for r, line in zip(rels, live): live_stats(stats, ndv, r, line)
        for k, ((h, order, at, rel), (body, a)) in enumerate(zip(probes, pbody)):
            new, _, _ = plan_rule(prog, stats, h, body, ndv, first=a)
            if [str(l) for l, _ in new] == [str(l) for l, _ in order]: continue
            nat = next(i for i, (l, _) in enumerate(new) if l is a)
            spec = join_spec(prog, h, new, nat, ri, scc) if "join" in NATIVE else None
            if spec is not None:
                db.execute("SELECT set_fixpoint_reprobe(?, ?, 'join', ?)", (name, k, spec.encode().hex()))
            else:
                ensure_indexes(prog, db, new, views, made)
                db.execute("SELECT set_fixpoint_reprobe(?, ?, 'sql', ?)", (name, k, sql_select(prog, h, new, delta_at=nat, tables=tables).encode().hex()))
            probes[k] = (h, new, nat, rel); replanned += 1
        chunk *= 2
    out = " ".join(map(str, acc[:4] + [0, f"{acc[4]:.1f}", f[6], ",".join(f"{t:.1f}:{n}" for t, n in per_acc)]))
    for r in rels:
        for vt in tables[r]: db.execute(f"DROP TABLE temp.{vt}")
    db.execute("SELECT set_fixpoint_close(?)", (name,))
    made.add(("sfx", name))
    if os.environ.get("RDL_PROFILE"):
        rows, waves, visits, runs, skips, solve, total, per = (out.split() + [""])[:8]
        sizes = [db.execute(f"SELECT count(*) FROM {e.rel}").fetchone()[0] for e, _ in edges]
        print(f"    native set {'/'.join(rels)}: {kinds.count('edge')} edge and {kinds.count('probe')} probe rules ({njoin} of {len(probes)} probes native joins); seed and edge SQL "
              f"{t_sql:.2f}s (edge rows {sizes}); {waves} waves, {runs} probe statements run and {skips} skipped, {visits} visits, "
              f"{rows} new rows; {solve} ms solve, {total} ms with writes; {replanned} probes replanned", flush=True)
        if os.environ.get("RDL_PROFILE") == "2" and per:
            slow = sorted(zip(per.split(","), probes), key=lambda x: -float(x[0][0].split(":")[0]))[:8]
            for (ms, (h, order, at, rel)) in ((p.split(":"), q) for p, q in slow):
                print(f"      probe {ms[0]} ms, {ms[1]} rows: {h} delta {rel} order {[str(l) for l, _ in order]}", flush=True)
    return True

def ensure_indexes(prog, db, order, views, made):
    """A composite index on each derived relation for every bound-column pattern the plan probes it with."""
    bound = set()
    for l, _ in order:
        a = l.atom if isinstance(l, Agg) else l
        if isinstance(a, Atom) and a.rel in views and a.rel in prog.view_src and not PLAN0:
            # A view over an input is probed through the input's indexes. If none serves this probe, SQLite builds an
            # automatic index per statement, which for a filtered view (`modnode`: node where kind = "module") scans
            # every node, every round. Materialize it once instead; it then gets indexes like any derived relation.
            base, cols = prog.view_src[a.rel]
            probed = {cols[c] for c, t in enumerate(a.args) if isinstance(t, Const) or (isinstance(t, Var) and t.n in bound)}
            if probed and not any(ix and ix[0] in probed for ix in prog.stats.indexes.get(base, [])):
                db.execute(f"CREATE TEMP TABLE {a.rel}__m AS SELECT * FROM {a.rel}")
                db.execute(f"DROP VIEW {a.rel}")
                db.execute(f"ALTER TABLE {a.rel}__m RENAME TO {a.rel}")
                views.discard(a.rel)
                del prog.view_src[a.rel]
                if os.environ.get("RDL_TRACE"): print(f"  materialized view {a.rel} for a probe on {sorted(probed)} in {order}", flush=True)
        if isinstance(a, Atom) and a.rel not in prog.inputs and a.rel not in views:
            cols = [c for c, t in enumerate(a.args) if isinstance(t, Const) or (isinstance(t, Var) and t.n in bound)]
            if cols and (a.rel, tuple(cols)) not in made:
                made.add((a.rel, tuple(cols)))
                name = f"{a.rel}_" + "_".join(map(str, cols))
                db.execute(f"CREATE INDEX IF NOT EXISTS {name} ON {a.rel}({', '.join(f'c{c}' for c in cols)})")
        if hasattr(l, "vars"): bound |= l.vars()

def insert_rows(prog, db, head, select):
    if head.rel in prog.lattice:
        merge_lattice(prog, db, head.rel, select, delta=False)
    else:
        db.execute(f"INSERT OR IGNORE INTO {head.rel} {select}")

PROFILE_EXPLAINED = set()
PROFILE_RULES = {}   # RDL_PROFILE=2: seconds per (rule, first round or delta position), each body timed alone
JOIN_MISS = defaultdict(int)   # literals with no native join step, by kind (RDL_PROFILE=1 prints them)
STAGE = defaultdict(float)     # RDL_PROFILE=1: CPU seconds per evaluation stage, summed over strata
PROFILE = defaultdict(float)   # RDL_PROFILE=1: seconds per lattice merge phase, summed over rounds

def merge_lattice(prog, db, rel, select, delta):
    """Joins candidate values per key (flat lattice: equal stays, different becomes top) into `rel`; with `delta`,
    records the keys whose value changed in new_<rel>. Returns the number of changed keys."""
    k = len(prog.decls[rel]) - 1
    keys = ", ".join(f"c{i}" for i in range(k))
    v = f"c{k}"
    if "upsert" in NATIVE:
        return merge_upsert(prog, db, rel, select, delta)
    t = time.time()
    def lap(phase):
        nonlocal t
        now = time.time(); PROFILE[phase] += now - t; t = now
    db.execute(f"DROP TABLE IF EXISTS cand")
    db.execute(f"CREATE TEMP TABLE cand AS SELECT * FROM ({select}) WHERE 0")
    lap("create cand")
    db.execute(f"INSERT INTO cand {select}")
    lap("candidates (rule bodies)")
    on = " AND ".join(f"r.c{i} = j.c{i}" for i in range(k))
    joined = (f"SELECT {keys}, CASE WHEN count(DISTINCT {v}) > 1 OR max({v}) = {TOP} THEN {TOP} ELSE min({v}) END AS {v} "
              f"FROM cand GROUP BY {keys}")
    changed_sql = (f"SELECT j.* FROM ({joined}) j LEFT JOIN {rel} r ON {on} "
                   f"WHERE r.{v} IS NULL OR (r.{v} <> {TOP} AND r.{v} <> j.{v})")
    db.execute(f"DROP TABLE IF EXISTS chg")
    db.execute(f"CREATE TEMP TABLE chg AS {changed_sql}")
    lap("group-by join + compare with old")
    # A changed key whose old value was a different constant becomes top.
    db.execute(f"UPDATE chg SET {v} = {TOP} WHERE EXISTS (SELECT 1 FROM {rel} r WHERE {' AND '.join(f'r.c{i} = chg.c{i}' for i in range(k))} AND r.{v} <> chg.{v})")
    lap("update to top")
    db.execute(f"INSERT OR REPLACE INTO {rel} SELECT * FROM chg")
    lap("insert or replace")
    n = db.execute("SELECT count(*) FROM chg").fetchone()[0]
    if delta:
        db.execute(f"INSERT INTO new_{rel} SELECT * FROM chg")
    lap("delta copy")
    if os.environ.get("RDL_PROFILE"):
        PROFILE["rounds"] += 1
        print(f"    merge {rel}: {db.execute('SELECT count(*) FROM cand').fetchone()[0]} candidates, {n} changed", flush=True)
    return n

def merge_upsert(prog, db, rel, select, delta):
    """The same merge as one UPSERT: each candidate either inserts its key or joins into the stored value (flat
    lattice: a different constant makes top). Triggers record the keys that changed, so there is no candidate table,
    GROUP BY, or compare pass. A general lattice needs its join as a native function in the SET and WHERE."""
    k = len(prog.decls[rel]) - 1
    keys = ", ".join(f"c{i}" for i in range(k))
    v = f"c{k}"
    t = time.time()
    if not db.execute(f"SELECT 1 FROM sqlite_temp_master WHERE name = 'chg_{rel}'").fetchone():
        cols = ", ".join(f"c{i}" for i in range(k + 1))
        news = ", ".join(f"NEW.c{i}" for i in range(k + 1))
        db.execute(f"CREATE TEMP TABLE chg_{rel}({cols}, PRIMARY KEY ({keys}))")
        for ev in ("INSERT", "UPDATE"):
            db.execute(f"CREATE TEMP TRIGGER chg_{rel}_{ev.lower()} AFTER {ev} ON {rel} BEGIN "
                       f"INSERT INTO chg_{rel} VALUES ({news}) ON CONFLICT({keys}) DO UPDATE SET {v} = excluded.{v}; END")
    db.execute(f"INSERT INTO {rel} SELECT * FROM ({select}) WHERE true ON CONFLICT({keys}) DO UPDATE SET {v} = {TOP} "
               f"WHERE {v} <> {TOP} AND {v} <> excluded.{v}")
    PROFILE["upsert"] += time.time() - t
    n = db.execute(f"SELECT count(*) FROM chg_{rel}").fetchone()[0]
    if delta:
        db.execute(f"INSERT INTO new_{rel} SELECT * FROM chg_{rel}")
    db.execute(f"DELETE FROM chg_{rel}")
    if os.environ.get("RDL_PROFILE"):
        PROFILE["rounds"] += 1
        print(f"    upsert {rel}: {n} changed", flush=True)
    return n

def semi_naive_round(prog, db, scc, planned, first, deltas=None):
    """One round: every rule over the full relations (`first`), else each delta variant of `deltas` ((head, order,
    index of the delta atom)), or of `planned` with its one order."""
    t = time.time()
    def lap(phase):
        nonlocal t
        now = time.time(); PROFILE[phase] += now - t; t = now
    for r in scc:
        db.execute(f"DROP TABLE IF EXISTS new_{r}")
        db.execute(f"CREATE TABLE new_{r} AS SELECT * FROM {r} WHERE 0")
    by_head = defaultdict(list)
    variants = [(h, order, [i]) for h, order, i in deltas] if deltas and not first else \
               [(h, order, [i for i, (l, _) in enumerate(order) if isinstance(l, Atom) and not l.neg and l.rel in scc]) for h, order in planned]
    for h, order, rec_idx in variants:
        if first:
            by_head[h.rel].append(sql_select(prog, h, order))
        else:
            for i in rec_idx:  # one variant per recursive atom, that atom reading the previous round's delta
                by_head[h.rel].append(sql_select(prog, h, order, delta_at=i))
        if os.environ.get("RDL_PROFILE") == "2":   # each body on its own, timed; the round then runs as usual
            for i in ([None] if first else rec_idx):
                tq = time.time()
                db.execute(f"SELECT count(*) FROM ({sql_select(prog, h, order, delta_at=i)})").fetchone()
                key = (repr(h) + " :- " + repr([l for l, _ in order]), "first" if first else f"delta@{repr(order[i][0])}")
                PROFILE_RULES[key] = PROFILE_RULES.get(key, 0.0) + time.time() - tq
                if time.time() - tq > 0.03 and (key not in PROFILE_EXPLAINED or time.time() - tq > 1):
                    PROFILE_EXPLAINED.add(key)
                    q = sql_select(prog, h, order, delta_at=i)
                    print(f"  slow body {time.time() - tq:.3f}s {key}\n    {q}")
                    for r in db.execute("EXPLAIN QUERY PLAN " + q): print("     ", r)
    t = time.time()
    total = 0
    for rel, selects in by_head.items():
        union = " UNION ALL ".join(f"SELECT * FROM ({s})" for s in selects)
        if rel in prog.lattice:
            total += merge_lattice(prog, db, rel, union, delta=True)
        else:
            lap("set: round setup")
            db.execute("DROP TABLE IF EXISTS cand")
            db.execute(f"CREATE TEMP TABLE cand AS {union}")
            lap("set: rule bodies")
            db.execute(f"INSERT INTO new_{rel} SELECT * FROM cand EXCEPT SELECT * FROM {rel}")
            n = db.execute(f"SELECT count(*) FROM new_{rel}").fetchone()[0]
            db.execute(f"INSERT OR IGNORE INTO {rel} SELECT * FROM new_{rel}")
            lap("set: dedup and insert")
            total += n
    for r in scc:
        if True:
            db.execute(f"DROP TABLE IF EXISTS delta_{r}")
            db.execute(f"ALTER TABLE new_{r} RENAME TO delta_{r}")
    lap("set: round setup")
    PROFILE["set: rounds"] += 1
    return total

# ---- Ascent backend ----------------------------------------------------------------------------------------------

RUST_TY = {"node": "u32", "int": "i64", "sym": "Sym", "flat": "Flat"}

def ascent_term(t, env, want=None):
    if isinstance(t, Var):
        v = t.n
        return v
    if isinstance(t, Const):
        if isinstance(t.v, str): return json.dumps(t.v)
        return f"{t.v}{'u32' if want == 'node' else 'i64' if want in ('int', None) else ''}" if want != "flat" else f"Flat({t.v})"
    if isinstance(t, TopT): return "Flat::TOP"
    if isinstance(t, Call):
        a = [ascent_term(x, env) for x in t.args]
        if t.f == "cat": return f"cat({a[0]}, {a[1]})"
        if t.f == "flat_bin": return f"flat_bin({a[0]}, {a[1]}, {a[2]})"
    raise ValueError(t)

def gen_ascent(prog, plan):
    """The program as one `ascent!` block. Each rule body is emitted in the planned order (Ascent iterates the first
    atom and probes indexes for the rest, so the order is the plan). `nearest` becomes a tape climb or descent."""
    stats = Stats.__new__(Stats)
    stats.kfreq, stats.nodes, stats.size = plan["kfreq"], plan["nodes"], plan["size"]
    stats.parents, stats.card, stats.steps_cache, stats.kindsets = plan["parents"], plan["card"], {}, prog.kindsets
    # Posting-weighted fanouts and constant frequencies if the plan has them (older plans do not: uniform NDV).
    # Document ranges are a SQL lowering only.
    stats.fan = {(k.rsplit("/", 1)[0], int(k.rsplit("/", 1)[1])): v for k, v in plan.get("fan", {}).items()}
    stats.memo = {("const", *json.loads(k)): v for k, v in plan.get("const", {}).items()}
    stats.indexes, stats.docs, stats.db = {}, None, None
    ndv = {(k.rsplit("/", 1)[0], int(k.rsplit("/", 1)[1])): v for k, v in plan["ndv"].items()}
    lines = ["ascent::ascent! {", "    pub struct Program;"]
    for rel, cols in prog.decls.items():
        kw = "lattice" if rel in prog.lattice else "relation"
        lines.append(f"    {kw} {rel}({', '.join(RUST_TY[t] for _, t in cols)});")
    for rel, rows in prog.facts.items():
        for row in rows:
            args = [lit(v, t) for v, (_, t) in zip(row, prog.decls[rel])]
            lines.append(f"    {rel}({', '.join(args)});")
    done = set()   # the magic-set rules share term objects with the rules they came from: rename each once
    def rename(t):
        if isinstance(t, Var) and id(t) not in done: done.add(id(t)); t.n = "v_" + t.n
        if isinstance(t, Call): [rename(a) for a in t.args]
    for h, body in prog.rules:   # Rust identifiers: `$fn` must not become the keyword `fn`
        [rename(a) for a in h.args]
        for l in body:
            if isinstance(l, Atom): [rename(a) for a in l.args]
            elif isinstance(l, Cmp): rename(l.l); rename(l.r)
            elif isinstance(l, Agg) and id(l) not in done: done.add(id(l)); l.var = "v_" + l.var; [rename(a) for a in l.atom.args]
            elif isinstance(l, Nearest) and id(l) not in done: done.add(id(l)); l.n, l.a = "v_" + l.n, "v_" + l.a
    for h, body in prog.rules:
        order, _, _ = plan_rule(prog, stats, h, body, ndv)
        types = {}
        parts = []
        for l, mode in order:
            if isinstance(l, Atom) and not l.neg:
                args = []
                for col, t in enumerate(l.args):
                    ty = prog.decls[l.rel][col][1]
                    if isinstance(t, Var):
                        if t.n in types: args.append(f"?{t.n}_eq") ; parts_eq = True
                        else: args.append(t.n); types[t.n] = ty
                    elif isinstance(t, Wild): args.append("_")
                    elif isinstance(t, TopT): args.append("?Flat::TOP")
                    else: args.append(lit(t.v, ty))
                # A variable repeated in an atom is a join in Ascent: reuse the name directly.
                args = [a[1:-3] if a.endswith("_eq") else a for a in args]
                parts.append(f"{l.rel}({', '.join(args)})")
            elif isinstance(l, Atom) and l.neg:
                args = []
                for col, t in enumerate(l.args):
                    ty = prog.decls[l.rel][col][1]
                    if isinstance(t, Var): args.append(t.n)
                    elif isinstance(t, Wild): args.append("_")
                    else: args.append(lit(t.v, ty))
                parts.append(f"!{l.rel}({', '.join(args)})")
            elif isinstance(l, Cmp):
                if l.op == "=" and isinstance(l.l, Var) and l.l.n not in types:
                    parts.append(f"let {l.l.n} = {expr(l.r, types)}"); types[l.l.n] = "int"
                else:
                    op = {"=": "=="}.get(l.op, l.op)
                    parts.append(f"if {expr(l.l, types)} {op} {expr(l.r, types, peer=l.l)}")
            elif isinstance(l, Agg):
                a = l.atom
                args = ", ".join("_" if isinstance(t, Wild) else t.n if isinstance(t, Var) else lit(t.v, prog.decls[a.rel][i][1]) for i, t in enumerate(a.args))
                parts.append(f"agg {l.var} = ascent::aggregators::count() in {a.rel}({args})"); types[l.var] = "count"
            elif isinstance(l, Nearest):
                ks = f"KS_{l.kset.upper()}"
                if mode == "climb":
                    if l.a in types: parts.append(f"if TAPE.nearest({val(l.n, types.get(l.n))}, {ks}) == Some({val(l.a, types.get(l.a))})")
                    else: parts.append(f"if let Some({l.a}) = TAPE.nearest({val(l.n, types.get(l.n))}, {ks})"); types[l.a] = "node_val"
                else:
                    if l.n in types: parts.append(f"if TAPE.nearest({val(l.n, types.get(l.n))}, {ks}) == Some({val(l.a, types.get(l.a))})")
                    else: parts.append(f"for {l.n} in TAPE.members({val(l.a, types.get(l.a))}, {ks})"); types[l.n] = "node_val"
        head = []
        for col, t in enumerate(h.args):
            ty = prog.decls[h.rel][col][1]
            if isinstance(t, Var):
                vt = types.get(t.n)
                v = val(t.n, vt)
                if ty == "flat" and vt == "int": v = f"Flat({v})"
                elif ty == "int" and vt == "flat": v = f"({v}).0"
                elif vt == "count": v = f"{t.n} as i64"
                head.append(v)
            elif isinstance(t, TopT): head.append("Flat::TOP")
            elif isinstance(t, Const): head.append(lit(t.v, ty))
            elif isinstance(t, Call): head.append(expr(t, types))
        lines.append(f"    {h.rel}({', '.join(head)}) <-- {', '.join(parts)};")
    lines.append("}")
    ks = [f"pub const KS_{n.upper()}: &[&str] = &[{', '.join(json.dumps(k) for k in v)}];" for n, v in prog.kindsets.items()]
    loader = ["pub fn load_inputs(p: &mut Program, dir: &str) {"]
    conv = {"node": "r[{i}].parse().unwrap()", "int": "r[{i}].parse().unwrap()", "sym": "intern(&r[{i}])"}
    for rel in prog.inputs:
        cols = ", ".join(conv[t].format(i=i) for i, (_, t) in enumerate(prog.decls[rel]))
        loader.append(f'    p.{rel} = tsv(dir, "{rel}").iter().map(|r| ({cols},)).collect();')
    loader.append("}")
    dump = ["pub fn dump_outputs(p: &Program, out: &str) -> Vec<(&'static str, usize)> {", "    let mut counts = Vec::new();"]
    for rel in prog.outputs:
        if rel == "finding": continue
        cols = " + \"\\t\" + &".join(f"r.{i}.to_string()" for i in range(len(prog.decls[rel])))
        dump.append(f'    let mut rows: Vec<String> = p.{rel}.iter().map(|r| {cols}).collect(); rows.sort();')
        dump.append(f'    std::fs::write(format!("{{out}}/{rel}.csv"), rows.iter().map(|r| r.clone() + "\\n").collect::<String>()).unwrap();')
        dump.append(f'    counts.push(("{rel}", rows.len()));')
    dump.append("    counts")
    dump.append("}")
    # `finding(rule, n)` is written by position; a program without findings has none.
    rows = "p.finding.iter().map(|(r, n)| (r.to_string(), *n)).collect()" if "finding" in prog.outputs else "Vec::new()"
    dump.append(f"pub fn finding_rows(p: &Program) -> Vec<(String, u32)> {{ let _ = p; {rows} }}")
    return "\n".join(ks + lines + loader + dump) + "\n"

def lit(v, ty):
    if isinstance(v, str): return json.dumps(v)
    return {"node": f"{v}u32", "int": f"{v}i64", "flat": f"Flat({v})"}.get(ty, str(v))

def val(name, vt):
    """A body variable's value: Ascent binds atom columns by reference; `let`, `if let` and `for` bind values."""
    return name if vt in ("count", "node_val", "int_val") else f"*{name}"

def expr(t, types, peer=None):
    if isinstance(t, Var): return val(t.n, types.get(t.n))
    if isinstance(t, TopT): return "Flat::TOP"
    if isinstance(t, Const):
        if isinstance(t.v, str): return json.dumps(t.v)
        if isinstance(peer, Var) and types.get(peer.n) == "count": return f"{t.v}usize"
        if isinstance(peer, Var) and types.get(peer.n) == "flat": return f"Flat({t.v})"
        return str(t.v)
    if isinstance(t, Call):
        a = [expr(x, types) for x in t.args]
        if t.f == "cat": return f"cat({a[0]}, {a[1]})"
        if t.f == "flat_bin": return f"flat_bin({a[0]}, {a[1]}, {a[2]})"
    raise ValueError(t)

def prune(prog):
    """Drops the rules of relations no output reads, directly or through other rules: what a program specialized to
    a query (its outputs and demand) leaves dead."""
    body_of = defaultdict(list)
    for h, b in prog.rules: body_of[h.rel].append(b)
    need, todo = set(), list(prog.outputs)
    while todo:
        r = todo.pop()
        if r in need: continue
        need.add(r)
        for b in body_of[r]:
            for l in b:
                a = l.atom if isinstance(l, Agg) else l
                if isinstance(a, Atom): todo.append(a.rel)
    prog.rules = [(h, b) for h, b in prog.rules if h.rel in need]
    return prog

if __name__ == "__main__":
    mode, src = sys.argv[1], sys.argv[2]
    prog = parse(open(src).read())
    if os.environ.get("RDL_NODEMAND"):
        prog.demand = []
    if os.environ.get("RDL_MAGIC") == "all" or "*" in prog.demand:
        prog = prune(magic_adorn(prune(prog)))
        print("magic sets:", {k: (len(v) if k in ("adornments", "full") else v) for k, v in prog.magic_report.items()}, flush=True)
    else:
        prog = prune(magic_sets(prune(prog)))
    if mode == "sql":
        run_sql(prog, sys.argv[3], sys.argv[4])
    elif mode == "ascent":
        plan = json.load(open(sys.argv[3]))
        open(sys.argv[4], "w").write(gen_ascent(prog, plan))
    elif mode == "check":
        sccs, _ = strata(prog)
        print(len(prog.decls), "relations,", len(prog.rules), "rules,", len(sccs), "components")
