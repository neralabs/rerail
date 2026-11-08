"""Bounded positive controls for each native transaction-pattern detector and helper interface."""
import argparse,os,re,subprocess,sys
from pathlib import Path
from migration import ROOT,read_json,write_json,sha

SCENARIOS=[
 ('detector',r'tp_finding\(f, param, category, calls\)', 'test_finder_flags_plain_database_connection_param',True),
 ('calls',r'tp_internal_call\(c, name, method, i\)','test_internal_transaction_finder_collects_all_three_variants',True),
 ('override',r'tp_final_category\(f, "INTERNAL_TXN_VIOLATION"\)','test_finder_overrides_category_when_internal_txn_violation_found',True),
 ('inference',r'TPInferred\(@place\(path, 0\), category\)','test_infer_category_backend_ops_default_for_api',False),
 ('signature',r'TPSignature\(@place\("", 0\), value\)','test_compute_signature_message_returns_utf8_bytes',False),
 ('verify',r'TPVerify\(@place\("", 0\), @ed25519_ok\(key, message, signature\)\)','test_sign_and_verify_categories_roundtrip',False),
 ('loader',r'TPLoad\(@place\("", 0\), value\)','test_load_baseline_returns_dict_for_valid_file',False),
 ('constants',r'TPConstant\(@place\("", 0\), "core", value\)','test_core_legitimate_functions_cover_transaction_generators',False),
 ('init',r'TPInit\(@place\("", 0\), 1\)','test_is_connection_class_init_matches_connection_class',False),
 ('categories',r'TPCategoryMember\(@place\("", 0\), i, category, name\)','test_get_categorized_functions_always_contains_core_legitimate',False),
]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['source','engine','baseline','reporter-oracle','out']:parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--scenario',type=int,required=True);args=parser.parse_args()
    for name in ['source','engine','baseline','reporter_oracle','out']:setattr(args,name,getattr(args,name).absolute())
    name,head,test,configured=SCENARIOS[args.scenario];base=args.out.parent/name;base.mkdir(parents=True,exist_ok=True)
    source=(ROOT/'gate/data-layer/packs/data-layer/candidates/rr/transaction-patterns.rr').read_text()
    disabled,count=re.subn(r'(?m)^('+head+r'\s*:-)',r'\1 1 = 0,',source);assert count==1,(name,count)
    directory=base/'disabled';directory.mkdir(exist_ok=True);(directory/'transaction-patterns.rr').write_text(disabled)
    baseline=read_json(args.baseline);index=next(i for i,t in enumerate(baseline['collected']) if t.endswith('::'+test))
    def run(label,script,arguments,mutant=False):
        path=base/(label+'.json');command=[sys.executable,str(Path(__file__).with_name(script)),*arguments,'--out',str(path)]
        if mutant:command.extend(['--rule-root',str(directory)])
        result=subprocess.run(command,capture_output=True,text=True,timeout=100,env={**os.environ,'PYTHONHASHSEED':'0'})
        (base/(label+'.log')).write_text(result.stdout+result.stderr)
        return dict(exitCode=result.returncode,command=command,report=read_json(path))
    arguments=['native','--source',str(args.source),'--engine',str(args.engine),'--baseline',str(args.baseline),
        '--part',str(index),'--parts',str(len(baseline['collected']))]
    green=run('original-green','patterns_contract.py',arguments);red=run('original-red','patterns_contract.py',arguments,True)
    assert (green['exitCode'],red['exitCode'])==(0,1),(name,green['exitCode'],red['exitCode'])
    assert green['report']['collected']==red['report']['collected']==baseline['collected']
    assert not red['report']['semanticFallbacks'] and all(c['status']=='passed' for c in red['report']['nativeCalls'])
    assert all({p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='failed',teardown='passed') for phases in red['report']['cases'].values())
    record=dict(name=name,disabledPolicy=disabled,disabledPolicySha256=sha(disabled.encode()),originalGreen=green,originalRed=red)
    if configured:
        cases=ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-transaction-patterns-reporter-cases.json';rows=read_json(cases)['cases'];i=next(i for i,r in enumerate(rows) if r['id']=='calls-ordered-duplicates')
        arguments=['native','--source',str(args.source),'--engine',str(args.engine),'--oracle',str(args.reporter_oracle),
            '--cases',str(cases),'--part',str(i),'--parts',str(len(rows))]
        cg=run('reporter-green','reporting.py',arguments);cr=run('reporter-red','reporting.py',arguments,True)
        assert (cg['exitCode'],cr['exitCode'])==(0,1)
        assert cg['report']['results'][0]['value'] and cr['report']['results'][0]['value']==[]
        record.update(reporterGreen=cg,reporterRed=cr)
    write_json(args.out,dict(engineSha256=sha(args.engine.read_bytes()),control=record,fullMigrationAcceptance=False,script=Path(__file__).read_text()))
    print(name,'rejects disabled native output')


if __name__=='__main__':main()
