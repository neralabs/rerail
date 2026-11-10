"""Source/native production comparisons for nontrivial import and literal shapes."""
import argparse
import importlib
from pathlib import Path
import subprocess
import sys
from migration import ROOT,PIN,read_json,write_json,compare,sha,git
from first_ten_production_contract import source_case,native_case

CHECK='no_api_root_imports'
CASES=[dict(id=CHECK+':'+name,check=CHECK,files=[dict(path='impl/a.py',source=text)]) for name,text in [
    ('unicode-off','from ａｐｉ import X\n'),('unicode-on','# import api\nfrom ａｐｉ import ＬENGTH\n'),
    ('literal-order','value="import api\\nfrom api import X"\n'),
    ('concat','value=("from api import X\\n"\n "import api")\n')]]


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:p.add_argument('--'+name,type=Path,required=name=='out')
    a=p.parse_args();paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['first_ten_production_contract.py','replay.py','migration.py']]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths}
    if a.command=='source':
        source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN);path=source/f'scripts/lint_{CHECK}.py'
        compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':'+path.relative_to(source).as_posix()]))
        sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_'+CHECK)
        report=dict(format='rerail.api-imports-production-source/v1',sourcePin=PIN,passed=True,sourceSha256=sha(path.read_bytes()),harnessFiles=hashes,
                    cases=[dict(input=c,expected=source_case(module,c)) for c in CASES])
    else:
        baseline=read_json(a.baseline);compare(baseline['passed'],True);compare([r['input'] for r in baseline['cases']],CASES)
        assert not any(n.startswith('scripts.lint_') for n in sys.modules)
        report=dict(format='rerail.api-imports-production-native/v1',sourcePin=PIN,oracle=False,passed=False,
            engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),harnessFiles=hashes,cases=[])
        for row in baseline['cases']:
            try:actual,native=native_case(a.engine.absolute(),row['input'])
            except Exception as error:actual,native=dict(nativeFailure=str(error)),{}
            report['cases'].append(dict(input=row['input'],expected=row['expected'],actual=actual,native=native,passed=actual==row['expected']));write_json(a.out,report)
        report['passed']=all(r['passed'] for r in report['cases'])
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    write_json(a.out,report);print(dict(cases=len(report['cases']),passed=report['passed']));raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
