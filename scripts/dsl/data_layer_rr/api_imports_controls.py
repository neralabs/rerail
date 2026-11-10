"""Counterexamples for API-import matching, scopes, DTO types and CLI enforcement."""
import argparse
import json
from pathlib import Path
import tempfile
from migration import ROOT,PIN,sha,compare,write_json
from replay import execute_native


def run(engine):
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/no-api-root-imports.rr';text=policy.read_text()
    library=(ROOT/'docs/rule-syntax/lib/data-layer/api-imports-api.rr').read_text()
    def inline(changed):return text.replace('#include "data-layer/api-imports-api.rr"',changed)
    def source(value):return [dict(path='input.py',source=value)]
    ordinary=dict(entryPoint='check_file')
    batch=dict(entryPoint='check_file',nariBatch='true',nariRoot='/project',ownerDocs=['input.py'],ownerPaths=['/project/a.py'])
    empty='\n.decl ni_disabled()\n'
    values=[dict(path='api-imports-values.json',source=json.dumps(dict(rows=[dict(raw=dict(line=2.5,message='ok'),fallback='a.py')])))]
    specs=[
        ('ast-import',source('from api import X\n'),ordinary,
         text.replace('nari_module(i) :-','nari_module(i) :- ni_disabled(),')+empty,1,0),
        ('literal-from',source('x="from api import X"\n'),ordinary,
         text.replace('nari_regex_0(t, i, a, b) :-','nari_regex_0(t, i, a, b) :- ni_disabled(),')+empty,1,0),
        ('literal-import',source('x="import api"\n'),ordinary,
         text.replace('nari_regex_1(t, i, a, b) :-','nari_regex_1(t, i, a, b) :- ni_disabled(),')+empty,1,0),
        ('supplied-source',source('from api import X\n'),dict(batch,entryPoint='check_file_content',providedSource='pass'),
         text+'\nnari_api_gate(d) :- nari_file(d).\n',0,1),
        ('dotfile-suffix',source('from api import X\n'),dict(batch,ownerPaths=['/project/.py']),
         inline(library.replace(', strlen(@replace_all(owner,".*/","")) > 3','')),0,1),
        ('test-scope',source('from api import X\n'),dict(batch,ownerPaths=['/project/tests/a.py']),
         inline(library.replace('!ni_test(i),','')),0,1),
        ('duplicate-files',[dict(path=p,source='from api import X\n') for p in ['first.py','second.py']],
         dict(batch,ownerDocs=['first.py','second.py'],ownerPaths=['/project/a.py','/project/a.py']),
         inline(library.replace('ni_first(owner,i),','')),1,2),
        ('walk-suffix',[],dict(entryPoint='_ni_walk_files',walkFiles=['a.py','a.txt'],excludeDirs=[]),
         inline(library.replace('suffix(".py",p), !ni_walk_excluded(i)','!ni_walk_excluded(i)')),1,2),
        ('check-flag',[],dict(entryPoint='_ni_cli_report',sourceRoot='/project',argv=['lint.py','--check=yes'],
             cliPath=['/project/a.py'],cliLine=['2'],cliMessage=['message']),text+'\nnic_check().\n',None,None),
        ('fact-line-type',values,dict(entryPoint='_ni_from_fact'),text+'\nnif_integer(i) :- nif_line_kind(i,"number").\n',None,None)]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in [Path(__file__),*[Path(__file__).with_name(n) for n in ['migration.py','replay.py']]]}
    report=dict(format='rerail.api-imports-controls/v1',sourcePin=PIN,oracle=False,engineSha256=sha(engine.read_bytes()),
                policySha256=sha(policy.read_bytes()),harnessFiles=hashes,passed=False,cases=[])
    folder=ROOT/'.rerail/api-imports-finalec3';folder.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='controls-',dir=folder) as temporary:
        for name,files,options,mutation,a_count,b_count in specs:
            if mutation==text:raise AssertionError('Control did not change behavior: '+name)
            path=Path(temporary)/(name+'.rr');path.write_text(mutation)
            a=execute_native(engine,policy,files,options);b=execute_native(engine,path,files,options)
            if a_count is not None:compare((len(a['diagnostics']),len(b['diagnostics'])),(a_count,b_count))
            elif name=='check-flag':
                status=lambda r:[x['parameters']['value'] for x in r['diagnostics'] if x['code']=='NI_EXIT']
                compare(status(a),[0]);compare(status(b),[1])
            else:
                value=lambda r:json.loads(next(x['parameters']['value_json'] for x in r['diagnostics'] if x['code']=='NI_VALUE'))
                compare(value(a),None);compare(value(b)['line'],2.5)
            assert a['diagnostics']!=b['diagnostics']
            report['cases'].append(dict(name=name,passed=True,original=a,disabled=b,mutation=mutation,mutationSha256=sha(mutation.encode())))
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True;return report


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--engine',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
    a=p.parse_args();value=run(a.engine.absolute());write_json(a.out,value);print(dict(controls=len(value['cases']),passed=True))


if __name__=='__main__':main()
