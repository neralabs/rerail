"""Record real filesystem inventory failures and positive production reporter results."""
import argparse
from contextlib import redirect_stdout,redirect_stderr
import importlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from migration import ROOT,PIN,MANIFEST,compare,git,read_json,write_json,sha,verify_source_cohort,verify_engine_artifact
from dto import canonical
from dispatch_cli_contract import fixture,INVENTORY
from replay import program_inputs


def cases():
    witness=dict(path='impl/common/l2065_plan_core/a.py',source='ctx.is_sql_backend\n')
    def row(name,files=(),**extra):return dict(id=name,files=[witness,*files],**extra)
    def file(text):return dict(path=INVENTORY,source=text)
    rows=[row('missing'),row('empty',[file('')]),row('comments',[file('# comment\n \n')]),
        row('entries',[file('first\nfirst\nsecond\n')]),row('directory',[dict(path=INVENTORY,directory=True)]),
        row('dangling-link',[dict(path=INVENTORY,symlink='missing')]),
        row('directory-link',[dict(path='folder',directory=True),dict(path=INVENTORY,symlink='../../folder')]),
        row('text-link',[dict(path='text.txt',source='entry\n'),dict(path=INVENTORY,symlink='../../text.txt')]),
        row('parent-is-file',[dict(path='scripts/allowlists',source='file\n')]),
        row('symlink-loop',[dict(path=INVENTORY,symlink='lint_serverplan_dispatch_architecture.txt')]),
        row('unreadable',[file('entry\n')],mode=0),row('unsigned',signature='UNSIGNED'),row('invalid-signature',signature='!!!!')]
    for name,data in [('invalid-start','6162ff6364'),('invalid-continuation','6162e228a16364'),
        ('truncated','6162e282'),('surrogate','eda080'),('overlong','c080'),('too-large','f4908080')]:
        rows.append(row(name,[dict(path=INVENTORY,bytesHex=data)]))
    rows += [row('scan-fifo',[dict(path=INVENTORY,fifo=True)],action='scan'),
        row('scan-directory',[dict(path=INVENTORY,directory=True)],action='scan'),
        row('scan-unreadable',[file('entry\n')],action='scan',mode=0),
        row('scan-invalid-utf8',[dict(path=INVENTORY,bytesHex='ff')],action='scan')]
    return rows


def exception(error,root):
    result=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=canonical(error.args,root))
    if isinstance(error,OSError):result.update(errno=error.errno,filename=str(error.filename).replace(str(root),'$ROOT'))
    return result


def observe(module,root,signature=None,action=None):
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.fact_store import JsonFactStore
    from unittest.mock import patch
    if action=='scan':
        out,err=io.StringIO(),io.StringIO()
        with redirect_stdout(out),redirect_stderr(err):
            try:value=dict(value=canonical(module.scan_repository(root),root))
            except BaseException as error:value=dict(error=exception(error,root))
        return dict(value,stdout=out.getvalue(),stderr=err.getvalue())
    plugin=module.dispatch_architecture_plugin(root)
    engine=FactEngine(root=root,fact_store=JsonFactStore(root/'tmp/facts'),extractors=module.dispatch_fact_extractors(root),enable_fact_cache=False)
    view=engine.materialize((plugin,),max_workers=1);out,err=io.StringIO(),io.StringIO()
    with patch.object(module,'INVENTORY_SIGNATURE',signature if signature is not None else module.INVENTORY_SIGNATURE),redirect_stdout(out),redirect_stderr(err):
        try:value=dict(value=canonical(plugin.reporter(view),root))
        except BaseException as error:value=dict(error=exception(error,root))
    return dict(value,stdout=out.getvalue().replace(str(root),'$ROOT'),stderr=err.getvalue().replace(str(root),'$ROOT'))


def native(engine,module,root,case,mode,surface):
    policy=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr'
    cohort=read_json(ROOT/MANIFEST)['engineSource'];verify_source_cohort(ROOT,cohort);verify_engine_artifact(ROOT,cohort,engine)
    options={} if surface=='policy' else dict(entryPoint='_production_contract')
    if case.get('action')=='scan':options=dict(entryPoint='_dispatch_scan_repository',env=['data-layer.dispatch.scan-only=true'])
    if 'signature' in case:options['dispatchSignature']=case['signature']
    resources=[]
    if mode=='snapshot':
        buffers=[]
        for p in root.rglob('*'):
            if not p.is_file():continue
            name=p.relative_to(root).as_posix()
            try:p.read_text(encoding='utf-8')
            except (OSError,UnicodeDecodeError) as error:resources.append(dict(path=name,bufferError=type(error).__name__))
            else:buffers.append(name)
        options['inputFiles']=buffers
    execution=None
    with tempfile.TemporaryDirectory(prefix='dispatch-io-query-') as temporary:
        base=Path(temporary);write_json(base/'options.json',options)
        run=subprocess.run([str(engine),'run-rr',str(policy),str(root),str(base/'out'),str(base/'options.json')],capture_output=True,text=True,timeout=100)
        if run.returncode:return dict(error=exception(RuntimeError(run.stderr),root),stdout='',stderr=''),dict(returncode=run.returncode,stderr=run.stderr.replace(str(root),'$ROOT'),requestedPolicySha256=sha(policy.read_bytes()))
        execution=json.loads(run.stdout);execution['programInputs']=program_inputs(execution['programInputs']);execution['diskResources']=resources
    try:
        if case.get('action')=='scan':
            rows=execution['diagnostics']
            if any(r['code']!='DCLI_SCAN_FACT' for r in rows):raise ValueError('Unexpected native scan row')
            values=sorted((r['parameters'] for r in rows),key=lambda p:(p['path'],p['line'],p['source_code'],p['expression'],p['visit_order'],p['layer']))
            value=dict(value=canonical(tuple(module.Finding(p['source_code'],Path(p['path']),p['line'],p['scope'],p['expression'],p['message'],p['occurrence']) for p in values)),stdout='',stderr='')
        elif surface=='reporter':
            from dispatch_repository_contract import decode
            result=decode(module,execution);value=dict(value=result['violations'],stdout=result['stdout'],stderr=result['stderr'])
        else:
            rows=execution['diagnostics']
            from dispatch_repository_contract import raise_read_error
            raise_read_error(rows,root)
            values=[module.StructuredViolation(module.LINTER_NAME,r['code'],root/r['path'],r['line'],r['message'],'hard-fail') for r in rows]
            value=dict(value=canonical(values,root),stdout='',stderr='')
    except BaseException as error:value=dict(error=exception(error,root),stdout='',stderr='')
    return value,execution


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','out']:p.add_argument('--'+name,type=Path,required=name in ['source','out'])
    p.add_argument('--mode',choices=['snapshot','workspace'],default='workspace');p.add_argument('--surface',choices=['reporter','policy'],default='reporter');p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);a=p.parse_args()
    if not 0<=a.part<a.parts:p.error('Invalid shard')
    source=a.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN);path=source/'scripts/lint_serverplan_dispatch_architecture.py'
    compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':scripts/lint_serverplan_dispatch_architecture.py']))
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    rows=[];fallbacks=[];corpus=cases();baseline=read_json(a.baseline) if a.command=='native' else None
    if baseline:
        compare(baseline['passed'],True);compare(baseline['sourcePin'],PIN);compare(baseline['sourceSha256'],sha(path.read_bytes()));compare([r['input'] for r in baseline['cases']],corpus)
        from data_layer_audit.native_guard import codes
        original=codes({'dispatch':module})
        def guard(frame,event,_arg):
            if event=='call' and frame.f_code in original and not original[frame.f_code][2]:fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed')
    files=[Path(__file__),*[Path(__file__).with_name(n) for n in ['dispatch_cli_contract.py','dispatch_repository_contract.py','migration.py','replay.py','dto.py']]]
    report=dict(format='rerail.dispatch-inventory-io/v1',sourcePin=PIN,sourceSha256=sha(path.read_bytes()),admitted=False,oracle=a.command=='source',mode=a.mode,surface=a.surface,part=a.part,parts=a.parts,cases=rows,semanticFallbacks=fallbacks,complete=False,passed=False,harnessFiles={f.relative_to(ROOT).as_posix():sha(f.read_bytes()) for f in files})
    if baseline:report.update(engineSha256=sha(a.engine.read_bytes()),baselineSha256=sha(a.baseline.read_bytes()))
    write_json(a.out,report)
    for i,case in enumerate(corpus):
        if baseline and i%a.parts!=a.part:continue
        with tempfile.TemporaryDirectory(prefix='dispatch-inventory-io-') as temporary:
            root=Path(temporary).resolve();fixture(root,[f for f in case['files'] if not f.get('fifo')])
            for file in case['files']:
                if file.get('fifo'):
                    fifo_target=root/file['path'];fifo_target.parent.mkdir(parents=True,exist_ok=True);os.mkfifo(fifo_target)
            if 'mode' in case:(root/INVENTORY).chmod(case['mode'])
            row=dict(input=case)
            try:
                if not baseline:row['expected']=observe(module,root,case.get('signature'),case.get('action'))
                else:
                    sys.setprofile(guard)
                    try:actual,execution=native(a.engine.absolute(),module,root,case,a.mode,a.surface)
                    finally:sys.setprofile(None)
                    from validate_supplied_facts_checkpoint import equal
                    row.update(expected=baseline['cases'][i]['expected'],actual=actual,native=execution,passed=equal(actual,baseline['cases'][i]['expected']))
            finally:
                if 'mode' in case:(root/INVENTORY).chmod(0o600)
        rows.append(row);write_json(a.out,report);print(dict(id=case['id'],passed=row.get('passed',True)),flush=True)
    if not baseline:
        by_id={r['input']['id']:r['expected'] for r in rows};compare(bool(by_id['empty']['value']),True);compare(by_id['directory']['error']['type'],'builtins.IsADirectoryError');compare(by_id['invalid-start']['error']['type'],'builtins.UnicodeDecodeError')
    report.update(complete=True,passed=all(r.get('passed',True) for r in rows) and not fallbacks);write_json(a.out,report);raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
