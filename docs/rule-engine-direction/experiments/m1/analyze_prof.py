"""Stage breakdown from a profiled run (code-rule-profile addon, RERAIL_CODE_RULE_PROFILE + RERAIL_PROFILE)."""
import json, sys, collections
d = sys.argv[1]
p = json.load(open(f'{d}/parity.json'))
lines = open(f'{d}/stderr.log').read().splitlines()
applies = []; trace = []; marks = {}
for l in lines:
    if l.startswith('RERAIL_CODE_RULE_PROFILE '): applies.append(json.loads(l.split(' ',1)[1]))
    elif l.startswith('RERAIL_TRACE '): trace.append(json.loads(l.split(' ',1)[1]))
    elif l.startswith('RERAIL_PROFILE_PHASE '):
        e = json.loads(l.split(' ',1)[1]); marks[e['phase']] = e['startUnixUs']
last_file_rev = max(a['revision'] for a in p['applies'] if a['phase']=='file-inputs')
glob_rev = last_file_rev + 1
keys = ['parseNs','regexNs','tapeEncodeNs','tapeDecodeRestoreNs','storageReadNs','storageWritePublicationNs','manifestCandidateLookupNs','anchorIndexNs','anchorSelectionNs','astMatchNs','serializationNs','otherNs','totalWorkerCpuNs']
ing = collections.Counter(); glob = collections.Counter()
for a in applies:
    tgt = ing if a['revision'] <= last_file_rev else (glob if a['revision']==glob_rev else None)
    if tgt is None: continue
    for k in keys: tgt[k] += (a.get(k) or 0)
rb = [e for e in trace if e['phase']=='relational-batch']
gstart = min(e['startUnixUs'] for e in rb) if rb else None
spans_g = collections.Counter(); spans_i = collections.Counter()
for e in trace:
    if e['phase'] in ('relational-batch',): continue
    t = spans_g if (gstart and e['startUnixUs'] >= gstart and e['startUnixUs'] < marks.get('unchanged', 1e30)) else (spans_i if e['startUnixUs'] < marks.get('unchanged',1e30) else None)
    if t is not None: t[e['phase']] += (e.get('cpuNs') or 0)
s = lambda c: {k: round(v/1e9,2) for k,v in sorted(c.items(), key=lambda x:-x[1]) if v/1e9 >= 0.05}
cpu = lambda k: round((p['phaseCpuMs'][k]['userMs']+p['phaseCpuMs'][k]['systemMs'])/1000,2)
out = {'run': d.rstrip('/').split('/')[-1], 'coldCpu': cpu('cold'),
       'ingestApplyCpu': round(sum(a['cpuMs'] for a in p['applies'] if a['phase']=='file-inputs')/1000,2),
       'globalApplyCpu': round(sum(a['cpuMs'] for a in p['applies'] if a['revision']==glob_rev)/1000,2),
       'ingestWorkerCounters': s(ing), 'globalWorkerCounters': s(glob),
       'relationalBatchCpu': round(sum(e.get('cpuNs') or 0 for e in rb)/1e9,2),
       'globalSpanCpu(nested,not additive)': s(spans_g), 'ingestSpanCpu(nested,not additive)': s(spans_i),
       'failures': p['failures']}
print(json.dumps(out, indent=1))

# Exclusive CPU per span phase within the global (relational) window, via same-thread interval nesting.
def exclusive(events):
    by_t = collections.defaultdict(list)
    for e in events: by_t[e['thread']].append(e)
    excl = collections.Counter()
    for t, evs in by_t.items():
        evs.sort(key=lambda e: (e['startUnixUs'], -e['wallNs']))
        stack = []
        for e in evs:
            end = e['startUnixUs'] + e['wallNs']/1000
            while stack and stack[-1][1] < e['startUnixUs'] + 0.5: stack.pop()
            if stack: excl[stack[-1][0]['phase']] -= (e.get('cpuNs') or 0)
            excl[e['phase']] += (e.get('cpuNs') or 0)
            stack.append((e, end))
    return excl
unch = marks.get('unchanged', 1e30)
gev = [e for e in trace if gstart and gstart <= e['startUnixUs'] < unch]
iev = [e for e in trace if (not gstart or e['startUnixUs'] < gstart) and e['startUnixUs'] >= marks.get('cold',0)]
eg = exclusive(gev); ei = exclusive(iev)
cat = lambda ph: 'sqlite' if ph.startswith('sql-') or ph.startswith('anchor-') and ph not in ('anchor-batch','anchor-fact-preparation') else ph
print(json.dumps({'globalExclusive': s(eg), 'ingestExclusive': s(ei),
  'globalSqlExclusiveCpu': round(sum(v for k,v in eg.items() if k.startswith('sql-'))/1e9,2),
  'globalNonSqlExclusiveCpu': round(sum(v for k,v in eg.items() if not k.startswith('sql-'))/1e9,2)}, indent=1))
