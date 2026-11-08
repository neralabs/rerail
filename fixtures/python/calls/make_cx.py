"""Writes calls_cx.rr from calls.rr (decision 0076, O114): usage `make_cx.py calls.rr calls_cx.rr`, run in
native/src/code_rule_dsl/typing/programs. Each rewrite names the text it replaces and fails when calls.rr no longer has
it."""
import re, sys
src, dst = sys.argv[1], sys.argv[2]
s = open(src).read()
R = "docof({a}, d9), docof({b}, e9), rr_cx_reach(d9, e9)"
def rep(old, new, count=1):
    global s
    n = s.count(old)
    if n != count:
        sys.exit(f"expected {count} of {old!r}, found {n}")
    s = s.replace(old, new)

rep('#include "python/conditions.rr"\n', '''#include "python/conditions.rr"
// The import closure, components.rr's (the engine adds it to a program that reads it): rr_cx_reach(d, e), e is in
// d's closure; rr_cx_sees(d, q), q is a module of d's closure.
''')

# Closed-world subclasses: those in the reader's closure (`anc` reversed), not a class's importers.
rep('.decl sub(c: node, s: node)\nsub(c, c) :- kind(c, "class_definition").\nsub(c, s) :- sub(c, m), base(s, m).\n', '')
lines = s.split('\n')
out = []
for l in lines:
    m = re.match(r'^(\w+)\((\w+),(.*)\bsub\((\w+), s\)\.$', l)
    if m:
        head, h, k = m.group(1), m.group(2), m.group(4)
        l = l[:l.rindex('sub(')] + f"anc(s, {k}), " + R.format(a=h, b='s') + '.'
    out.append(l)
s = '\n'.join(out)
assert 'sub(' not in re.sub(r'\w+sub\(', '', s), [l for l in s.split('\n') if re.search(r'(?<!\w)sub\(', l)]

# Demand: lookups keyed by the node that asks, not by the class asked.
rep('''.decl need_attr(c: node, a: symbol)
need_attr(c, a) :- alook(_, c, a).
need_attr(c, "__init__") :- callee(k, e), pv(e, "cls", c).
need_attr(c, "__call__") :- callee(k, e), pv(e, "inst", c).
''', '''.decl asks(n: node, c: node, a: symbol)
asks(n, c, a) :- alook(n, c, a).
asks(k, c, "__init__") :- callee(k, e), pv(e, "cls", c).
asks(k, c, "__call__") :- callee(k, e), pv(e, "inst", c).
''')
rep('need_attr(k, m) :- pcall(_, o, m), pv(o, "inst", k).', 'asks(c, k, m) :- pcall(c, o, m), pv(o, "inst", k).')
rep('need_attr(k, a) :- icall(_, o, a), pv(o, "inst", k).', 'asks(c, k, a) :- icall(c, o, a), pv(o, "inst", k).')
rep('''.decl need_name(a: symbol)
need_name(a) :- need_attr(_, a).
need_name("__init__"). need_name("__call__"). need_name("__new__").
''', '')
rep('found(c, a, c) :- bound_in(c, a), need_name(a).', 'found(c, a, c) :- bound_in(c, a).')
rep('.pragma "magic-transform" "found"\n', '')
rep('cread(c, x) :- cann(c, x, _), need_name(x).', 'cread(c, x) :- cann(c, x, _).')
rep('adeclared(c, a) :- need_attr(c, a), found(c, a, at), cann(at, a, t), !aopen(t).', 'adeclared(c, a) :- found(c, a, at), cann(at, a, t), !aopen(t).')
rep('.decl fv(c: node, a: symbol, k: symbol, v: node)', '.decl fv(n: node, c: node, a: symbol, k: symbol, v: node)')
rep('fv(c, a, k, v) :- need_attr(c, a), found(c, a, at), bv(at, a, k, v).', 'fv(n, c, a, k, v) :- asks(n, c, a), found(c, a, at), bv(at, a, k, v).')
rep('pfn(c, o, f) :- pcall(c, o, m), pv(o, "inst", k), fv(k, m, "fn", f).', 'pfn(c, o, f) :- pcall(c, o, m), pv(o, "inst", k), fv(c, k, m, "fn", f).')
rep('ifn(c, f) :- icall(c, o, a), pv(o, "inst", k), fv(k, a, "fn", f).', 'ifn(c, f) :- icall(c, o, a), pv(o, "inst", k), fv(c, k, a, "fn", f).')
rep('pv(n, k, v) :- alook(n, c, a), fv(c, a, k, v).', 'pv(n, k, v) :- alook(n, c, a), fv(n, c, a, k, v).')
rep('.decl attr_open(c: node, a: symbol)', '.decl attr_open(n: node, c: node, a: symbol)')
s = re.sub(r'^attr_open\(c, a\) :- need_attr\(c, a\), ', 'attr_open(n, c, a) :- asks(n, c, a), ', s, flags=re.M)
assert not [l for l in s.split('\n') if 'need_' in l and not l.startswith('//')], [l for l in s.split('\n') if 'need_' in l]
rep('vopen(n) :- vpos(n), alook(n, c, a), attr_open(c, a).', 'vopen(n) :- vpos(n), alook(n, c, a), attr_open(n, c, a).')
rep('vopen(c) :- vpos(c), icall(c, o, a), pv(o, "inst", k), attr_open(k, a).', 'vopen(c) :- vpos(c), icall(c, o, a), pv(o, "inst", k), attr_open(c, k, a).')
rep('attr_open(k, "__init__").', 'attr_open(c, k, "__init__").')
rep('attr_open(k, "__call__").', 'attr_open(c, k, "__call__").')

# A module's TypeVars, keyed by its module node.
rep('.decl mtvar(d: number, x: symbol)\nmtvar(d, x) :- modnode(d, m), ', '.decl mtvar(m: node, x: symbol)\nmtvar(m, x) :- modnode(_, m), ')
rep('docof(g, d), mtvar(d, v).', 'docof(g, d), modnode(d, mm), mtvar(mm, v).', 2)

# A module's bindings, keyed by the module; a reader sees those of its closure.
rep('.decl qv(q: symbol, k: symbol, v: node)', '.decl qv(m: node, q: symbol, k: symbol, v: node)')
rep('qv(q, "mod", m) :- modname(m, q).', 'qv(m, q, "mod", m) :- modname(m, q).')
rep('qv(@dot(q, x), k, v) :- modname(m, q), bv(m, x, k, v).', 'qv(m, @dot(q, x), k, v) :- modname(m, q), bv(m, x, k, v).')
rep('.decl isqmod(q: symbol)\nisqmod(q) :- modname(_, q).\n', '')
rep('bv(s, x, k, v) :- ibind(s, x, q), qv(q, k, v).', 'bv(s, x, k, v) :- ibind(s, x, q), qv(m9, q, k, v), ' + R.format(a='s', b='m9') + '.')
rep('pv(n, k, v) :- qn(n, q), qv(q, k, v).', 'pv(n, k, v) :- qn(n, q), qv(m9, q, k, v), ' + R.format(a='n', b='m9') + '.')
rep('pv(n, k, v) :- bq(n, q), qv(q, k, v).', 'pv(n, k, v) :- bq(n, q), qv(m9, q, k, v), ' + R.format(a='n', b='m9') + '.')
rep('qv("builtins.object", "cls", c).', 'qv(m9, "builtins.object", "cls", c), ' + R.format(a='o', b='m9') + '.')
rep('pv(n, "inst", c) :- litq(n, q), qv(q, "cls", c).', 'pv(n, "inst", c) :- litq(n, q), qv(m9, q, "cls", c), ' + R.format(a='n', b='m9') + '.')
rep('.decl qknown(q: symbol)\nqknown(q) :- qv(q, _, _).\n', '')
rep('.decl qopen(q: symbol)\nqopen(@dot(q, x)) :- modname(m, q), bopen(m, x).', '.decl qopen(m: node, q: symbol)\nqopen(m, @dot(q, x)) :- modname(m, q), bopen(m, x).')
rep('bopen(s, x) :- ifrom(s, x, m, _), isqmod(m), ibind(s, x, q), qopen(q).',
    'bopen(s, x) :- ifrom(s, x, m, _), docof(s, d9), rr_cx_sees(d9, m), ibind(s, x, q), qopen(m9, q), docof(m9, e9), rr_cx_reach(d9, e9).')
rep('bopen(s, x) :- ifrom(s, x, m, _), isqmod(m), ibind(s, x, q), !qknown(q).',
    'bopen(s, x) :- ifrom(s, x, m, _), docof(s, d9), rr_cx_sees(d9, m), ibind(s, x, q), !bknown(s, q).\n'
    '.decl bknown(s: node, q: symbol)\nbknown(s, q) :- ibind(s, _, q), qv(m9, q, _, _), ' + R.format(a='s', b='m9') + '.')
rep('vopen(n) :- vpos(n), qn(n, q), qopen(q).', 'vopen(n) :- vpos(n), qn(n, q), qopen(m9, q), ' + R.format(a='n', b='m9') + '.')
rep('vopen(n) :- vpos(n), attr_of(n, o, _), qn(o, q), isqmod(q), qn(n, q2), !qknown(q2).',
    'vopen(n) :- vpos(n), attr_of(n, o, _), qn(o, q), docof(n, d9), rr_cx_sees(d9, q), qn(n, q2), !vknown(n, q2).\n'
    '.decl vknown(n: node, q: symbol)\nvknown(n, q) :- vpos(n), attr_of(n, _, _), qn(n, q), qv(m9, q, _, _), ' + R.format(a='n', b='m9') + '.')
rep('extb(e) :- basex(_, e), qn(e, q), !qknown(q).',
    'extb(e) :- basex(_, e), qn(e, q), !xknown(e, q).\n'
    '.decl xknown(e: node, q: symbol)\nxknown(e, q) :- basex(_, e), qn(e, q), qv(m9, q, _, _), ' + R.format(a='e', b='m9') + '.')
rep('ext_import(s, x) :- rifrom(s, x, m, _), !isqmod(m).', 'ext_import(s, x) :- rifrom(s, x, m, _), docof(s, d9), !rr_cx_sees(d9, m).')
rep('ext_import(s, x) :- ribind(s, x, q), !rifrom(s, x, _, _), !isqmod(q).', 'ext_import(s, x) :- ribind(s, x, q), !rifrom(s, x, _, _), docof(s, d9), !rr_cx_sees(d9, q).')
rep('.decl tqv(q: symbol, k: symbol, v: node)', '.decl tqv(m: node, q: symbol, k: symbol, v: node)')
rep('tqv(@dot(q, x), k, v) :- modname(m, q), tbv(m, x, k, v).', 'tqv(m, @dot(q, x), k, v) :- modname(m, q), tbv(m, x, k, v).')
rep('tbv(s, x, k, v) :- ibind(s, x, q), tqv(q, k, v).', 'tbv(s, x, k, v) :- ibind(s, x, q), tqv(m9, q, k, v), ' + R.format(a='s', b='m9') + '.')
rep('tv(n, k, v) :- qn(n, q), tqv(q, k, v).', 'tv(n, k, v) :- qn(n, q), tqv(m9, q, k, v), ' + R.format(a='n', b='m9') + '.')
for bad in ['isqmod', 'qv(q', 'qopen(q)']:
    assert not [l for l in s.split('\n') if bad in l and not l.startswith('//')], (bad, [l for l in s.split('\n') if bad in l])

# Subtyping between a node's types, per node: the classes a name names in the node's closure.
C = 'type_of(n, t), cls_q(c, t), ' + R.format(a='n', b='c') + ', anc(c, b)'
rep('''.decl tname(t: symbol)
tname(t) :- type_of(_, t).
.decl tsub(t: symbol, u: symbol)
tsub(t, t) :- tname(t).
tsub(t, "object") :- tname(t).
tsub("bool", "int") :- tname("bool").
tsub(t, u) :- tname(t), cls_q(c, t), anc(c, b), cls_q(b, u).
tsub(t, u) :- tname(t), cls_q(c, t), anc(c, b), basex(b, e), qn(e, u), !qknown(u).
tsub(t, x) :- tname(t), cls_q(c, t), anc(c, b), basex(b, e), builtin_ref(e, x).''' if False else '''.decl tname(t: symbol)
tname(t) :- type_of(_, t).
.decl tsub(t: symbol, u: symbol)
tsub(t, t) :- tname(t).
tsub(t, "object") :- tname(t).
tsub("bool", "int") :- tname("bool").
tsub(t, u) :- tname(t), cls_q(c, t), anc(c, b), cls_q(b, u).
''', f'''.decl tsub(n: node, t: symbol, u: symbol)
tsub(n, t, t) :- type_of(n, t).
tsub(n, t, "object") :- type_of(n, t).
tsub(n, "bool", "int") :- type_of(n, "bool").
tsub(n, t, u) :- {C}, cls_q(b, u).
''')
rep('tsub(t, u) :- tname(t), cls_q(c, t), anc(c, b), basex(b, e), qn(e, u), !qknown(u).', f'tsub(n, t, u) :- {C}, basex(b, e), qn(e, u), !xknown(e, u).')
rep('tsub(t, x) :- tname(t), cls_q(c, t), anc(c, b), basex(b, e), builtin_ref(e, x).', f'tsub(n, t, x) :- {C}, basex(b, e), builtin_ref(e, x).')
rep('.decl tsup_open(t: symbol)', '.decl tsup_open(n: node, t: symbol)')
rep('tsup_open(t) :- tname(t), cls_q(c, t), anc(c, b), base_open(b).', f'tsup_open(n, t) :- {C}, base_open(b).')
rep('tsup_open(t) :- tname(t), cls_q(c, t), anc(c, b), basex(b, e), extb(e), !builtin_ref(e, "object").', f'tsup_open(n, t) :- {C}, basex(b, e), extb(e), !builtin_ref(e, "object").')
rep('type_miss(n, u) :- type_first(n, t0), tsub(t0, u), type_of(n, t), !tsub(t, u).', 'type_miss(n, u) :- type_first(n, t0), tsub(n, t0, u), type_of(n, t), !tsub(n, t, u).')
rep('proven_type(n, u) :- type_first(n, t), tsub(t, u),', 'proven_type(n, u) :- type_first(n, t), tsub(n, t, u),')
rep('possible_type(n, u) :- type_of(n, t), tsub(t, u).', 'possible_type(n, u) :- type_of(n, t), tsub(n, t, u).')
rep('possible_open(n) :- type_of(n, t), tsup_open(t).', 'possible_open(n) :- type_of(n, t), tsup_open(n, t).')
assert not [l for l in s.split('\n') if 'tname' in l and not l.startswith('//')]

assert not [l for l in s.split('\n') if 'qknown' in l and not l.startswith('//')]
hdr = '''// calls_cx.rr: calls.rr with every cross-file lookup a function of the reader's import closure (decision 0076, O114),
// so its fixpoint can be memoised per import component. Generated from calls.rr by
// fixtures/python/calls/make_cx.py; edit calls.rr and regenerate. What differs:
// - a module's bindings (`qv`, `tqv`, `qopen`) are keyed by the module, and a reader sees those of its closure
//   (`rr_cx_reach`); "is q a module" is `rr_cx_sees`;
// - attribute lookups are keyed by the node that asks (`asks`, `fv`, `attr_open`), not by the class asked, and
//   `found` is not cut to the names some reader asks for;
// - closed-world `self` and annotations reach the subclasses in the reader's closure, not every importer's;
// - subtyping (`tsub`, `tsup_open`) is per node, over the classes its closure names.
'''
open(dst, 'w').write(hdr + s)
print("ok")
