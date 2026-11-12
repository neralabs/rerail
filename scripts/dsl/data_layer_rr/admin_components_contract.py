"""Record complete admin metadata, registered extraction, projection and supplied facts."""
from dataclasses import asdict,fields
import contextlib
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from async_pattern_components_contract import hydrate

CHECK='admin_async_purity'


def cases(_module):
    rows=[dict(kind='metadata',id='metadata')]
    texts=dict(
        all='asyncio.to_thread(work); loop.run_in_executor(None,work); disable_sync_in_async_check()\n',
        clean='pass\n',comment='# asyncio.to_thread(work)\n',
        inline_comment='pass # run_in_executor(work)\n',
        string='value = "asyncio.to_thread(work)"\n',
        docstring='"""\nasyncio.to_thread(work)\n"""\nrun_in_executor(work)\n',
        single_quotes="'''\nrun_in_executor(work)\n'''\ndisable_sync_in_async_check()\n",
        mixed_quotes='"""\nasyncio.to_thread(work)\n\'\'\'\nrun_in_executor(work)\n',
        same_line='"""ignored"""; asyncio.to_thread(work)\nrun_in_executor(work)\n',
        odd_quotes='"""asyncio.to_thread(work)\nrun_in_executor(work)\n',
        syntax_error='def broken(: asyncio.to_thread(work)\n',
        duplicates='run_in_executor(work)\nrun_in_executor(work)\n',
        repeated_same_line='run_in_executor(a); run_in_executor(b)\n',
        boundaries='arun_in_executor(work)\nrun_in_executor_async(work)\nobject.run_in_executor(work)\n',
        unicode='☃run_in_executor(work)\né_run_in_executor(work)\nrun_in_executor☃(work)\n',
        splitlines='asyncio.to_thread(work)\x85run_in_executor(work)\u2028disable_sync_in_async_check()\n',
        whitespace='\u2003# run_in_executor(work)\nrun_in_executor(work)\t\u3000\n',
    )
    for name,source in texts.items():
        rows.append(dict(kind='extract',id='extract-'+name,path='l8000_admin/tools/a.py',source=source))
    for name,path,allowed in [
        ('allowlisted','l8000_admin/tools/a.py',['l8000_admin/tools/a.py']),
        ('other-allowlist','l8000_admin/tools/a.py',['a.py']),
        ('tests','l8000_admin/tools/tests/a.py',[]),
        ('test-prefix','l8000_admin/tools/latest_a.py',[]),
        ('cache','l8000_admin/tools/__pycache__/a.py',[]),
        ('outside','other/a.py',[]),
        ('non-python','l8000_admin/tools/a.txt',[]),
    ]:
        rows.append(dict(kind='extract',id='extract-'+name,path=path,source=texts['all'],allowed=allowed))
    hit=dict(file='l8000_admin/tools/a.py',line=4,pattern='caller pattern',text='caller text')
    for i,value in enumerate([hit,dict(hit,file='a/../b.py'),dict(hit,file='/external.py'),dict(hit,line=True),dict(hit,line=2**100),dict(hit,line=None),dict(hit,text=['caller',1]),dict(hit,pattern={'caller':True})]):
        rows.append(dict(kind='project',id=f'project-{i}',value=value))
    payloads=[None,{},[],0,'bad',dict(violations=None),dict(violations={}),dict(violations='bad'),dict(violations=[None,[],0,'bad']),
        dict(violations=[hit]),dict(violations=[hit,hit])]
    payloads += [dict(violations=[{k:v for k,v in hit.items() if k!=missing}]) for missing in hit]
    values=[None,False,True,0,-1,2**100,-2**100,1.9,'4','  -４  ','bad',[],{},[1],dict(key=1),'$NAN','$INF','$NEG_INF']
    for key in hit:
        payloads += [dict(violations=[dict(hit,**{key:value})]) for value in values]
    for i,payload in enumerate(payloads):
        rows.append(dict(kind='report',id=f'report-{i}',bundles=[dict(path='ignored.py',value=payload)]))
    rows += [
        dict(kind='report',id='report-order',bundles=[dict(path='ignored.py',value=dict(violations=[dict(hit,file=p,line=l,text=t)])) for p,l,t in [('z.py',2,'z'),('a.py',1,'a'),('a.py',1,'a')]]),
        dict(kind='report',id='report-missing-family',bundles=[dict(path='a.py',facts={})]),
        dict(kind='report',id='report-other-family',bundles=[dict(path='a.py',facts={'other':dict(violations=[hit])})]),
        dict(kind='report',id='report-unfiltered',allowed=[hit['file']],bundles=[dict(path='ignored.py',value=dict(violations=[hit,dict(hit,file='tools/tests/a.py')]))]),
        dict(kind='report',id='report-lexical-line-order',bundles=[dict(path='ignored.py',value=dict(violations=[dict(hit,line=n) for n in [-10,-2,0,1,10,2,20,2**100,-2**100]]))]),
        dict(kind='report',id='report-message-order',bundles=[dict(path='ignored.py',value=dict(violations=[dict(hit,text=t) for t in ['z','a','a','☃']]))]),
        dict(kind='report',id='report-external-path-order',bundles=[dict(path='ignored.py',value=dict(violations=[dict(hit,file=p) for p in ['a/../b.py','/zzzz/external.py','//network/a.py','./a//b.py','']]))]),
        dict(kind='report',id='report-cwd-relative-order',cwdRoot=True,bundles=[dict(path='ignored.py',value=dict(violations=[dict(hit,file=p) for p in ['a.py','/external.py','','a/../b.py']]))]),
    ]
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir()
    with patch.object(module,'ROOT',root),patch.object(module,'ALLOWLISTED_FILES',case.get('allowed',[])),contextlib.chdir(root if case.get('cwdRoot') else Path.cwd()):
        try:
            kind=case['kind']
            if kind=='metadata':
                plugin=module.admin_async_purity_plugin(root)
                scope=asdict(plugin.file_scope);scope['directories']=[str(p.relative_to(root)) for p in scope['directories']]
                value=dict(constants={n:getattr(module,n) for n in ['LINTER_NAME','SCAN_DIRS','ALLOWLISTED_FILES','ALLOWLIST_SIGNATURE']},
                    patterns=[dict(pattern=p.pattern,flags=p.flags,name=name) for p,name in module.FORBIDDEN_PATTERNS],
                    skips=[dict(pattern=p.pattern,flags=p.flags) for p in module.SKIP_PATTERNS],
                    dto=[dict(name=f.name,type=str(f.type)) for f in fields(module.Violation)],
                    extractor=asdict(module.ADMIN_ASYNC_PURITY_EXTRACTOR),plugin={n:getattr(plugin,n) for n in ['name','enforcement','description']},scope=scope)
            elif kind=='extract':
                from scripts.lint_lib.fact_engine import materialize_source_facts
                spec=module.ADMIN_ASYNC_PURITY_EXTRACTOR
                result=materialize_source_facts(path=root/case['path'],source=case['source'],specs=(spec,),extractors=module.admin_async_purity_fact_extractors(root))
                value=dict(facts=result.facts,errors=result.errors)
            elif kind=='project':value=module._violation_to_fact(module.Violation(**hydrate(case['value'])))
            elif kind=='report':
                family=module.ADMIN_ASYNC_PURITY_EXTRACTOR.extractor_id
                bundles={str(i):SimpleNamespace(path=b['path'],facts=hydrate(b.get('facts',{family:b.get('value')}))) for i,b in enumerate(case['bundles'])}
                value=module.admin_async_purity_plugin(root).reporter(SimpleNamespace(root=root,files=bundles))
            else:raise ValueError('Unknown admin component')
            return dict(value=canonical(value,scratch))
        except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from admin_components_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.admin-components/v1',cases,exercise,install,['admin_components_contract.py','async_pattern_components_contract.py'])
