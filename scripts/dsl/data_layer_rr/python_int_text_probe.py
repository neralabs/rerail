"""Check the native decimal text functor through both execution APIs."""
import os
from pathlib import Path
import sys
import tempfile
from migration import ROOT,MANIFEST,read_json,write_json,sha
from replay import execute_native,execute_workspace


def run(out):
    engine=ROOT/read_json(ROOT/MANIFEST)['engineSource']['cliArtifact']['path']
    policy=ROOT/'test/fixtures/rr/python-int-text.rr'
    expected=[['large','1267650600228229401496703205376'],['unicode','12']]
    report=dict(engineSha256=sha(engine.read_bytes()),cases=[],passed=False)
    with tempfile.TemporaryDirectory(prefix='python-int-text-') as temporary:
        os.environ['RERAIL_TAPE_CACHE']=str(Path(temporary)/'cache.sqlite')
        for mode,execute in [('snapshot',execute_native),('workspace',execute_workspace)]:
            native=execute(engine,policy,[],{})
            actual=sorted([[r['parameters']['id'],r['parameters']['value']] for r in native['diagnostics']])
            report['cases'].append(dict(mode=mode,native=native,expected=expected,actual=actual,passed=actual==expected))
            write_json(out,report)
    report['passed']=all(r['passed'] for r in report['cases']);write_json(out,report)
    if not report['passed']:raise SystemExit(1)


if __name__=='__main__':run(Path(sys.argv[1]))
