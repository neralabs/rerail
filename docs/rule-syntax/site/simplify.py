"""Meaning-preserving clean-up of a translated rule, run before metavariable sharing.

translate.py lowers the JSON one construct at a time: every `any`, `stopBy`, `$option` and util becomes a
relation. That keeps the correspondence with the JSON, and it makes rules several times longer than they
need to be. This pass removes what the correspondence alone put there:

  1. merge: helper relations with the same rules (up to the names of their local variables) become one;
     repeated until nothing changes, so helpers that differed only in which duplicate they called merge too.
  2. inline: a generated helper (or a util whose name says nothing, such as `r12`) that is not recursive
     and is called once, as a whole positive literal, is replaced by its body: a conjunction for one rule,
     a disjunction for up to MAX_DISJ rules. Calls under `!` or inside `count : { … }` are left alone.
  3. repeats: a literal repeated in one body is kept once.
  4. bound: `node(_, x, _, _, _)` is dropped wherever x is bound otherwise; translate.py's grounding check
     puts it back where it is needed.
  5. families: relations alike up to their strings fold into one relation with a key and a table (fold.py).

Every step rewrites a Datalog program into an equivalent one: identical definitions are interchangeable,
and a non-recursive relation with one use can be unfolded at that use. Helper bodies here have only the
node column `n` (metavariable columns are added later, by share_metavariables), and capitals are ast-grep
metavariables, shared by design, so only lower-case locals are renamed when a body moves.
"""

import json
import re

MAX_DISJ = 4          # inline a multi-rule helper as a disjunction up to this many rules
MAX_INLINE_LEN = 400  # and only when the inlined text stays readable
GENERATED = re.compile(r"^(alt|stop|option|set)_\d+$")
MEANINGLESS = re.compile(r"^[a-z]{1,2}_?\d+$")   # utils named r12, c1746, m_3

# A string, or a backtick pattern fenced by n backticks (`` f(`a`) `` holds single backticks inside).
_PROT = re.compile(r'"(?:[^"\\]|\\.)*"|(?P<f>`+)[\s\S]*?(?<!`)(?P=f)(?!`)')
_CAP = re.compile(r"(?<![\w@])\$*[A-Z][A-Z0-9_]*\b")
_VAR = re.compile(r"(?<![\w@$])([a-z][a-z]*\d+|[a-z])\b(?!\s*\()")


def _split_protected(s):
    """Alternate pieces: even indices are code, odd ones string or backtick literals (left untouched)."""
    out, at = [], 0
    for m in _PROT.finditer(s):
        out += [s[at:m.start()], m.group(0)]
        at = m.end()
    return out + [s[at:]]


def _mentions_metavariable(lits):
    """A capital in code, or `$X` in a pattern: an ast-grep metavariable, shared across the whole match."""
    for l in lits:
        for i, p in enumerate(_split_protected(l)):
            if i % 2 == 0 and _CAP.search(p):
                return True
            if i % 2 and p.startswith("`") and re.search(r"\$+[A-Z_]", p):
                return True
    return False


def _code(s):
    return "".join(p for i, p in enumerate(_split_protected(s)) if i % 2 == 0)


def _rename(lit, mapping):
    parts = _split_protected(lit)
    for i in range(0, len(parts), 2):
        parts[i] = _VAR.sub(lambda m: mapping.get(m.group(1), m.group(1)), parts[i])
    return "".join(parts)


def _locals(lits):
    out = []
    for l in lits:
        for m in _VAR.finditer(_code(l)):
            v = m.group(1)
            if v not in out:
                out.append(v)
    return out


def _calls(lit):
    """Relation names a literal refers to: called, or passed as an argument (`precedes_stop(a, b, stop_9)`)."""
    code = _code(lit)
    return set(re.findall(r"(?<![\w.@$])([a-z_]\w*)\(", code)) | set(re.findall(r"(?<![\w.@$])([a-z]\w*_\d+|[a-z]{1,2}\d+)\b(?!\s*\()", code))


def _canon(heads_lits):
    """A definition with its local variables renamed in order of appearance: equal canon = same relation."""
    out = []
    for head_args, lits in heads_lits:
        names = {}
        for v in _locals([head_args] + lits):
            names.setdefault(v, f"_v{len(names)}")
        out.append((_rename(head_args, names), tuple(_rename(l, names) for l in lits)))
    return tuple(sorted(out))


class Rules:
    """All rules of one translated policy: violation rules (in `body` entries), utils and helpers."""

    def __init__(self, entries, helper_rules, facts, protected):
        self.entries = entries            # dicts with head, lits (and edits): order is the output order
        self.helper_rules = helper_rules  # [head, lits] lists, mutated in place
        self.facts = facts                # name -> values
        self.protected = protected        # relations that must keep their name (violations, param utils)

    def all(self):
        for e in self.entries:
            yield e
        for r in self.helper_rules:
            yield r

    @staticmethod
    def head(r):
        return r["head"] if isinstance(r, dict) else r[0]

    @staticmethod
    def lits(r):
        return r["lits"] if isinstance(r, dict) else r[1]

    @staticmethod
    def set_lits(r, lits):
        if isinstance(r, dict):
            r["lits"] = lits
        else:
            r[1] = lits

    def defs(self):
        d = {}
        for r in self.all():
            name, args = self.head(r).split("(", 1)
            d.setdefault(name, []).append(r)
        return d

    def rename_calls(self, mapping):
        """Rename relations. A generated name (alt_3) is renamed wherever it appears, as a call or as a relation
        argument; a short one (c5) only where it is called, because variables have names like that too."""
        if not mapping:
            return
        bare = [k for k in mapping if GENERATED.match(k)]
        alts = "|".join(map(re.escape, mapping))
        pat = re.compile(r"(?<![\w.@$])(" + alts + r")(?=\s*\()" + (r"|(?<![\w.@$])(" + "|".join(map(re.escape, bare)) + r")\b" if bare else ""))
        for r in self.all():
            new = []
            for l in self.lits(r):
                parts = _split_protected(l)
                for i in range(0, len(parts), 2):
                    parts[i] = pat.sub(lambda m: mapping[m.group(1) or m.group(2)], parts[i])
                new.append("".join(parts))
            self.set_lits(r, new)


def merge(rs, decls):
    """Identical definitions become one. Facts compare by their values."""
    changed = True
    merged = 0
    while changed:
        changed = False
        mapping = {}
        seen = {}
        for name, vals in sorted(rs.facts.items()):
            key = ("facts", json.dumps(vals, sort_keys=True), decls.get(name, "").split("(", 1)[-1])
            if key in seen:
                mapping[name] = seen[key]
            else:
                seen[key] = name
        defs = rs.defs()
        bare_used = _bare_names(rs, {n for n in defs if MEANINGLESS.match(n)})
        seen = {}
        for name in sorted(defs, key=lambda x: (not (x not in rs.protected and not GENERATED.match(x)), x)):
            if name in rs.protected or name in mapping:
                continue
            rules = defs[name]
            if not all(rs.head(r) == f"{name}(n)" for r in rules):
                continue
            key = _canon([("n", rs.lits(r)) for r in rules])
            if key in seen:
                if GENERATED.match(name) or (MEANINGLESS.match(name) and name not in bare_used):
                    mapping[name] = seen[key]
                continue
            seen[key] = name
        if mapping:
            changed = True
            merged += len(mapping)
            rs.rename_calls(mapping)
            drop = set(mapping)
            rs.helper_rules[:] = [r for r in rs.helper_rules if rs.head(r).split("(")[0] not in drop]
            rs.entries[:] = [e for e in rs.entries if rs.head(e).split("(")[0] not in drop]
            for n in drop:
                rs.facts.pop(n, None)
                decls.pop(n, None)
    return merged


def _bare_names(rs, names):
    """Names among `names` that appear somewhere not as a call: as a relation argument, or as a variable."""
    out = set()
    for r in rs.all():
        for l in rs.lits(r):
            out |= set(re.findall(r"(?<![\w.@$])([a-z]\w*)\b(?!\s*\()", _code(l))) & names
    return out


def _recursive(defs, rs):
    graph = {n: set().union(*[_calls(l) for r in rules for l in rs.lits(r)]) & set(defs) for n, rules in defs.items()}
    rec = set()
    for start in graph:
        stack, seen = list(graph[start]), set()
        while stack:
            x = stack.pop()
            if x == start:
                rec.add(start)
                break
            if x not in seen:
                seen.add(x)
                stack.extend(graph.get(x, ()))
    return rec


def inline(rs, decls):
    """Unfold helpers used once, in rounds. A round inlines every eligible helper whose one caller is not itself
    being inlined in the same round (that one waits for the next round, when its use has moved)."""
    inlined = 0
    while True:
        defs = rs.defs()
        rec = _recursive(defs, rs)
        uses = {}
        for r in rs.all():
            for i, l in enumerate(rs.lits(r)):
                for c in _calls(l):
                    if c in defs:
                        uses.setdefault(c, []).append((r, l))
        cands = {}
        for name, rules in defs.items():
            if name in rs.protected or name in rec or not (GENERATED.match(name) or MEANINGLESS.match(name)):
                continue
            if len(uses.get(name, [])) != 1 or len(rules) > MAX_DISJ or not all(rs.head(r) == f"{name}(n)" for r in rules):
                continue
            # A metavariable in the helper stays there. Inlined, it would meet the caller's metavariables of the
            # same name (two `any` branches that each bind $X separately would have to agree), and the `unset(X)`
            # translate.py adds where X is unbound would apply to the whole rule, not to one branch.
            if any(_mentions_metavariable(rs.lits(r)) for r in rules):
                continue
            r, lit = uses[name][0]
            m = re.fullmatch(rf"{re.escape(name)}\(([a-zA-Z_]\w*)\)", lit.strip())
            if m:
                cands[name] = (r, m.group(1), rules)
        gone = set()
        for name, (r, arg, rules) in cands.items():
            if rs.head(r).split("(")[0] in cands:
                continue
            caller = rs.lits(r)
            lit = f"{name}({arg})"
            idx = [i for i, l in enumerate(caller) if l.strip() == lit]
            if len(idx) != 1:
                continue
            i = idx[0]
            taken = set(_locals([rs.head(r)] + caller)) | {arg}
            bodies = []
            for hr in rules:
                body = rs.lits(hr)
                mapping = {"n": arg}
                for v in _locals(body):
                    if v == "n":
                        continue
                    base = re.sub(r"\d+$", "", v) or "x"
                    k = 1
                    while f"{base}{k}" in taken:
                        k += 1
                    mapping[v] = f"{base}{k}"
                    taken.add(mapping[v])
                bodies.append([_rename(l, mapping) for l in body])
            if len(bodies) == 1:
                new = bodies[0]
            else:
                text = "(" + " ; ".join(", ".join(b) for b in bodies) + ")"
                if len(text) > MAX_INLINE_LEN:
                    continue
                new = [text]
            rs.set_lits(r, caller[:i] + new + caller[i + 1:])
            gone.add(name)
        if not gone:
            return inlined
        rs.helper_rules[:] = [h for h in rs.helper_rules if rs.head(h).split("(")[0] not in gone]
        rs.entries[:] = [e for e in rs.entries if rs.head(e).split("(")[0] not in gone]
        for name in gone:
            decls.pop(name, None)
        inlined += len(gone)


def drop_node_atoms(rs):
    """`node(_, x, _, _, _)` only grounds x; translate.py re-adds it where nothing else does."""
    pat = re.compile(r"^node\(_, ([a-z]\w*), _, _, _\)$")
    for r in rs.all():
        lits = rs.lits(r)
        keep = []
        for j, l in enumerate(lits):
            m = pat.match(l.strip())
            if m:
                others = " ".join(_code(x) for k, x in enumerate(lits) if k != j)
                if re.search(rf"(?<![\w@$]){m.group(1)}\b", others):
                    continue
            keep.append(l)
        if keep != lits:
            rs.set_lits(r, keep or lits)


def dedup_literals(rs):
    """A conjunction does not change when a literal repeats: keep the first of each."""
    for r in rs.all():
        lits = rs.lits(r)
        seen, keep = set(), []
        for l in lits:
            if l.strip() not in seen:
                seen.add(l.strip())
                keep.append(l)
        if len(keep) != len(lits):
            rs.set_lits(r, keep)


def unused_facts(rs, decls):
    called = set()
    for r in rs.all():
        for l in rs.lits(r):
            called |= _calls(l)
    for n in list(rs.facts):
        if n not in called:
            rs.facts.pop(n)
            decls.pop(n, None)


def simplify(entries, prog, protected):
    """Simplify in place. `entries` are translate.py's body rule dicts; prog.rules holds helpers and facts."""
    helper_rules = [list(r) for r in prog.rules if r[0] != "#facts"]
    facts = {r[1]: r[2] for r in prog.rules if r[0] == "#facts"}
    rs = Rules(entries, helper_rules, facts, protected)
    stats = {"merged": 0, "inlined": 0}
    from fold import fold  # fold.py imports helpers from here
    for rnd in range(10):
        a = merge(rs, prog.decls)
        b = inline(rs, prog.decls)
        stats["merged"] += a
        stats["inlined"] += b
        if not a and not b:
            # 5. families: copies alike up to their strings become one relation and a table (fold.py), once.
            if rnd < 9 and "folded" not in stats:
                stats["folded"] = fold(rs, prog.decls, prog.notes)
                if stats["folded"]:
                    continue
            break
    dedup_literals(rs)
    drop_node_atoms(rs)
    unused_facts(rs, prog.decls)
    prog.rules = [("#facts", n, v) for n, v in rs.facts.items()] + [tuple(r) for r in rs.helper_rules]
    return stats
