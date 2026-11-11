"""Check that native scope comparisons reject removal of inference guards."""
import argparse
from pathlib import Path

from migration import ROOT,read_json,write_json,sha
from replay import execute_native,execute_workspace


def run(engine,baseline,out):
    original=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-state-tripwires-source.rr'
    program=original.read_text();case=read_json(baseline)['cases'][0]
    expected=case['expected']['scopes'];data=case['input']
    prefix='spt_1_'
    changes={
        'baseline':None,
        'unselected-facts':(f'{prefix}fact_context(f) :- {prefix}contexts(f,_,_), {prefix}reachable(f), docof(f,d), spt_needs_facts(d).',
                            f'{prefix}fact_context(f) :- {prefix}contexts(f,_,_), docof(f,d), spt_needs_facts(d).'),
        'unselected-runtime':(f'{prefix}runtime_context(f) :- {prefix}fact_context(f).',
                              f'{prefix}runtime_context(f) :- {prefix}contexts(f,_,_), docof(f,d), spt_needs_facts(d).'),
    }
    records=[];out.parent.mkdir(parents=True,exist_ok=True)
    for name,change in changes.items():
        text=program
        if change:
            if text.count(change[0])!=1:raise ValueError('Native inference guard changed: '+name)
            text=text.replace(*change)
        path=out.parent/(name+'.rr');path.write_text(text)
        for mode,execute in [('snapshot',execute_native),('workspace',execute_workspace)]:
            native=execute(engine,path,[dict(path=data['path'],source=data['source'])],dict(entryPoint='_analysis_contexts'))
            actual=dict(facts=[],runtime=[])
            for row in native['diagnostics']:
                if row['code']!='SPT_ANALYSIS_CONTEXT':raise ValueError('Unexpected analysis control diagnostic')
                p=row['parameters'];actual[p['family']].append([p['line'],p['name'],p['class_scope']] if p['family']=='facts' else [p['line'],p['name']])
            actual={key:sorted(values) for key,values in actual.items()}
            matched=actual==expected
            if matched!=(name=='baseline'):raise AssertionError('Scope control did not distinguish '+name)
            records.append(dict(control=name,mode=mode,matched=matched,expected=expected,actual=actual,
                mutation=change,programSha256=sha(text.encode()),native=native))
    report=dict(format='rerail.state-analysis-controls/v1',passed=True,fullMigrationAcceptance=False,
        engineSha256=sha(engine.read_bytes()),baselineSha256=sha(baseline.read_bytes()),programSha256=sha(original.read_bytes()),cases=records)
    write_json(out,report);print(dict(cases=len(records),passed=True))


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for key in ['engine','baseline','out']:parser.add_argument('--'+key,required=True,type=Path)
    args=parser.parse_args();run(args.engine.absolute(),args.baseline,args.out)
