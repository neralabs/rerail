"""Counterfactual controls for scanner families, basename gates, paths, facts and CLI."""
import argparse
import json
from pathlib import Path
import re
import tempfile
from migration import ROOT,PIN,sha,compare,write_json
from replay import execute_native


def run(engine):
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-name-inference.rr';text=policy.read_text()
    library=(ROOT/'docs/rule-syntax/lib/data-layer/name-inference-api.rr').read_text()
    source=lambda value:[dict(path='input.py',source=value)]
    options=dict(entryPoint='_scan_source',nameDocs=['input.py'],nameBasenames=['runtime.py'])
    empty='\n.decl ninf_disabled()\n';specs=[]
    for i,content in enumerate(['column == "count"','ref.endswith(".raw")','refs.get(f"{ref}.raw")'],1):
        prefix=f'ppe_NameInference{i}(p,l,l,{i-1},0) :- '
        if prefix not in text:raise AssertionError('Missing scanner family')
        specs.append(('family-'+str(i),source(content),options,text.replace(prefix,prefix+'ninf_disabled(),')+empty,1,0))
    inline=lambda changed:text.replace('#include "data-layer/name-inference-api.rr"',changed)
    specs.extend([
        ('filename',source('column.split("_")'),options,text.replace('name_actual_name(d,p,"sql_column_type_hints.py")','name_actual_name(d,p,_)'),0,1),
        ('comment',source('# column == "count"'),options,re.sub(r', !match\([^\n]+?, t\)\.', '.',text,count=1),0,1),
        ('default-scope',[],dict(entryPoint='_name_paths',namePaths=['a.py','tests/a.py','x/__pycache__/b.py','test/c.py']),
         inline(library.replace('!match("(?s)(?:.*/)?(?:tests|__pycache__)(?:/.*)?",p).','i >= 0.')),2,4),
        ('name-check-flag',[],dict(entryPoint='_name_cli',checkMode='0',nameMessages=['a.py:1: SPNI001 message']),
         inline(library.replace('option("checkMode","0")','option("checkMode","SWAP")').replace(
             'option("checkMode","1")','option("checkMode","0")').replace('option("checkMode","SWAP")','option("checkMode","1")')),None,None),
        ('name-fact-line',source('\ncolumn == "count"'),dict(options,entryPoint='_extract_name_inference_facts'),
         inline(library.replace('to_string(l)','"1"')),None,None),
    ])
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in [Path(__file__),*[Path(__file__).with_name(n) for n in ['migration.py','replay.py']]]}
    report=dict(format='rerail.name-inference-controls/v1',sourcePin=PIN,oracle=False,engineSha256=sha(engine.read_bytes()),
        policySha256=sha(policy.read_bytes()),harnessFiles=hashes,passed=False,cases=[])
    folder=ROOT/'.rerail/name-inference-finalec3';folder.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='controls-',dir=folder) as temporary:
        for name,files,settings,mutation,a_count,b_count in specs:
            if mutation==text:raise AssertionError('Control changed nothing: '+name)
            path=Path(temporary)/(name+'.rr');path.write_text(mutation)
            a=execute_native(engine,policy,files,settings);b=execute_native(engine,path,files,settings)
            if a_count is not None:compare((len(a['diagnostics']),len(b['diagnostics'])),(a_count,b_count))
            elif name=='name-check-flag':
                status=lambda r:[v['parameters']['value'] for v in r['diagnostics'] if v['code']=='NINF_EXIT']
                compare(status(a),[0]);compare(status(b),[1])
            else:
                line=lambda r:json.loads(next(v['parameters']['value_json'] for v in r['diagnostics'] if v['code']=='NINF_FACT'))['line']
                compare(line(a),2);compare(line(b),1)
            assert a['diagnostics']!=b['diagnostics']
            report['cases'].append(dict(name=name,passed=True,original=a,disabled=b,mutation=mutation,mutationSha256=sha(mutation.encode())))
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True;return report


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--engine',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
    args=p.parse_args();value=run(args.engine.absolute());write_json(args.out,value);print(dict(controls=len(value['cases']),passed=True))
