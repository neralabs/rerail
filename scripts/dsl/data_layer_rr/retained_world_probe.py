"""Exercise retained demand with a constant request and a changing filesystem."""
import os
from pathlib import Path
import tempfile
import sys
from migration import ROOT,MANIFEST,read_json,write_json,sha
from replay import execute_repository


def run(out):
    engine=ROOT/read_json(ROOT/MANIFEST)['engineSource']['cliArtifact']['path']
    policy=ROOT/'test/fixtures/rr/retained-world-demand.rr'
    report=dict(engineSha256=sha(engine.read_bytes()),cases=[],passed=False)
    with tempfile.TemporaryDirectory(prefix='retained-world-demand-') as temporary:
        for mode in ['snapshot','workspace']:
            root=Path(temporary)/mode;root.mkdir();(root/'pkg').mkdir()
            os.environ['RERAIL_TAPE_CACHE']=str(Path(temporary)/(mode+'.sqlite'))
            for step,present in enumerate([False,True,True,False,True]):
                file=root/'pkg/a.py'
                if present:file.write_text('x=1\n')
                elif file.exists():file.unlink()
                row=dict(mode=mode,step=step,expected=['pkg/a.py'] if present else [])
                try:
                    native=execute_repository(engine,policy,root,{},mode)
                    row.update(native=native,actual=[v['path'] for v in native['diagnostics']])
                    row['passed']=row['actual']==row['expected']
                except Exception as error:
                    row.update(error=str(error),passed=False)
                report['cases'].append(row);write_json(out,report)
    report['passed']=all(row['passed'] for row in report['cases']);write_json(out,report)
    if not report['passed']:raise SystemExit(1)


if __name__=='__main__':run(Path(sys.argv[1]))
