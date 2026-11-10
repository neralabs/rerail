#!/usr/bin/env python3
"""Compare cold, warm, and edited whole jobs with ordinary fact persistence enabled."""
import argparse, hashlib, json, os, pathlib, re, resource, shutil, subprocess, time
p = argparse.ArgumentParser()
p.add_argument('--binary', type=pathlib.Path, required=True)
p.add_argument('--corpus', type=pathlib.Path, required=True)
p.add_argument('--out', type=pathlib.Path, required=True)
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
corpus = a.out / 'corpus'
if not corpus.exists(): shutil.copytree(a.corpus, corpus)
env = {k:v for k,v in os.environ.items() if not k.startswith('RERAIL_')}
env.update(RUST_MIN_STACK='67108864', RERAIL_THREADS='4', RERAIL_SHARED_CORPUS=str(corpus.resolve()), RERAIL_TAPE_CACHE=str((a.out/'facts.sqlite').resolve()))
results = []
for label in ['cold', 'warm', 'warm-profile', 'edit', 'edit-warm']:
    if label == 'edit':
        # Change bytes, preserving the rule findings. The edited file is private to this comparison.
        edited = next(p for p in sorted(corpus.rglob('*.py')) if p.stat().st_size > 0)
        with edited.open('ab') as f: f.write(b'\n# rerail fact admission edit\n')
    report = a.out / (label+'.json')
    e = env | {'RERAIL_RR_OUT': str((a.out/label).resolve()), 'RERAIL_RR_REPORT':str(report.resolve())}
    if label == 'warm-profile': e['RERAIL_PROFILE'] = str((a.out/label/'profile').resolve())
    command = ['nice','-n','19', str(a.binary.resolve()), 'complete_async_family_authored_rr_corpus_profile','--ignored','--nocapture','--test-threads=1']
    before = resource.getrusage(resource.RUSAGE_CHILDREN); start = time.monotonic()
    with (a.out/(label+'.log')).open('w') as f: run = subprocess.run(command, env=e, stdout=f, stderr=subprocess.STDOUT)
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    row = dict(label=label, exit=run.returncode, wall=time.monotonic()-start, cpu=after.ru_utime+after.ru_stime-before.ru_utime-before.ru_stime)
    if report.exists():
        r = json.loads(report.read_text()); row.update(files=r['report']['files'], keys=r['keys'], phases=r['report']['phases'], coreCpu=r['cpuNs']/1e9, crossKept=r['report'].get('crossStrata'))
    results.append(row); (a.out/'measurements.json').write_text(json.dumps(results, indent=2)+'\n')
    print(label, row['exit'], round(row['cpu'],3), round(row['wall'],3), flush=True)
    if run.returncode: raise SystemExit(run.returncode)
    assert row['files'] == 5725 and row['keys'] == [], row
(a.out/'manifest.json').write_text(json.dumps(dict(binary=str(a.binary), sha256=hashlib.sha256(a.binary.read_bytes()).hexdigest(), corpus=str(a.corpus), environment={k:v for k,v in env.items() if k.startswith('RERAIL_') or k == 'RUST_MIN_STACK'}), indent=2)+'\n')
