"""Require both callback-wrapper findings to reject successful empty native output."""
import argparse,os,re,subprocess,sys
from pathlib import Path
from migration import ROOT,read_json,write_json,sha


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ['source','engine','baseline','oracle','out']:parser.add_argument('--'+key,type=Path,required=True)
    args=parser.parse_args()
    for key in vars(args):setattr(args,key,getattr(args,key).absolute())
    base=args.out.parent;source=(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/transaction-callback-wrappers.rr').read_text()
    baseline=read_json(args.baseline);case_path=ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-transaction-callback-wrappers-reporter-cases.json';cases=read_json(case_path)['cases'];controls=[]
    def run(name,script,arguments,rule_root=None):
        out=base/(name+'.json');command=[sys.executable,str(Path(__file__).with_name(script)),*arguments,'--out',str(out)]
        if rule_root:command.extend(['--rule-root',str(rule_root)])
        result=subprocess.run(command,capture_output=True,text=True,timeout=100,env={**os.environ,'PYTHONHASHSEED':'0'})
        (base/(name+'.log')).write_text(result.stdout+result.stderr)
        return dict(exitCode=result.returncode,command=command,report=read_json(out))
    for reason,test,case in [(0,'test_flags_transaction_callback_wrapper_helper','entry-read'),
        (1,'test_flags_inline_reentrant_versioned_callback_wrapper','named-versioned_read')]:
        text,count=re.subn(r'(?m)^(tcw_issue\(f, '+str(reason)+r'\)\s*:-)',r'\1 1 = 0,',source);assert count==1
        directory=base/f'disabled-{reason}';directory.mkdir(exist_ok=True);(directory/'transaction-callback-wrappers.rr').write_text(text)
        index=next(i for i,t in enumerate(baseline['collected']) if t.endswith('::'+test))
        arguments=['native','--check','transaction_callback_wrappers','--tests','scripts/tests/test_lint_transaction_callback_wrappers.py',
            '--source',str(args.source),'--engine',str(args.engine),'--baseline',str(args.baseline),'--part',str(index),'--parts',str(len(baseline['collected']))]
        green=run(f'original-control-green-{reason}','fact_contract.py',arguments)
        red=run(f'original-control-red-{reason}','fact_contract.py',arguments,directory)
        assert green['exitCode']==0 and red['exitCode']==1 and red['report']['collected']==baseline['collected']
        assert not red['report']['semanticFallbacks'] and not red['report']['missingResults']
        assert len(green['report']['events'])==len(red['report']['events'])==1
        assert green['report']['events'][0]['facts'] and red['report']['events'][0]['facts']==[]
        assert all(c['status']=='passed' and c['diagnostics']==[] for c in red['report']['nativeCalls'])
        assert all({p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='failed',teardown='passed') for phases in red['report']['cases'].values())
        index=next(i for i,c in enumerate(cases) if c['id']==case)
        arguments=['native','--source',str(args.source),'--engine',str(args.engine),'--cases',str(case_path),'--oracle',str(args.oracle),'--part',str(index),'--parts',str(len(cases))]
        rg=run(f'reporter-control-green-{reason}','reporting.py',arguments)
        rr=run(f'reporter-control-red-{reason}','reporting.py',arguments,directory)
        assert rg['exitCode']==0 and rr['exitCode']==1 and rg['report']['results'][0]['value'] and rr['report']['results'][0]['value']==[]
        controls.append(dict(reason=reason,disabledPolicy=text,disabledPolicySha256=sha(text.encode()),originalGreen=green,originalRed=red,reporterGreen=rg,reporterRed=rr))
    write_json(args.out,dict(engineSha256=sha(args.engine.read_bytes()),controls=controls,fullMigrationAcceptance=False,script=Path(__file__).read_text()))
    print('Both original and configured positive controls reject disabled findings')


if __name__=='__main__':main()
