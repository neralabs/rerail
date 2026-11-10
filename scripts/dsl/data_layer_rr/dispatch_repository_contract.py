"""Record dispatch repository scope, facts, metadata, and complete reporter results."""
import argparse
from contextlib import ExitStack,redirect_stderr,redirect_stdout
from dataclasses import asdict
import importlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT,PIN,compare,git,sha,read_json,write_json
from dto import canonical
from replay import write_fixture


def cases(module):
    base=module._BASE_SERVER_PLAN_FILE
    root=module._GENERIC_DIRS[0]
    witness='def f(ctx):\n if ctx.backend_type == "sqlite": pass\n'
    rows=[]
    def add(name,files=(),**extra):rows.append(dict(id=name,files=list(files),**extra))
    def file(path,source=witness):return dict(path=path,source=source)
    add('empty');add('clean',[file(root+'/clean.py','pass\n')])
    add('generic-roots',[file(p+'/positive.py') for p in module._GENERIC_DIRS])
    add('outside',[file('impl/outside.py'),file('api/a.py'),file('other/a.py')])
    add('base-executor',[file(base,'def f(backend):\n return backend.transport\n')])
    add('base-clean',[file(base,'def f(backend):\n return backend.get_lowering_context().branch_table\n')])
    add('mixed-codes',[file(root+'/all.py','ctx.is_sql_backend\nisinstance(step,QueryStep)\ntype(step).__name__ == "QueryStep"\nif dialect: pass\n')])
    add('excluded-tests',[file(root+'/'+p+'/a.py') for p in ['test','tests','conform','normal']])
    add('test-names',[file(root+'/'+p) for p in ['test_a.py','a_test.py','a.py','conftest.py']])
    add('nested-boundaries',[file(root+'/'+p+'/a.py') for p in ['.git','.venv','venv','tmp','__pycache__','node_modules','.hidden']])
    add('extensions',[file(root+'/'+p) for p in ['.py','.hidden.py','a.py','a.PY','a.pyi','notes.txt']])
    add('ordering',[file(root+'/'+p) for p in ['z.py','a/z.py','a-.py','a.py']])
    add('duplicates',[file(root+'/a.py',witness+witness)])
    add('syntax',[file(root+'/broken.py','def broken(:\nctx.is_sql_backend\n')])
    add('late-boundaries',[file(p) for p in sorted(module._LATE_BOUNDARY_FILES)])
    add('cross-file-type',[file(base,'def f(engine:RenderEngine):\n engine.transport\n'),file(root+'/other.py','RenderEngine=BackendExecutor\n')])
    add('shared-source-scopes',[file(base,'def f(backend):\n backend.transport\n'),file(root+'/same.py','def f(backend):\n backend.transport\n')])
    add('alias-from-base',[file(base,'def f(backend):\n backend.transport\n'),dict(path=root+'/alias.py',symlink='../l2070_server_plan/base_server_plan.py')])
    add('alias-to-base',[file('impl/shared.py','def f(backend):\n backend.transport\n'),dict(path=base,symlink='../../shared.py')])
    inventory='scripts/allowlists/lint_serverplan_dispatch_architecture.txt'
    add('empty-inventory',[file(inventory,'# no suppressions\n \n')])
    add('nonempty-inventory',[file(root+'/a.py'),file(inventory,'entry\n')])
    add('bad-signature',[file(root+'/a.py')],signature='invalid')
    add('empty-scope-directories',[dict(path=p,directory=True) for p in module._GENERIC_DIRS])
    add('scope-root-is-file',[file(root,'pass\n')])
    add('base-is-directory',[dict(path=base,directory=True)])
    add('directory-alias',[file(root+'/sub/a.py'),dict(path=root+'/link',symlink='sub')])
    add('scope-root-alias',[file('shared/a.py'),dict(path=root,symlink='../../shared')])
    add('inventory-alias',[file('shared-inventory.txt','entry\n'),dict(path=inventory,symlink='../../shared-inventory.txt')])
    add('inventory-alias-empty',[file('shared-inventory.txt','# comment\n'),dict(path=inventory,symlink='../../shared-inventory.txt')])
    for name,text in [('unicode-comment','\x1c\u2000# comment\u0085\u2000\n'),
        ('unicode-lines','entry\u0085second\u2028third\u2029'),('duplicate-lines','entry\nentry\n'),
        ('inline-comment','entry # retained\n')]:add('inventory-'+name,[file(inventory,text)])
    for value in ['','UNSIGNED','NEEDS_SIGNING','!!!!','AAAA']:
        add('signature-'+value,[file(root+'/a.py')],signature=value)
    add('inventory-short-circuits-signature',[file(inventory,'entry\n')],signature='invalid')
    for name,text in [('long','entry '+('x'*1200)+'\n'),('nul','alpha\0beta\n'),
        ('mixed-lines',' a\r\nb\rc\v d\f e\x1c f\x1d g\x1e h\n'),
        ('whitespace','\u2000keep\tinternal\u2000\n\u2000# ignored\n')]:
        add('inventory-'+name,[file(inventory,text)])
    return rows


def observe(module,case):
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.fact_store import JsonFactStore
    with tempfile.TemporaryDirectory(prefix='dispatch-repository-') as temporary,ExitStack() as stack:
        root=Path(temporary);write_fixture(root,case['files'])
        stack.enter_context(patch.object(module,'ROOT',root))
        if 'signature' in case:stack.enter_context(patch.object(module,'INVENTORY_SIGNATURE',case['signature']))
        plugin=module.dispatch_architecture_plugin(root)
        engine=FactEngine(root=root,fact_store=JsonFactStore(root/'tmp/facts'),extractors=module.dispatch_fact_extractors(root),enable_fact_cache=False)
        stdout,stderr=io.StringIO(),io.StringIO()
        with redirect_stdout(stdout),redirect_stderr(stderr):
            plan=engine.plan([plugin]);view=engine.materialize((plugin,),max_workers=1)
            findings=module._findings(view);violations=plugin.reporter(view)
            inventory=module._load_inventory(root/'scripts/allowlists/lint_serverplan_dispatch_architecture.txt')
        return dict(selectedFiles=[p.relative_to(root).as_posix() for p in plan.files],
            facts=canonical({p:b.facts for p,b in sorted(view.files.items())},root),
            findings=canonical(tuple(findings),root),violations=canonical(violations,root),
            plugin=canonical(dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description,
                file_scope=asdict(plugin.file_scope),required_facts=tuple(asdict(f) for f in plugin.required_facts)),root),
            inventory=canonical(inventory),stdout=stdout.getvalue().replace(str(root),'$ROOT'),stderr=stderr.getvalue().replace(str(root),'$ROOT'))


def decode(module,native):
    rows=native['diagnostics']
    allowed={'DL_FILE','DL_META','SDR_CONFIG','SDR_SEQUENCE','SDR_SCOPE','SDR_FACT','SDR_REPORT','SDR_OUTPUT','SDR_INVENTORY'}
    if any(r['code'] not in allowed for r in rows):raise ValueError('Unexpected repository contract rows')
    config={}
    for r in rows:
        if r['code']!='SDR_CONFIG':continue
        p=r['parameters'];identity=(p['section'],p['key'])
        if identity in config:raise ValueError('Duplicate configuration field')
        config[identity]=p
    if not config:raise ValueError('Native dispatch repository metadata is missing')
    def scalar(kind,value):
        if kind=='string':return value
        if kind=='boolean':return {'true':True,'false':False}[value]
        if kind=='none':
            if value!='':raise ValueError('Unexpected null payload')
            return None
        raise ValueError('Unknown configuration scalar: '+kind)
    def value(p):
        if p['kind']!='tuple':return scalar(p['kind'],p['value'])
        items=sorted((r['parameters'] for r in rows if r['code']=='SDR_SEQUENCE' and
            (r['parameters']['section'],r['parameters']['key'])==(p['section'],p['key'])),key=lambda p:p['ordinal'])
        if [r['ordinal'] for r in items]!=list(range(len(items))):raise ValueError('Noncontiguous configuration sequence')
        return tuple(scalar(r['kind'],r['value']) for r in items)
    def section(name):return {p['key']:value(p) for p in sorted(config.values(),key=lambda p:p['field_order']) if p['section']==name}
    plugin=section('plugin');extractor=section('extractor');scope=section('scope')
    scopes=[r['parameters'] for r in rows if r['code']=='SDR_SCOPE']
    for part in ['directories','explicit_files']:
        scope[part]=tuple(Path(p['path']) for p in sorted(scopes,key=lambda p:p['ordinal']) if p['part']==part)
    scope={k:scope[k] for k in ['directories','explicit_files','include_tests','broad_repo_scope']}
    metadata={r['parameters']['key']:r['parameters']['value'] for r in rows if r['code']=='DL_META'}
    root=Path(metadata['root'])
    files=sorted((r['parameters'] for r in rows if r['code']=='DL_FILE'),key=lambda p:p['path_order'])
    if len({p['path'] for p in files})!=len(files):raise ValueError('Duplicate selected file')
    facts={Path(p['path']):({extractor['extractor_id']:[]} if p['fact_present'] else {}) for p in files}
    events=sorted((r['parameters'] for r in rows if r['code']=='SDR_FACT'),
        key=lambda p:(p['path_order'],p['line'],p['code_value'],p['expression'],p['visit_order'],p['layer']))
    findings=[]
    for p in events:
        item=dict(code=p['code_value'],path=p['path'],line=p['line'],scope=p['scope'],expression=p['expression'],message=p['message'],occurrence=p['occurrence'])
        facts[Path(p['path'])][extractor['extractor_id']].append(item)
        findings.append(module.Finding(item['code'],Path(item['path']),item['line'],item['scope'],item['expression'],item['message'],item['occurrence']))
    findings.sort(key=lambda f:(f.path.as_posix(),f.line,f.code))
    reports=sorted((r['parameters'] for r in rows if r['code']=='SDR_REPORT'),
        key=lambda p:(p['phase'],p['path'],p['line'],p['code_value'],p['expression'],p['visit_order'],p['layer']))
    violations=[module.StructuredViolation(plugin['name'],p['code_value'],root/p['path'],p['line'],p['message'],metadata['findingSeverity']) for p in reports]
    output=sorted((r['parameters'] for r in rows if r['code']=='SDR_OUTPUT'),key=lambda p:p['ordinal'])
    stdout=''.join(r['text'] for r in output if r['channel']=='stdout')
    stderr=''.join(r['text'] for r in output if r['channel']=='stderr')
    inventory=tuple(p['value'] for p in sorted((r['parameters'] for r in rows if r['code']=='SDR_INVENTORY'),key=lambda p:p['ordinal']))
    return dict(selectedFiles=[p['path'] for p in files],facts=canonical(facts,root),findings=canonical(tuple(findings),root),
        violations=canonical(violations,root),plugin=canonical(dict(**plugin,file_scope=scope,required_facts=(extractor,)),root),
        inventory=canonical(inventory),stdout=stdout.replace(str(root),'$ROOT'),stderr=stderr.replace(str(root),'$ROOT'))


def execute_case(engine,rule,files,options,mode):
    from replay import execute_workspace,_execute
    if mode=='workspace':return execute_workspace(engine,rule,files,options)
    if mode!='snapshot':raise ValueError('Unknown execution mode')
    def buffered(root):
        # Directory entries describe the physical fixture. A snapshot contains
        # file buffers; every file enters it, including files outside rule scope.
        return dict(options,inputFiles=[f['path'] for f in files if (root/f['path']).is_file()])
    return _execute(engine,rule,files,buffered,None,'corpus')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command',choices=['source','native'],nargs='?',default='source')
    parser.add_argument('--source',type=Path,required=True);parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--engine',type=Path);parser.add_argument('--baseline',type=Path)
    parser.add_argument('--mode',choices=['snapshot','workspace'],default='workspace')
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1)
    args=parser.parse_args();source=args.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    if not 0<=args.part<args.parts:parser.error('Invalid shard')
    path=source/'scripts/lint_serverplan_dispatch_architecture.py'
    compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':scripts/lint_serverplan_dispatch_architecture.py']))
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['migration.py','dto.py','replay.py']]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths}
    corpus=cases(module);rows=[];fallbacks=[]
    if args.command=='native':
        baseline=read_json(args.baseline);compare(baseline['sourcePin'],PIN);compare(baseline['passed'],True)
        compare(baseline['sourceSha256'],sha(path.read_bytes()));compare([r['input'] for r in baseline['cases']],corpus)
        from data_layer_audit.native_guard import codes
        original_codes=codes({'serverplan_dispatch_architecture':module})
        def guard(frame,event,_arg):
            if event=='call' and frame.f_code in original_codes and not original_codes[frame.f_code][2]:
                fallbacks.append(frame.f_code.co_qualname);raise AssertionError('Original semantics executed: '+frame.f_code.co_qualname)
    for i,case in enumerate(corpus):
        if i%args.parts!=args.part:continue
        if args.command=='source':row=dict(input=case,expected=observe(module,case))
        else:
            row=dict(input=case,expected=baseline['cases'][i]['expected'],passed=False)
            sys.setprofile(guard)
            try:
                options=dict(entryPoint='_production_contract')
                if 'signature' in case:options['dispatchSignature']=case['signature']
                row['native']=execute_case(args.engine.absolute(),ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr',case['files'],options,args.mode)
                row['actual']=decode(module,row['native']);row['passed']=row['actual']==row['expected']
            except Exception as error:row['error']=dict(type=type(error).__name__,detail=str(error))
            finally:sys.setprofile(None)
        rows.append(row)
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report=dict(format='rerail.dispatch-repository-contract/v1',oracle=args.command=='source',passed=all(r.get('passed',True) for r in rows) and not fallbacks,admitted=False,
        sourcePin=PIN,sourceSha256=sha(path.read_bytes()),harnessFiles=hashes,cases=rows,mode=args.mode,part=args.part,parts=args.parts,semanticFallbacks=fallbacks)
    if args.command=='native':report.update(engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()))
    write_json(args.out,report);print(dict(cases=len(rows),positives=sum(bool(c['expected']['violations']) for c in rows),passed=report['passed']))
    raise SystemExit(0 if report['passed'] else 1)


if __name__=='__main__':main()
