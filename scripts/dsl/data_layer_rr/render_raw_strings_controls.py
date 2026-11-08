"""Disable independent native render outputs and require successful executions to fail positive assertions."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
from migration import ROOT,read_json,sha,write_json

SPECS=[('literal','literal-file','literal',0),('template','call-0-0-file','call-0-0',None),('static','call-3-0-file','call-3-0',None),
       ('statistics',None,None,0),('count-property',None,None,0),('repr','literal-file','literal',0),
       ('summary','formats-1',None,None),('detailed','formats-1',None,None),('hints','autofix-hints',None,None),
       ('source-gate','literal-file',None,0),('name','name-0',None,None),('docstring','docstring-file','docstring',None),
       ('allowed','allowed-1-0-file','allowed-1-0',None),('raise','nested-raise-file','nested-raise',None),('suppression','call-0-0-file','call-0-0',None)]


def disable(text,name):
    if name in {'literal','template','static'}:
        head,match={
            'literal':('rs_hit','"string_literal"'),
            'template':('rs_special','"template_call"'),
            'static':('rs_special','"static_call"'),
        }[name]
        changed,n=re.subn(r'(?m)^('+head+r'\([^\n]*?'+match+r'[^\n]*?\)\s*:-)',r'\1 1 = 0,',text)
    elif name in {'statistics','count-property'}:
        old='RSReport(@place(path, 0), methods, violations, selected)'
        new=old.replace('methods,','0,') if name=='statistics' else old.replace('violations,','0,')
        n=text.count(old);changed=text.replace(old,new)
    elif name=='repr':
        old='rs_repr_value(value, @python_repr(value))';n=text.count(old);changed=text.replace(old,'rs_repr_value(value, "disabled")')
    elif name in {'summary','detailed'}:
        changed,n=re.subn(r'(?m)^RSFormatted\(@place\("", 0\), "'+name+r'", .*?\) :-',
            'RSFormatted(@place("", 0), "'+name+'", "disabled") :-',text)
    else:
        head={'source-gate':'RSRead','name':'RSName','hints':'RSHint','docstring':'rs_docstring','allowed':'rs_allowed','raise':'rs_raise_or_assert','suppression':'rs_suppressed'}[name]
        changed,n=re.subn(r'(?m)^('+head+r'\([^\n]*?\)\s*:-)',r'\1 1 = 0,',text)
    expected=2 if name in {'summary','detailed','docstring','raise'} else 1
    assert n==expected,(name,n,expected)
    return changed


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for key in ['source','engine','baseline','api-oracle','reporter-oracle','out']:p.add_argument('--'+key,type=Path,required=True)
    p.add_argument('--control',type=int,required=True);args=p.parse_args()
    for key in ['source','engine','baseline','api_oracle','reporter_oracle','out']:setattr(args,key,getattr(args,key).absolute())
    name,api_id,report_id,original_index=SPECS[args.control];base=args.out.parent/('control-'+name);mutant=base/'disabled';mutant.mkdir(parents=True,exist_ok=True)
    rule=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/render-raw-strings.rr';disabled=disable(rule.read_text(),name);(mutant/rule.name).write_text(disabled)
    record=dict(name=name,disabledPolicy=disabled,disabledPolicySha256=sha(disabled.encode()))
    def run(label,script,arguments,red=False):
        out=base/(label+'.json');command=[sys.executable,str(Path(__file__).with_name(script)),*arguments,'--out',str(out)]
        if red:command+=['--rule-root',str(mutant)]
        r=subprocess.run(command,capture_output=True,text=True,timeout=100,env=dict(os.environ,PYTHONHASHSEED='0'))
        (base/(label+'.log')).write_text(r.stdout+r.stderr);return dict(exitCode=r.returncode,command=command,report=read_json(out))
    if original_index is not None:
        values=['native','--source',str(args.source),'--engine',str(args.engine),'--baseline',str(args.baseline),'--part',str(original_index),'--parts','5']
        green=run('original-green','render_raw_strings_contract.py',values);red=run('original-red','render_raw_strings_contract.py',values,True)
        assert (green['exitCode'],red['exitCode'])==(0,1),(name,green['exitCode'],red['exitCode'])
        assert not red['report']['semanticFallbacks'] and all(c['status']=='passed' for c in red['report']['nativeCalls'])
        record.update(originalGreen=green,originalRed=red)
    for interface,identity,script,corpus,oracle in [('api',api_id,'fresh.py','rr-fresh-render-raw-strings-cases.json',args.api_oracle),('reporter',report_id,'reporting.py','rr-render-raw-strings-reporter-cases.json',args.reporter_oracle)]:
        if identity is None:continue
        path=ROOT/'gate/data-layer/fixtures/host/data-layer-port'/corpus;cases=read_json(path)['cases'];index=next(i for i,c in enumerate(cases) if c['id']==identity)
        values=['native','--source',str(args.source),'--engine',str(args.engine),'--cases',str(path),'--oracle',str(oracle),'--part',str(index),'--parts',str(len(cases))]
        if interface=='api':values+=['--baseline',str(args.baseline)]
        green=run(interface+'-green',script,values);red=run(interface+'-red',script,values,True)
        assert (green['exitCode'],red['exitCode'])==(0,1),(name,interface,green['exitCode'],red['exitCode'])
        assert green['report']['results'][0]['status']=='passed' and red['report']['results'][0]['status']=='failed'
        assert not red['report'].get('semanticFallbacks') and not red['report'].get('sourceDetectorModules')
        record[interface+'Green']=green;record[interface+'Red']=red
    write_json(args.out,dict(format='rerail.render-raw-strings-controls/v1',engineSha256=sha(args.engine.read_bytes()),control=record))
    print(name)


if __name__=='__main__':main()
