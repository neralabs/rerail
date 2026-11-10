"""Disable each backend scanner family and check its recorded positive or boundary."""
import argparse
import json
from pathlib import Path
import re
import tempfile
from migration import ROOT,PIN,sha,compare,write_json
from replay import execute_native


def run(engine):
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/backend-type-strings.rr';text=policy.read_text()
    library=(ROOT/'docs/rule-syntax/lib/data-layer/backend-types-api.rr').read_text()
    cli=(ROOT/'docs/rule-syntax/lib/data-layer/text-cli.rr').read_text()
    source=lambda value:[dict(path='input.py',source=value)]
    ordinary=dict(entryPoint='check_file_content',sourcePath='impl/a.py')
    positive=['dialect.name == "sqlite"','dialect.name in ("mysql",)',
        'backend_type == "sqlite"','backend == "sqlite"','backend in ("sqlite",)',
        '"sqlite" in class_name','"sqlite" in type(conn)','conn.__class__.__name__.lower()']
    empty='\n.decl bts_disabled()\n'
    specs=[]
    for i,content in enumerate(positive):
        prefix=f'bts_occurrence(d, l, {i}, i, a, @slice(raw, a, b), snippet) :- '
        changed=text.replace(prefix,prefix+'bts_disabled(), ')+empty
        if changed==text+empty:raise AssertionError('Regex family absent')
        specs.append(('regex-'+str(i),source(content),ordinary,changed,1,0))
    specs.extend([
        ('skip-path',source(positive[2]),dict(ordinary,sourcePath='impl/tests/a.py'),
         text.replace('bts_skip(path) :-','bts_skip(path) :- bts_disabled(),')+empty,0,1),
        ('comment',source('# '+positive[2]),ordinary,
         re.sub(r', !match\([^\n]+?, raw\)\.', '.',text,count=1),0,1),
        ('direct-dotfile',[],dict(entryPoint='find_python_files',candidates=['.py'],candidateKinds=['direct']),
         text.replace('#include "data-layer/backend-types-api.rr"',library.replace(', strlen(@replace_all(p,".*/","")) > 3','')),0,1),
        ('batch-read',[],dict(entryPoint='_bts_prepare',batchPaths=['impl/a.py','impl/tests/a.py']),
         text.replace('#include "data-layer/backend-types-api.rr"',library.replace(
             'bts_batch_path(i,p), !bts_batch_skip(i).','bts_batch_path(i,p).').replace(
             'BatchSkipped(@place("",0),i,p) :-','BatchSkipped(@place("",0),i,p) :- bts_disabled(),'))+empty,1,2),
    ])
    # Count read rows for this control. A skipped marker is metadata, not a requested read.
    swapped=cli.replace('option("checkMode","0")','option("checkMode","SWAP")').replace(
        'option("checkMode","1")','option("checkMode","0")').replace('option("checkMode","SWAP")','option("checkMode","1")')
    specs.append(('backend-check-flag',[],dict(entryPoint='_cli_report',checkMode='0',cliFile=['a.py'],
        cliLine=['2'],cliColumn=['3'],cliCode=['BT003'],cliMessage=['message'],cliSnippet=['snippet']),
        text.replace('#include "data-layer/text-cli.rr"',swapped),None,None))
    values=[dict(path='backend-values.json',source=json.dumps(dict(values=[dict(file='a.py',line=True,column=2,message='m',code='BT003',snippet='s')])))]
    specs.append(('backend-fact-type',values,dict(entryPoint='_bts_fact'),
        text.replace('#include "data-layer/backend-types-api.rr"',library.replace('btf_line(i,l),','btf_line(i,old_l), l="1",')),None,None))
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in [Path(__file__),*[Path(__file__).with_name(n) for n in ['migration.py','replay.py']]]}
    result=dict(format='rerail.backend-types-controls/v1',sourcePin=PIN,oracle=False,engineSha256=sha(engine.read_bytes()),
        policySha256=sha(policy.read_bytes()),harnessFiles=hashes,passed=False,cases=[])
    folder=ROOT/'.rerail/backend-types-finalec3';folder.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='controls-',dir=folder) as temporary:
        for name,files,options,mutation,a_count,b_count in specs:
            if mutation==text:raise AssertionError('Control changed nothing: '+name)
            path=Path(temporary)/(name+'.rr');path.write_text(mutation)
            a=execute_native(engine,policy,files,options);b=execute_native(engine,path,files,options)
            if a_count is not None:
                select=lambda r:[v for v in r['diagnostics'] if
                    (name!='batch-read' or v['code']=='BTS_READ') and (name!='skip-path' or v['code']!='BTS_SKIPPED')]
                compare((len(select(a)),len(select(b))),(a_count,b_count))
            elif name=='backend-check-flag':
                select=lambda r:[v['parameters']['value'] for v in r['diagnostics'] if v['code']=='TEXT_CLI_EXIT']
                compare(select(a),[0]);compare(select(b),[1])
            else:
                select=lambda r:json.loads(next(v['parameters']['value_json'] for v in r['diagnostics'] if v['code']=='BTS_FACT'))['line']
                assert select(a) is True and type(select(b)) is int and select(b)==1
            assert a['diagnostics']!=b['diagnostics']
            result['cases'].append(dict(name=name,passed=True,original=a,disabled=b,mutation=mutation,mutationSha256=sha(mutation.encode())))
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    result['passed']=True;return result


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--engine',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
    args=p.parse_args();value=run(args.engine.absolute());write_json(args.out,value);print(dict(controls=len(value['cases']),passed=True))
