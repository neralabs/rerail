"""Summarize m1 runs: python3 summarize.py runs/<dir>..."""
import json, re, sys, os
def res(path):
    r = {}
    for line in open(path):
        m = re.fullmatch(r'(real|user|sys)\s+([\d.]+)', line.strip())
        if m: r[m[1]] = float(m[2])
        m = re.fullmatch(r'\s*(\d+)\s+(maximum resident set size|peak memory footprint)\s*', line)
        if m: r[m[2]] = int(m[1])
    return r
rows = []
for d in sys.argv[1:]:
    row = {'run': os.path.basename(d.rstrip('/'))}
    try:
        st = json.load(open(f'{d}/start.json')); en = json.load(open(f'{d}/end.json'))
        row['exit'] = en['exit']; row['loadStart'] = st['load']; row['loadEnd'] = en['load']
    except Exception as e: row['exit'] = 'running?'
    if os.path.exists(f'{d}/resource.txt'):
        r = res(f'{d}/resource.txt')
        row.update(procWall=r.get('real'), procCpu=round(r.get('user',0)+r.get('sys',0),2), procUser=r.get('user'), procSys=r.get('sys'),
                   peakRssGiB=round(r.get('maximum resident set size',0)/2**30,3), footprintGiB=round(r.get('peak memory footprint',0)/2**30,3))
    if os.path.exists(f'{d}/parity.json'):
        p = json.load(open(f'{d}/parity.json'))
        cpu = lambda k: round((p['phaseCpuMs'][k]['userMs']+p['phaseCpuMs'][k]['systemMs'])/1000,3) if k in p['phaseCpuMs'] else None
        ing = [a for a in p['applies'] if a['phase']=='file-inputs']
        glob = [a for a in p['applies'] if a['phase']=='global-results']
        row.update(expected=p['expected'], actual=p['actual'], fp=len(p['falsePositives']), fn=len(p['falseNegatives']), failures=len(p['failures']),
                   failureText=[str(f)[:200] for f in p['failures'][:3]],
                   compileS=round(p['compilationMs']/1000,3), compileCpu=cpu('compile'),
                   coldWall=round(p['coldMs']/1000,2), coldCpu=cpu('cold'),
                   ingestCpu=round(sum(a['cpuMs'] for a in ing)/1000,2), ingestWall=round(sum(a['wallMs'] for a in ing)/1000,2),
                   globalCpu=round(sum(a['cpuMs'] for a in glob[:1])/1000,3) if glob else None, globalWall=round(glob[0]['wallMs']/1000,3) if glob else None,
                   warmUnchangedWallMs=p['unchangedFileMs'] and round(p['unchangedFileMs'],1), warmUnchangedCpu=cpu('unchanged'),
                   warmEditWallMs=p['editedFileMs'] and round(p['editedFileMs'],1), warmEditCpu=cpu('edit'),
                   driverPeakRssGiB=round(p['peakRssBytes']/2**30,3), incrementalDiffs=len(p['incrementalDifferences']))
    rows.append(row)
print(json.dumps(rows, indent=1))
