"""Reject empty detector outputs against original assertions and configured positives."""
import argparse,json,os,re,subprocess,sys
from pathlib import Path
from migration import ROOT,sha,read_json,write_json


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ['source','engine','baseline','api-oracle','reporter-oracle','out']:
        parser.add_argument('--'+key,type=Path,required=True)
    parser.add_argument('--group',choices=['families','auxiliary'],default='families')
    parser.add_argument('--family',type=int,choices=[1,2,3,4])
    args=parser.parse_args()
    for key in ['source','engine','baseline','api_oracle','reporter_oracle','out']:setattr(args,key,getattr(args,key).absolute())
    base=args.out.parent;source=(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/transaction-closures.rr').read_text()
    baseline=read_json(args.baseline);api=read_json(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-fresh-transaction-closures-cases.json')['cases']
    reports=read_json(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-transaction-closures-reporter-cases.json')['cases']
    controls=[]
    def run(name,script,arguments,rule_root=None):
        output=base/(name+'.json');command=[sys.executable,str(Path(__file__).with_name(script)),*arguments,'--out',str(output)]
        if rule_root:command.extend(['--rule-root',str(rule_root)])
        result=subprocess.run(command,capture_output=True,text=True,timeout=110,env={**os.environ,'PYTHONHASHSEED':'0'})
        (base/(name+'.log')).write_text(result.stdout+result.stderr)
        return dict(exitCode=result.returncode,report=read_json(output),command=command)
    for family,original,reporter in [
      (1,'test_detector_flags_nonlocal_keyword','named-nonlocal-both'),
      (2,'test_detector_flags_global_keyword','named-global-both'),
      (3,'test_detector_flags_mutating_method_on_outer_variable[append]','named-mutator-both'),
      (4,'test_detector_flags_subscript_assignment_to_outer_variable','named-subscript-both')] if args.group=='families' else []:
        if args.family is not None and family!=args.family:continue
        disabled,count=re.subn(r'(?m)^(Tx'+str(family)+r'\([^\n]*?\)\s*:-)',r'\1 1 = 0,',source)
        assert count==1;directory=base/('disabled-'+str(family));directory.mkdir(exist_ok=True)
        (directory/'transaction-closures.rr').write_text(disabled)
        index=next(i for i,t in enumerate(baseline['collected']) if t.endswith('::'+original))
        arguments=['native','--source',str(args.source),'--engine',str(args.engine),'--baseline',str(args.baseline),'--part',str(index),'--parts',str(len(baseline['collected']))]
        green=run('original-control-green-'+str(family),'transaction_contract.py',arguments)
        red=run('original-control-red-'+str(family),'transaction_contract.py',arguments,directory)
        assert green['exitCode']==0 and red['exitCode']==1 and not red['report']['semanticFallbacks']
        assert len(green['report']['calls'])==len(red['report']['calls'])==1
        assert all(c['status']=='passed' for c in red['report']['nativeCalls'])
        assert all(r['code'] in {'TX_LOCAL','TX_SCOPE','TX_CONSTANT'} for c in red['report']['nativeCalls'] for r in c['diagnostics'])
        before=green['report']['calls'][0]['after']['violations'];after=red['report']['calls'][0]['after']['violations']
        assert before and after==[]
        index=next(i for i,c in enumerate(reports) if c['id']==reporter)
        arguments=['native','--source',str(args.source),'--engine',str(args.engine),'--oracle',str(args.reporter_oracle),
            '--cases',str(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-transaction-closures-reporter-cases.json'),'--part',str(index),'--parts',str(len(reports))]
        rg=run('report-control-green-'+str(family),'reporting.py',arguments)
        rr=run('report-control-red-'+str(family),'reporting.py',arguments,directory)
        assert rg['exitCode']==0 and rr['exitCode']==1 and rg['report']['results'][0]['value']
        assert rr['report']['results'][0]['value']==[] and not rr['report'].get('sourceDetectorModules')
        controls.append(dict(family='TXN00'+str(family),disabledPolicy=disabled,disabledPolicySha256=sha(disabled.encode()),
            originalGreen=green,originalRed=red,reporterGreen=rg,reporterRed=rr))
    if args.group=='families':
        write_json(args.out,dict(engineSha256=sha(args.engine.read_bytes()),controls=controls,fullMigrationAcceptance=False,script=Path(__file__).read_text()))
        print(len(controls),'original and configured detector controls reject empty outputs');return
    disabled,count=re.subn(r'(?m)^((?:SyntaxFailure|ReadFailure|MissingTree)\([^\n]*?\)\s*:-)',r'\1 1 = 0,',source)
    assert count==5;directory=base/'disabled-parser';directory.mkdir(exist_ok=True);(directory/'transaction-closures.rr').write_text(disabled)
    index=next(i for i,c in enumerate(api) if c['id']=='file-syntax')
    arguments=['native','--source',str(args.source),'--engine',str(args.engine),'--baseline',str(args.baseline),'--oracle',str(args.api_oracle),
        '--cases',str(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-fresh-transaction-closures-cases.json'),'--part',str(index),'--parts',str(len(api))]
    green=run('parser-control-green','fresh.py',arguments);red=run('parser-control-red','fresh.py',arguments,directory)
    assert green['exitCode']==0 and red['exitCode']==1 and red['report']['results'][0]['value']==dict(result=[])
    assert not red['report']['semanticFallbacks'] and all(c['status']=='passed' for c in red['report']['calls'])
    constant_controls=[]
    for table,value,test in [('method','write','test_transaction_methods_include_core_variants'),
        ('mutator','append','test_mutating_methods_include_common_container_mutators')]:
        text=source.replace('tx_'+table+'("'+value+'").\n','');assert text!=source
        directory=base/('disabled-constant-'+table);directory.mkdir(exist_ok=True);(directory/'transaction-closures.rr').write_text(text)
        index=next(i for i,t in enumerate(baseline['collected']) if t.endswith('::'+test))
        arguments=['native','--source',str(args.source),'--engine',str(args.engine),'--baseline',str(args.baseline),'--part',str(index),'--parts',str(len(baseline['collected']))]
        green=run('constant-green-'+table,'transaction_contract.py',arguments)
        red=run('constant-red-'+table,'transaction_contract.py',arguments,directory)
        assert green['exitCode']==0 and red['exitCode']==1 and not red['report']['semanticFallbacks']
        assert all(c['status']=='passed' for c in red['report']['nativeCalls'])
        assert all(r['parameters']['value']!=value for c in red['report']['nativeCalls'] if c['entry']=='constants'
            for r in c['diagnostics'] if r['parameters']['table']==('transaction' if table=='method' else 'mutating'))
        constant_controls.append(dict(table=table,removed=value,green=green,red=red,disabledPolicy=text))
    result=dict(engineSha256=sha(args.engine.read_bytes()),controls=controls,parser=dict(green=green,red=red,disabledPolicy=disabled),constantControls=constant_controls,
        fullMigrationAcceptance=False,script=Path(__file__).read_text())
    write_json(args.out,result);print('Parser control and 2 native constant controls reject mutations')


if __name__=='__main__':main()
