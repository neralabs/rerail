#!/usr/bin/env python3
"""T13: attribute S8's warm edits in a samply profile (profile.json.gz + profile.json.syms.json).

usage: attribute.py RUN_DIR
Main test thread only (other threads' CPU in the same windows is printed separately). Arms are
segmented on the main thread: an arm starts where `buffered_root` (the directory snapshot) is
entered; arms go cold, one-file edit, whitespace edit per pass. Pass 0 is dropped. Weights are
threadCPUDelta (CPU, not wall). Categories are exclusive, first match wins.
"""
import bisect, collections, gzip, json, statistics, sys

run = sys.argv[1].rstrip('/') + '/'
p = json.load(gzip.open(run + 'profile.json.gz'))
s = json.load(open(run + 'profile.json.syms.json'))
strs = s['string_table']
symtab = {}
for x in s['data']:
    st = sorted(x['symbol_table'], key=lambda e: e['rva'])
    symtab[x['debug_name']] = ([e['rva'] for e in st], st)


def resolve(lib, addr):
    if lib not in symtab:
        return lib + '!?'
    rvas, st = symtab[lib]
    i = bisect.bisect_right(rvas, addr) - 1
    if i >= 0 and addr < st[i]['rva'] + st[i]['size']:
        return strs[st[i]['symbol']]
    return lib + '!?'


def frame_names(th):
    ft, fu, rt = th['frameTable'], th['funcTable'], th['resourceTable']
    out = []
    for fi in range(ft['length']):
        res = fu['resource'][ft['func'][fi]]
        lib = p['libs'][rt['lib'][res]]['debugName'] if res is not None and res >= 0 else '?'
        out.append(resolve(lib, ft['address'][fi]))
    return out


def stack_getter(th, names):
    pre, fr = th['stackTable']['prefix'], th['stackTable']['frame']
    cache = {}

    def get(si):
        if si is None:
            return ()
        chain, x = [], si
        while x is not None and x not in cache:
            chain.append(x)
            x = pre[x]
        base = cache[x] if x is not None else ()
        for y in reversed(chain):
            base = base + (names[fr[y]],)
            cache[y] = base
        return cache[si]

    return get


main = next(th for th in p['threads'] if th['name'].startswith('code_rule_dsl::slice::tests::s8'))
get = stack_getter(main, frame_names(main))
sm = main['samples']
cpu = [c or 0 for c in sm['threadCPUDelta']]
ROOT = 'rerail_engine::code_rule_dsl::facts::observe::roots::buffered_root'
RUN = 'rerail_engine::code_rule_dsl::slice::run'
CL = 'rerail_engine::code_rule_dsl::slice::tests::s8_measures_the_targets_on_s7s_sample::{{closure}}'

arms, cur, prev = [], None, None
for k, si in enumerate(sm['stack']):
    st = get(si)
    ph = 'root' if ROOT in st else ('run' if RUN in st else ('arm' if CL in st else 'other'))
    if ph == 'root' and prev != 'root':
        cur = {'start': sm['time'][k], 'idx': []}
        arms.append(cur)
    if ph != 'other' and cur is not None:
        cur['idx'].append(k)
        cur['end'] = sm['time'][k]
    prev = ph
warm = [a for j, a in enumerate(arms) if j % 3 != 0 and j >= 3]
cold = [a for j, a in enumerate(arms) if j % 3 == 0 and j >= 3]


def has(st, suffix):
    return any(f.endswith(suffix) for f in st)


CATS = [
    ('observer: directory snapshot (read + hash every file, store trees)', lambda st: ROOT in st),
    ('push: observe + propagate + the 4 derivations (overlay)', lambda st: has(st, 'Overlay::scoped') or has(st, 'observe::recover')),
    ('core facts rebuilt from tapes for every document (matches::documents)', lambda st: has(st, 'waves::matches::documents')),
    ('wave: family derivations written (record_derivations)', lambda st: has(st, 'schedule::run_over') and has(st, 'record_derivations')),
    ('wave: family digests (rows_digest)', lambda st: has(st, 'schedule::run_over') and has(st, 'rows_digest')),
    ('wave: evaluate the edited document (demand + SQL steps)', lambda st: has(st, 'schedule::run_over')),
    ('layer: family derivations written (record_derivations)', lambda st: has(st, 'schedule::layer') and has(st, 'record_derivations')),
    ('layer: SQL program over every document', lambda st: has(st, 'schedule::layer') or has(st, 'Layer::program')),
    ('layer: publish + setup', lambda st: has(st, 'Findings::publish') or (has(st, 'Layer::findings') and has(st, 'Connection::execute_batch') and not has(st, 'Layer::write::{{closure}}'))),
    ('layer: facts computed and inserted for every document (Layer::write)', lambda st: has(st, 'Layer::findings')),
    ('slice tables: keep / remember / statistics / memo', lambda st: any(has(st, x) for x in ('slice::keep', 'slice::remember', 'slice::kept_documents', 'Wave::statistics', 'memo::remember', 'memo::lookup', 'kept_program', 'memo::keep'))),
]


def categorise(arm):
    c = collections.Counter()
    for i in arm['idx']:
        st = get(sm['stack'][i])
        for name, pred in CATS:
            if pred(st):
                c[name] += cpu[i] / 1000
                break
        else:
            c['other'] += cpu[i] / 1000
    return c


rows = [categorise(a) for a in warm]
total = [sum(r.values()) for r in rows]
print(f'{run}: {len(arms)} arms, {len(warm)} warm arms kept (passes >= 1)')
print(f'main-thread CPU per warm arm, ms: min {min(total):.1f} median {statistics.median(total):.1f} max {max(total):.1f}')
others = 0
for th in p['threads']:
    if th is main:
        continue
    ts, cd = th['samples']['time'], th['samples']['threadCPUDelta']
    for a in warm:
        lo, hi = bisect.bisect_left(ts, a['start']), bisect.bisect_right(ts, a['end'])
        others += sum((cd[k] or 0) for k in range(lo, hi))
print(f'other threads CPU per warm arm, ms: {others / 1000 / len(warm):.1f}')
med_total = statistics.median(total)
print(f'{"stage":72s} {"median ms":>9s} {"share":>6s}  min-max')
names = [n for n, _ in CATS] + ['other']
for name in sorted(names, key=lambda n: -statistics.median([r[n] for r in rows])):
    vals = [r[name] for r in rows]
    med = statistics.median(vals)
    print(f'{name:72s} {med:9.1f} {100 * med / med_total:5.1f}%  {min(vals):.1f}-{max(vals):.1f}')
