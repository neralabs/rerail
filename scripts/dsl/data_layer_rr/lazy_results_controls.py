"""Independent original and reporter positives must fail when their native method arm is disabled."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys

from migration import ROOT, read_json, sha, write_json


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['source', 'engine', 'baseline', 'reporter-oracle', 'out']:
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--method', choices=['select_one', 'execute'], required=True)
    args = parser.parse_args()
    for name in ['source', 'engine', 'baseline', 'reporter_oracle', 'out']:
        setattr(args, name, getattr(args, name).absolute())
    base = args.out.parent/('control-'+args.method)
    mutant = base/'disabled'
    mutant.mkdir(parents=True, exist_ok=True)
    source = (ROOT/'gate/data-layer/packs/data-layer/candidates/rr/lazy-result-usage.rr').read_text()
    disabled, count = re.subn(r'(?m)^(lru_eligible\(f, cls, method, text\)\s*:-)',
        lambda m: m[0]+' method != "'+args.method+'",', source)
    assert count == 1
    (mutant/'lazy-result-usage.rr').write_text(disabled)
    baseline = read_json(args.baseline)
    i = 0 if args.method == 'select_one' else 1
    assert ['test_flags_concrete_transaction_read_return', 'test_flags_concrete_query_execution_return'][i] in baseline['collected'][i]
    def run(name, script, arguments, red=False):
        path = base/(name+'.json')
        command = [sys.executable, str(Path(__file__).with_name(script)), 'native', *arguments, '--out', str(path)]
        if red:
            command += ['--rule-root', str(mutant)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=100,
            env={**os.environ, 'PYTHONHASHSEED':'0'})
        (base/(name+'.log')).write_text(result.stdout+result.stderr)
        return dict(exitCode=result.returncode, command=command, report=read_json(path))
    arguments = ['--source', str(args.source), '--engine', str(args.engine), '--baseline', str(args.baseline),
        '--part', str(i), '--parts', str(len(baseline['collected']))]
    green = run('original-green', 'lazy_results_contract.py', arguments)
    red = run('original-red', 'lazy_results_contract.py', arguments, True)
    assert (green['exitCode'], red['exitCode']) == (0, 1)
    assert red['report']['collected'] == baseline['collected'] and not red['report']['semanticFallbacks']
    assert red['report']['calls'][0]['return'] == [] and green['report']['calls'][0]['return']
    assert all(call['status']=='passed' for call in red['report']['nativeCalls'])
    assert all({p['phase']:p['outcome'] for p in phases} == dict(setup='passed',call='failed',teardown='passed')
               for phases in red['report']['cases'].values())
    cases = ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-lazy-result-usage-reporter-cases.json'
    rows = read_json(cases)['cases']
    i = next(i for i,r in enumerate(rows) if r['id']=='method-'+args.method)
    arguments = ['--source',str(args.source),'--engine',str(args.engine),'--oracle',str(args.reporter_oracle),
        '--cases',str(cases),'--part',str(i),'--parts',str(len(rows))]
    rg = run('reporter-green','reporting.py',arguments)
    rr = run('reporter-red','reporting.py',arguments,True)
    assert (rg['exitCode'],rr['exitCode']) == (0,1)
    assert rg['report']['results'][0]['value'] and rr['report']['results'][0]['value']==[]
    write_json(args.out,dict(engineSha256=sha(args.engine.read_bytes()),method=args.method,
        disabledPolicy=disabled,disabledPolicySha256=sha(disabled.encode()),
        originalGreen=green,originalRed=red,reporterGreen=rg,reporterRed=rr,
        fullMigrationAcceptance=False,script=Path(__file__).read_text()))


if __name__ == '__main__':
    main()
