"""Disable inspection families, read selection, fact ownership, or CLI status."""
import argparse
import json
from pathlib import Path
import re
import tempfile
from migration import ROOT,PIN,sha,compare,write_json
from replay import execute_native


def run(engine):
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/no-source-inspection.rr';text=policy.read_text()
    library=(ROOT/'docs/rule-syntax/lib/data-layer/inspection-api.rr').read_text()
    cli=(ROOT/'docs/rule-syntax/lib/data-layer/text-cli.rr').read_text()
    source=lambda value:[dict(path='input.py',source=value)]
    options=dict(entryPoint='find_violations',inspectionDocs=['input.py'],inspectionPaths=['/project/tests/a.py'])
    empty='\n.decl nsi_disabled()\n';specs=[]
    for i,content in enumerate(['inspect.getsource(f)','inspect.getsourcelines(f)','inspect.getsourcefile(f)',
                                'from inspect import getsource','from inspect import getsourcelines']):
        line=next(line for line in text.splitlines() if line.startswith(f'ppe_Inspection{i}('))
        specs.append(('family-'+str(i),source(content),options,text.replace(line,line.replace(' :- ',' :- nsi_disabled(), ',1))+empty,1,0))
    inline=lambda changed:text.replace('#include "data-layer/inspection-api.rr"',changed)
    specs.extend([
        ('excluded-owner',source('inspect.getsource(f)'),dict(options,inspectionPaths=['/project/tests/__pycache__/a.py']),
         text.replace('!inspection_excluded(d),',''),0,1),
        ('comment',source('# inspect.getsource(f)'),options,re.sub(r', !match\([^\n]+?, t\)', '',text,count=1),0,1),
        ('read-selection',[],dict(entryPoint='_inspection_prepare',inspectionCandidates=['/project/tests/a.py','/project/tests/__pycache__/a.py','/project/a.txt']),
         inline(library.replace(', !inspection_path_excluded(p)','')),1,2),
        ('inspection-check-flag',[],dict(entryPoint='_cli_report',checkMode='0',sourceRoot='/project',
            cliFile=['/project/tests/a.py'],cliLine=['2'],cliSnippet=['line'],cliPattern=['pattern']),
         text.replace('#include "data-layer/text-cli.rr"',cli.replace('option("checkMode","0")','option("checkMode","SWAP")').replace(
             'option("checkMode","1")','option("checkMode","0")').replace('option("checkMode","SWAP")','option("checkMode","1")')),None,None),
        ('inspection-fact-owner',source('inspect.getsource(f)'),dict(options,entryPoint='scan_file_content'),
         inline(library.replace('@json_str(owner)','@json_str(p)')),None,None),
    ])
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in [Path(__file__),*[Path(__file__).with_name(n) for n in ['migration.py','replay.py']]]}
    report=dict(format='rerail.inspection-controls/v1',sourcePin=PIN,oracle=False,engineSha256=sha(engine.read_bytes()),
        policySha256=sha(policy.read_bytes()),harnessFiles=hashes,passed=False,cases=[])
    folder=ROOT/'.rerail/inspection-finalec3';folder.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='controls-',dir=folder) as temporary:
        for name,files,settings,mutation,a_count,b_count in specs:
            if mutation==text:raise AssertionError('Control changed nothing: '+name)
            path=Path(temporary)/(name+'.rr');path.write_text(mutation)
            a=execute_native(engine,policy,files,settings);b=execute_native(engine,path,files,settings)
            if a_count is not None:compare((len(a['diagnostics']),len(b['diagnostics'])),(a_count,b_count))
            elif name=='inspection-check-flag':
                statuses=lambda r:[v['parameters']['value'] for v in r['diagnostics'] if v['code']=='TEXT_CLI_EXIT']
                compare(statuses(a),[0]);compare(statuses(b),[1])
            else:
                owner=lambda r:json.loads(next(v['parameters']['value_json'] for v in r['diagnostics'] if v['code']=='NSI_FACT'))['file_path']
                compare(owner(a),'/project/tests/a.py');compare(owner(b),'input.py')
            assert a['diagnostics']!=b['diagnostics']
            report['cases'].append(dict(name=name,passed=True,original=a,disabled=b,mutation=mutation,mutationSha256=sha(mutation.encode())))
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True;return report


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--engine',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
    args=p.parse_args();value=run(args.engine.absolute());write_json(args.out,value);print(dict(controls=len(value['cases']),passed=True))
