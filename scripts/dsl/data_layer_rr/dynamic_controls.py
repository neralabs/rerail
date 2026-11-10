"""Require detector removals and missing exclusions to break recorded parity."""
import argparse
from copy import deepcopy
import os
from pathlib import Path
import re
import tempfile
from unittest.mock import patch
from migration import ROOT,PIN,read_json,write_json,sha,compare
from validate_supplied_facts_checkpoint import equal
from replay import execute_native,execute_workspace

WITNESSES=['getattr','globals','import','qualified','bare']


def results(execution,inputs):
    grouped={f'control-{i}.py':[] for i in range(len(inputs))}
    for row in execution['diagnostics']:
        if row['code']!='DML001' or row['path'] not in grouped:raise ValueError('Unexpected native control result')
        grouped[row['path']].append(row)
    values=[]
    for i,item in enumerate(inputs):
        path=(Path('root')/item.get('path','api/a.py')).as_posix()
        rows=sorted(grouped[f'control-{i}.py'],key=lambda r:(r['line'],r['parameters']['ordinal']))
        values.append(dict(value=[dict(kind='tuple',items=[dict(kind='path',value=path),r['line'],r['parameters']['snippet'],r['parameters']['label']]) for r in rows]))
    return values


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ['engine','baseline','out']:p.add_argument('--'+n,type=Path,required=True)
    p.add_argument('--mode',choices=['snapshot','workspace'],required=True);a=p.parse_args()
    source=read_json(a.baseline);compare(source['passed'],True);compare(source['sourcePin'],PIN)
    by_id={c['input']['id']:c for c in source['cases']};selected=[by_id[n] for n in WITNESSES]
    config=dict(by_id['configuration']['expected']['value']['items']);labels=config['primary.label']+config['imports.label']
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/dynamic-module-loading.rr';text=policy.read_text();execute=execute_native if a.mode=='snapshot' else execute_workspace
    report=dict(format='rerail.dynamic-controls/v1',fullMigrationAcceptance=False,sourcePin=PIN,complete=False,passed=False,mode=a.mode,engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()),cases=[],harnessFiles={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in [Path(__file__),Path(__file__).with_name('replay.py'),Path(__file__).with_name('migration.py')]})
    write_json(a.out,report)
    with tempfile.TemporaryDirectory(prefix='dynamic-controls-',dir=ROOT/'.rerail') as temporary:
        root=Path(temporary)
        with patch.dict(os.environ,{'RERAIL_TAPE_CACHE':str(root/'cache.sqlite')}):
            def run(name,changed,witnesses,wanted):
                if changed==text:raise ValueError('Control did not change its program')
                target=root/(name+'.rr');target.write_text(changed)
                inputs=[w['input'] for w in witnesses]
                files=[dict(path=f'control-{i}.py',source=w['content']) for i,w in enumerate(inputs)]
                # Logical names are caller arguments, never oracle outputs.
                options=dict(entryPoint='_scan_file_content',dmlDocuments=[f['path'] for f in files],dmlPaths=[w.get('path','api/a.py') for w in inputs])
                allowed={p for w in inputs for p in w.get('allowed',[])}
                if allowed:options['allowlistedFiles']=sorted(allowed)
                execution=execute(a.engine.absolute(),target,files,options);actual=results(execution,inputs)
                compare(equal(actual,wanted),True)
                expected=[w['expected'] for w in witnesses];compare(equal(actual,expected),False)
                row=dict(id=name,expected=expected,actual=actual,expectedMutationResult=wanted,passed=True,execution=execution,mutatedPolicy=changed,mutatedPolicySha256=sha(changed.encode()))
                report['cases'].append(row);write_json(a.out,report)
            for i,label in enumerate(labels):
                pattern=re.compile(r'(?m)^dml_candidate\(d, l, '+str(i)+r', [^\n]+\n')
                matches=pattern.findall(text);compare(len(matches),1)
                changed=text.replace(matches[0],matches[0].replace(':- dml_trigger(d),',':- 0=1,dml_trigger(d),'))
                wanted=deepcopy([w['expected'] for w in selected])
                for group in wanted:group['value']=[r for r in group['value'] if r['items'][3]!=label]
                run('pattern-'+str(i),changed,selected,wanted)
            negative_specs=[
                ('type-checking',"indent-' '-guard",'!dml_type_checking(d,l),',''),
                ('docstring','quotes-"""-unclosed','dml_state(d,l,""),dml_quotes(d,l,0,0),',''),
                ('fast-trigger','trigger-spacing',':- dml_trigger(d),',':- '),
                ('allowlist',"allowed-['api/a.py']",'!dml_allowed(owner),',''),
            ]
            for name,identity,needle,replacement in negative_specs:
                witness=by_id[identity];compare(witness['expected'],dict(value=[]));compare(needle in text,True)
                changed=text.replace(needle,replacement)
                target=root/(name+'.rr');target.write_text(changed);item=witness['input']
                options=dict(entryPoint='_scan_file_content',dmlDocuments=['control-0.py'],dmlPaths=[item.get('path','api/a.py')],allowlistedFiles=item.get('allowed',[]))
                execution=execute(a.engine.absolute(),target,[dict(path='control-0.py',source=item['content'])],options);actual=results(execution,[item])[0]
                compare(bool(actual['value']),True)
                report['cases'].append(dict(id=name,expected=witness['expected'],actual=actual,passed=True,execution=execution,mutatedPolicy=changed,mutatedPolicySha256=sha(changed.encode())));write_json(a.out,report)
    report.update(complete=True,passed=len(report['cases'])==9);write_json(a.out,report)
    print(dict(controls=len(report['cases']),passed=report['passed']))


if __name__=='__main__':main()
