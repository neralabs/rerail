"""Independent cache-family and transferable-property controls over successful native execution."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
from migration import ROOT, read_json, sha, write_json

SPECS = [
    ('dict_cache','value-0-content','value-0',0),
    ('set_cache','value-6-content','value-6',None),
    ('list_cache','value-9-content','value-9',5),
    ('constructor_cache','value-15-content','value-15',3),
    ('lru_cache','decorator-0-content','decorator-0',0),
    ('invalid_allowlist','allow-missing-reason','allow-missing-reason',7),
    ('violation-key',None,None,0),
    ('entry-key',None,None,6),
    ('entry-signature',None,None,6),
    ('violation-format','format-0',None,None),
    ('severity-counts','counts-2',None,None),
]


def disable(source, name):
    if name in {'dict_cache','set_cache','list_cache','constructor_cache','lru_cache'}:
        result,count=re.subn(r'(?m)^(mc_result\([^\n]*?\)\s*:-)',lambda m:m[0]+' pattern != "'+name+'",',source)
    elif name=='invalid_allowlist':
        result,count=re.subn(r'(?m)^(mc_al_error\(i, 7,[^\n]*?\)\s*:-)',r'\1 1 = 0,',source)
    elif name=='severity-counts':
        result,count=re.subn(r'(?m)^(MCCount\([^\n]*?\)\s*:-)',r'\1 1 = 0,',source)
    else:
        original='MCFact(@place(path, l), c, name, pattern, severity, i, sub, key, formatted, message)'
        entry='MCAllowEntry(@place("", 0), i, path, pattern, name, classification, reason, key, signature)'
        old,new={
            'violation-key':(original,original.replace(', key,',', "disabled",')),
            'violation-format':(original,original.replace(', formatted,',', "disabled",')),
            'entry-key':(entry,entry.replace(', key,',', "disabled",')),
            'entry-signature':(entry,entry.replace(', signature)',', "disabled")')),
        }[name]
        count=source.count(old)
        result=source.replace(old,new)
    assert count==1,(name,count)
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['source','engine','baseline','api-oracle','reporter-oracle','out']:
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--control',type=int,required=True)
    args=parser.parse_args()
    for name in ['source','engine','baseline','api_oracle','reporter_oracle','out']:
        setattr(args,name,getattr(args,name).absolute())
    name,api_id,report_id,original_index=SPECS[args.control]
    base=args.out.parent/('control-'+name)
    mutant=base/'disabled';mutant.mkdir(parents=True,exist_ok=True)
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/module-level-caches.rr'
    disabled=disable(policy.read_text(),name)
    (mutant/policy.name).write_text(disabled)
    def run(label,script,arguments,red=False):
        out=base/(label+'.json')
        command=[sys.executable,str(Path(__file__).with_name(script)),*arguments,'--out',str(out)]
        if red:command+=['--rule-root',str(mutant)]
        result=subprocess.run(command,capture_output=True,text=True,timeout=100,
            env={**os.environ,'PYTHONHASHSEED':'0'})
        (base/(label+'.log')).write_text(result.stdout+result.stderr)
        return dict(exitCode=result.returncode,command=command,report=read_json(out))
    record=dict(name=name,disabledPolicy=disabled,disabledPolicySha256=sha(disabled.encode()))
    if original_index is not None:
        arguments=['native','--source',str(args.source),'--engine',str(args.engine),'--baseline',str(args.baseline),
            '--part',str(original_index),'--parts','8']
        green=run('original-green','module_caches_contract.py',arguments)
        red=run('original-red','module_caches_contract.py',arguments,True)
        assert (green['exitCode'],red['exitCode'])==(0,1),(name,green['exitCode'],red['exitCode'])
        assert green['report']['collected']==red['report']['collected']==read_json(args.baseline)['collected']
        assert not red['report']['semanticFallbacks']
        assert all(c['status']=='passed' for c in red['report']['nativeCalls'])
        assert all({p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='failed',teardown='passed')
            for phases in red['report']['cases'].values())
        record.update(originalGreen=green,originalRed=red)
    for interface,identity,script,filename,oracle in [
        ('api',api_id,'fresh.py','rr-fresh-module-level-caches-cases.json',args.api_oracle),
        ('reporter',report_id,'reporting.py','rr-module-level-caches-reporter-cases.json',args.reporter_oracle)]:
        if identity is None:continue
        cases=ROOT/'gate/data-layer/fixtures/host/data-layer-port'/filename
        rows=read_json(cases)['cases'];i=next(i for i,r in enumerate(rows) if r['id']==identity)
        arguments=['native','--source',str(args.source),'--engine',str(args.engine),'--oracle',str(oracle),
            '--cases',str(cases),'--part',str(i),'--parts',str(len(rows))]
        if interface=='api':arguments+=['--baseline',str(args.baseline)]
        green=run(interface+'-green',script,arguments);red=run(interface+'-red',script,arguments,True)
        assert (green['exitCode'],red['exitCode'])==(0,1),(name,interface,green['exitCode'],red['exitCode'])
        assert green['report']['results'][0]['status']=='passed' and red['report']['results'][0]['status']=='failed'
        if interface=='reporter':assert green['report']['results'][0]['value'] and red['report']['results'][0]['value']==[]
        record.update({interface+'Green':green,interface+'Red':red})
    write_json(args.out,dict(engineSha256=sha(args.engine.read_bytes()),control=record,
        fullMigrationAcceptance=False,script=Path(__file__).read_text()))


if __name__=='__main__':main()
