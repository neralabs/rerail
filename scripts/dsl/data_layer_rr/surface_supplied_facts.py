"""Source-derived model-validation and reporting contracts for ServerPlan surface facts."""
import argparse
import importlib
import json
from pathlib import Path
import sys

from first_ten_supplied_facts import source_case
from migration import ROOT, PIN, git, compare, read_json, write_json, sha

CHECK='serverplan_grammar_surface'


def cases():
    hit=dict(kind='helper_definition',line=4,symbol='grammar_execute',message='first message')
    base=dict(path='impl/a.py',grammar_surface_hits=[hit])
    values=[None,[],{},dict(path='impl/a.py'),base,
        dict(base,syntax_error='supplied parser failure'),
        dict(base,grammar_surface_hits=[dict(hit,line=8),dict(hit,line=2),hit,hit])]
    for field in ['kind','line','symbol','message']:
        values.append(dict(base,grammar_surface_hits=[{k:v for k,v in hit.items() if k!=field}]))
        for value in [None,True,False,3,3.0,3.5,'3','3.0',2**100,[],{},'', '☃']:
            values.append(dict(base,grammar_surface_hits=[dict(hit,**{field:value})]))
    model_fields={
        'classes':dict(name='C',qualname='C',line=2,bases=['ServerPlanStep'],decorators=['wrapped'],is_step_subclass=True),
        'functions':dict(name='f',qualname='C.f',line=3,class_qualname='C',is_async=True,
            decorators=['wrapped'],return_annotation='int',parameter_names=['x'],call_names=['run'],
            attr_refs=['a'],name_refs=['b'],assigned_names=['c'],string_constants=['s']),
        'compile_methods':dict(class_name='C',class_qualname='C',function_qualname='C.compile',line=4),
    }
    for field,valid in model_fields.items():
        for value in [None,{},'wrong-container',[],[{}],[None],[valid]]:
            values.append(dict(base,**{field:value}))
        for missing in [k for k in valid if k in {'name','qualname','line','class_name','class_qualname','function_qualname'}]:
            values.append(dict(base,**{field:[{k:v for k,v in valid.items() if k!=missing}]}))
    for field in ['path','syntax_error','grammar_surface_hits']:
        for value in [None,False,3,3.5,{},[], '', '.', '//network/a.py']:
            values.append(dict(base,**{field:value}))
    for value in ['3e0','3.000',' 3.0 ','+3.0','3_0','3_0.0','٣','٣.0','-0.0',
                  '0.000000000000000000','-2e3',1e20,1e100,1e-100,0.0,-0.0]:
        values.append(dict(base,grammar_surface_hits=[dict(hit,line=value)]))
    for field,flag in [('classes','is_step_subclass'),('functions','is_async')]:
        for value in [None,False,0,1,2,0.0,1.0,0.5,[],{},'true','FALSE','yes','off','bad',' 1']:
            values.append(dict(base,**{field:[dict(model_fields[field],**{flag:value})]}))
    for field,member in [('classes','bases'),('classes','decorators'),('functions','parameter_names'),
                         ('functions','decorators'),('functions','string_constants')]:
        for value in [None,'bad-container',[None],[3],['one','one','☃']]:
            values.append(dict(base,**{field:[dict(model_fields[field],**{member:value})]}))
    result=[dict(id='surface-model:'+str(i),check=CHECK,bundles=[dict(path='input.py',value=value)])
            for i,value in enumerate(values)]
    result.append(dict(id='surface-model:file-order',check=CHECK,bundles=[
        dict(path=f'bundle-{i}.py',value=dict(base,path=p)) for i,p in enumerate(
            ['z.py','a/z.py','a-.py','./a.py','a.py','a.py','//network/a.py'])]))
    result.append(dict(id='surface-model:first-invalid-bundle',check=CHECK,bundles=[
        dict(path='a.py',value=base),dict(path='b.py',value={}),
        dict(path='c.py',value=dict(base,grammar_surface_hits=[{}]))]))
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__); parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=name=='out')
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args();assert 0<=args.part<args.parts
    dependencies=[Path(__file__),*[Path(__file__).with_name(n) for n in [
        'first_ten_supplied_facts.py','fact_transport.py','first_ten_production_contract.py','migration.py','replay.py']]]
    if args.command=='native':
        from surface_reporter_port import native_case
        dependencies += [Path(__file__).with_name(n) for n in ['model_transport.py','surface_reporter_port.py']]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in dependencies}
    report=dict(format='rerail.surface-supplied-facts/v1',sourcePin=PIN,execution=args.command,
        passed=False,fullMigrationAcceptance=False,harnessFiles=hashes,cases=[])
    if args.command=='source':
        source=args.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
        sys.path[:0]=[str(source),str(source/'scripts')]
        module=importlib.import_module('scripts.lint_'+CHECK)
        report['sourceFiles']={p:sha((source/p).read_bytes()) for p in [
            'scripts/lint_serverplan_grammar_surface.py','scripts/lint_lib/serverplan_facts.py']}
        for p in report['sourceFiles']:compare((source/p).read_text(),git(source,'show',PIN+':'+p)+'\n')
    else:
        baseline=read_json(args.baseline);compare(baseline['passed'],True);compare(baseline['sourcePin'],PIN)
        compare([r['input'] for r in baseline['cases']],cases())
        report.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()))
    for i in range(args.part,len(cases()),args.parts):
        case=cases()[i];row=dict(input=case)
        if args.command=='source':row['expected']=source_case(module,case)
        else:
            row['expected']=baseline['cases'][i]['expected']
            try:row['actual'],row['execution']=native_case(args.engine,case)
            except Exception as error:
                row['nativeFailure']=f'{type(error).__name__}: {error}'
                report['cases'].append(row);write_json(args.out,report);raise
            row['passed']=json.dumps(row['expected'],sort_keys=True)==json.dumps(row['actual'],sort_keys=True)
        report['cases'].append(row);write_json(args.out,report)
        if args.command=='native':compare(row['passed'],True)
    for p,digest in hashes.items():compare(digest,sha((ROOT/p).read_bytes()))
    report['passed']=True;write_json(args.out,report)
    print(dict(cases=len(report['cases']),errors=sum('error' in r['expected'] for r in report['cases']),passed=True))


if __name__=='__main__':main()
