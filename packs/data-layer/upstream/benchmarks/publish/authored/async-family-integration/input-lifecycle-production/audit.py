"""Audit completed input-lifecycle measurements; no benchmark executions."""
import hashlib, json, pathlib, re, statistics
root = pathlib.Path(__file__).resolve().parent
workspace = pathlib.Path('/Users/jeremy/orca/workspaces/rerail/Async-family-integration-comparison')
def read(name): return json.loads((root/name).read_text())
def write(name, value): (root/name).write_text(json.dumps(value, indent=2)+'\n')
# The only change after the measured build adds timing output to the containment test.
for path, digest in read('source-sha256.json').items():
    if path.endswith('/syntax/tests.rs'): continue
    assert hashlib.sha256((workspace/path).read_bytes()).hexdigest() == digest, path
runs = read('runs.json')
for run in runs:
    data = read(run['arm']+'-'+run['label']+'.json')
    assert data['keyCount'] == len(data['keys'])
    assert run['sql'] <= 100
    if not run['label'].startswith('edge-'):
        assert data['report']['files'] == 5725
        assert data['report']['nodes'] == 19007630
        assert data['keys'] == []
        assert run['sql'] == 81
assert sum(r['keys'] for r in runs if r['label'].startswith('edge-')) == 178
profiles = {}
for arm in ['baseline','production']:
    data = read(arm+'-profile.json'); spans = data['profile']['spans']
    assert sum(s['vmSteps'] for s in data['profile']['statements']) == 785573
    phases = {}
    for suffix in ['tape restore', 'facts from tape', 'validated tape columns', 'kind postings', 'identity maps', 'token spans', 'projection lines', 'projection depths', 'projection subtree sizes', 'projection ordinals', 'projection fields', 'project and intern']:
        matches = [s for s in spans if s['name'].endswith(suffix)]
        phases[suffix] = {'cpu':sum(s['cpuSeconds'] for s in matches), 'calls':sum(s['calls'] for s in matches)}
    payload = {k:sum(json.loads(d['text'])[k]*d['count'] for d in data['profile']['decisions'] if d['topic']=='input payload') for k in ['coreColumnBytes','identityCapacityBytes','extraSpanCapacityBytes']}
    profiles[arm] = {'phases':phases,'cumulativeAtPublication':payload,'vm':785573,'sql':81}
write('profile-costs.json',profiles)
tests = []
for path in sorted(list(root.glob('test-*.log'))+list(root.glob('definition-*.log'))):
    text = path.read_text(); result = re.search(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',text)
    assert result and int(result[1]) > 0 and int(result[2]) == 0, path
    tests.append({'log':path.name,'passed':int(result[1]),'ignored':int(result[3])})
assert sum(t['passed'] for t in tests) == 107
axes = {}
for way in ['baseline','tape']:
    for arm in ['baseline','production']:
        name = 'axes-'+way+'-'+arm
        log = (root/(name+'.log')).read_text()
        m = re.search(r'MEASURE .*?: (\d+) files, (\d+) nodes, (\d+) chunks, hit rows (\d+), CPU ([\d.]+)',log)
        assert m and (int(m[1]),int(m[2]),int(m[3])) == (100,3791229,17)
        assert int(m[4]) == (140230 if way=='tape' else 145408)
        local = re.search(r'  local\s+([\d.]+) s CPU',log)
        axes[name] = {'files':int(m[1]),'nodes':int(m[2]),'hits':int(m[4]),'phaseCpu':float(m[5]),'localCpu':float(local[1])}
write('axes-summary.json',axes)
uncached=read('production-uncached.json')
assert uncached['report']['files'] == 5725 and uncached['report']['nodes'] == 19007630 and uncached['keys'] == []
ss=uncached['profile']['spans']
assert sum(s['calls'] for s in ss if s['name'].endswith('validated tape columns') and 'tree-sitter and facts' in s['name']) == 5725
assert not any(s['name'].endswith('tape restore') and 'tree-sitter and facts' in s['name'] for s in ss)
write('validation.json',{'tests':tests,'focusedTestExecutions':107,'edgeFindings':178,'corpusFiles':5725,'corpusNodes':19007630,'fullCorpusRuns':len([r for r in runs if not r['label'].startswith('edge-')]),'uncachedCpu':uncached['cpuNs']/1e9,'uncachedWall':uncached['wallNs']/1e9,'uncachedDirectAdmissions':5725})
print('PASS: production hashes, 107 focused tests, 178 edge findings, paired corpus reports, SQL/VM counts, axis rows, uncached admissions')
print(json.dumps(profiles,indent=2))

phases=read('containment-phase.json')
assert len(phases)==2 and all(p['nodes']==8416 and p['referenceCpu']>0 and p['relationAndComparisonsCpu']>0 for p in phases)
for path,digest in read('final-source-sha256.json').items():
    assert hashlib.sha256((workspace/path).read_bytes()).hexdigest()==digest,path
print('PASS: final test source hashes and two containment phase checks')
