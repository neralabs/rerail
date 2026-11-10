"""Compare complete shared model projections, field presence, types and messages."""
import argparse
import importlib
import json
from pathlib import Path
import sys
import tempfile

from surface_supplied_facts import cases as reporter_cases
from migration import ROOT,PIN,git,compare,read_json,write_json,sha


def cases():
    result=reporter_cases()
    for root in ['root','.', '/', '//', 'a/../b']:
        result.append(dict(id='projection-root:'+root,check='serverplan_grammar_surface',messageRoot=root,
            bundles=[dict(path=f'owner-{i}.py',value=dict(path=p,grammar_surface_hits=[dict(kind='k',symbol='s',line='3.0',message='m')]))
                for i,p in enumerate(['impl/a.py','a/../b.py','//network/a.py','./a//b.py',''])]))
    for line in ['7.0',True,2**100]:
        result.append(dict(id='projection-models:'+str(line),check='serverplan_grammar_surface',bundles=[dict(path='a.py',value=dict(
            path='a.py',classes=[dict(name='C',qualname='C',line=line,is_step_subclass='YES')],
            functions=[dict(name='f',qualname='C.f',line=line,is_async='off',parameter_names=['x','x'])],
            compile_methods=[dict(class_name='C',class_qualname='C',function_qualname='C.compile',line=line)],
            ignored={'deep':{'line':'not a model field'}}))]))
    result.append(dict(id='projection-duplicate-view-key',check='serverplan_grammar_surface',bundles=[
        dict(path='./same.py',value=dict(path='discarded.py')),dict(path='same.py',value=dict(path='kept.py'))]))
    return result


def describe(value):
    from pydantic import BaseModel
    if isinstance(value,BaseModel):
        return dict(type=type(value).__name__,fieldsSet=sorted(value.model_fields_set),
            frozen=type(value).model_config.get('frozen',False),
            values={k:describe(getattr(value,k)) for k in type(value).model_fields})
    if isinstance(value,tuple):return dict(tuple=[describe(v) for v in value])
    return value


def error_value(error):
    return dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=list(error.args),
        details=error.errors(include_url=False,include_context=False),text=str(error)))


def source_case(shared,case):
    from scripts.lint_lib.linter_plugin import PerFileFactBundle,ProjectFactView
    with tempfile.TemporaryDirectory(prefix='projection-source-') as folder:
        root=Path(folder);spec=shared.SERVERPLAN_SURFACE_EXTRACTOR
        bundles={root/b['path']:PerFileFactBundle(root/b['path'],'digest',facts={spec.extractor_id:b['value']}) for b in case['bundles']}
        view=ProjectFactView(root,bundles,(spec,))
        try:
            value=shared.project_serverplan_facts_from_view(view,spec)
            return dict(model=describe(value),messages=shared.grammar_surface_messages(Path(case.get('messageRoot','root')),value))
        except Exception as error:return error_value(error)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:parser.add_argument('--'+name,type=Path,required=name=='out')
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args();assert 0<=args.part<args.parts
    dependencies=[Path(__file__),Path(__file__).with_name('surface_supplied_facts.py')]
    if args.command=='source':
        source=args.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN);sys.path[:0]=[str(source),str(source/'scripts')]
        shared=importlib.import_module('scripts.lint_lib.serverplan_facts')
        file=source/'scripts/lint_lib/serverplan_facts.py';compare(file.read_text(),git(source,'show',PIN+':scripts/lint_lib/serverplan_facts.py')+'\n')
    else:
        from projection_port import native_case
        dependencies += [Path(__file__).with_name(p) for p in ['projection_port.py','model_dtos.py','model_transport.py','surface_reporter_port.py','replay.py','migration.py']]
        baseline=read_json(args.baseline);compare(baseline['passed'],True);compare(baseline['sourcePin'],PIN)
        compare([r['input'] for r in baseline['cases']],cases())
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in dependencies}
    report=dict(format='rerail.shared-model-projection/v1',execution=args.command,sourcePin=PIN,
        passed=False,fullMigrationAcceptance=False,harnessFiles=hashes,part=args.part,parts=args.parts,cases=[])
    if args.command=='source':report['sourceSha256']=sha(file.read_bytes())
    else:report.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()))
    for i in range(args.part,len(cases()),args.parts):
        case=cases()[i];row=dict(input=case)
        if args.command=='source':row['expected']=source_case(shared,case)
        else:
            row['expected']=baseline['cases'][i]['expected']
            row['actual'],row['executions']=native_case(args.engine,case,shared_module=None)
            row['passed']=json.dumps(row['expected'],sort_keys=True)==json.dumps(row['actual'],sort_keys=True)
        report['cases'].append(row);write_json(args.out,report)
        if args.command=='native':compare(row['passed'],True)
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report['passed']=True;write_json(args.out,report);print(dict(cases=len(report['cases']),passed=True))


if __name__=='__main__':main()
