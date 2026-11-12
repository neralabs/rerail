"""Record complete boundary metadata, registered collectors, raw facts, and helper values."""
import ast
from dataclasses import asdict,fields
from pathlib import Path
from types import SimpleNamespace
from contract_runner import main
from dto import canonical
from async_pattern_components_contract import hydrate

CHECK='async_sync_boundary'
CONSTANTS=['SOURCE_TRIGGERS','ALLOWLIST','INFORMATIONAL_DIRS','CATEGORY_LABELS']


def cases(_module):
    hit=dict(file='impl/a.py',line=4,col=2,category='CUSTOM',function_name='work',call_text='call()',is_async_ctx=True)
    rows=[dict(kind='metadata',id='metadata')]
    texts=dict(sync='def work():\n    return asyncio.run(fetch())\n',async_='async def work():\n    return await asyncio.to_thread(fetch)\n',executor='async def work(loop):\n    return await loop.run_in_executor(None,fetch)\n',
        clean='pass\n',trigger_only='# to_thread\n',invalid='def broken(:\n',nested='async def outer():\n    def inner():\n        asyncio.run(fetch())\n    await to_thread(work)\n',
        lambda_='async def outer():\n    f = lambda: to_thread(work)\n',class_='async def outer():\n    class Inner:\n        task = to_thread(work)\n',
        module='asyncio.run(fetch())\n',duplicate='asyncio.run(fetch())\nasyncio.run(fetch())\n')
    for name,text in texts.items():rows.append(dict(kind='extract',id='extract-'+name,path='impl/a.py',source=text))
    for i,value in enumerate([None,{},'bad',0,[],[hit],hit,*[{k:v for k,v in hit.items() if k!=missing} for missing in hit]]):rows.append(dict(kind='load',id=f'load-shape-{i}',raw=value))
    values=[None,False,True,0,-1,2**100,1.9,'4','  -４  ','bad',[],{},[1],dict(key=1),'$NAN','$INF','$NEG_INF']
    for key in hit:
        for i,value in enumerate(values):rows.append(dict(kind='load',id=f'load-{key}-{i}',raw=dict(hit,**{key:value})))
    for i,path in enumerate(['a.py','a/../b.py','/external.py','.','//network/a.py']):rows.append(dict(kind='project',id=f'project-{i}',value=dict(hit,file=path,line=[0,True,-1,2**100,4][i])))
    bundles=[[],[dict(value=None)],[dict(value={})],[dict(value='bad')],[dict(value=[None,[],{},hit])],[dict(value=[hit,hit])],
        [dict(value=[dict(hit,file=p)]) for p in ['z.py','a.py','a/z.py','a-.py','/external.py']],
        [dict(value=[dict(hit,file='impl/sqlite/native_async_wrapper.py')])],
        [dict(value=[dict(hit,file='tests/a.py')])],
        [dict(value=[dict(hit,category='SYNC_WRAPS_ASYNC'),dict(hit,category='ASYNC_WRAPS_SYNC'),dict(hit,category='EXECUTOR_ESCAPE')])]]
    for i,group in enumerate(bundles):rows.append(dict(kind='report',id=f'report-{i}',bundles=[dict(path='ignored.py',**b) for b in group]))
    for i,expr in enumerate(['asyncio.run(x)','to_thread(x)','loop.run_in_executor(x)','pkg.inner.work(x)','make().x.y()','f()[0](x)','(lambda: None)()','(a or b)()']):rows.append(dict(kind='name',id=f'name-{i}',expression=expr))
    for i,stack in enumerate([[],[['outer',True]],[['outer',True],['inner',False]],[['<module>',False]]]):rows.append(dict(kind='stack',id=f'stack-{i}',stack=stack))
    rows.append(dict(kind='categories',id='categories',violations=[dict(hit,category=c) for c in ['CUSTOM','SYNC_WRAPS_ASYNC','ASYNC_WRAPS_SYNC','EXECUTOR_ESCAPE','SYNC_WRAPS_ASYNC']]))
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir()
    try:
        kind=case['kind']
        if kind=='metadata':
            plugin=module.async_sync_boundary_plugin(root)
            scope=asdict(plugin.file_scope);scope['directories']=[str(p.relative_to(root)) for p in scope['directories']]
            value=dict(constants={n:getattr(module,n) for n in CONSTANTS},extractor=asdict(module.ASYNC_SYNC_BOUNDARY_EXTRACTOR),plugin={n:getattr(plugin,n) for n in ['name','enforcement','description']},scope=scope)
        elif kind=='extract':
            from scripts.lint_lib.fact_engine import materialize_source_facts
            spec=module.ASYNC_SYNC_BOUNDARY_EXTRACTOR
            target=materialize_source_facts(path=root/case['path'],source=case['source'],specs=(spec,),extractors=module.async_sync_boundary_fact_extractors(root))
            value=dict(facts=target.facts,errors=target.errors)
        elif kind=='load':value=module._violation_from_fact(hydrate(case['raw']))
        elif kind=='project':value=module._violation_to_fact(module.Violation(**case['value']))
        elif kind=='report':
            family=module.ASYNC_SYNC_BOUNDARY_EXTRACTOR.extractor_id
            bundles={str(i):SimpleNamespace(path=b['path'],facts={family:hydrate(b['value'])}) for i,b in enumerate(case['bundles'])}
            value=module.async_sync_boundary_plugin(root).reporter(SimpleNamespace(root=root,files=bundles))
        elif kind=='name':value=module._get_call_name(ast.parse(case['expression'],mode='eval').body)
        elif kind=='stack':
            owner=module._BoundaryCollector('impl/a.py');owner._func_stack=[tuple(v) for v in case['stack']];value=dict(current=owner._current_function(),stack=owner._func_stack,file=owner.file_path,violations=owner.violations)
        elif kind=='categories':
            result=module.LintResult([module.Violation(**v) for v in case['violations']],3)
            value={name:getattr(result,name) for name in ['sync_wraps_async','async_wraps_sync','executor_escape','files_checked']}
        else:raise ValueError('Unknown boundary component')
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from boundary_port import install as existing_install
    from boundary_port import scan
    import inspect
    existing_install(backend,module)
    for entry in ['scan_tree','_get_call_name','_is_allowlisted','_is_informational']:
        signature=inspect.signature(getattr(module,entry))
        def bind(entry,signature):
            def run(*args,**kwargs):
                bound=signature.bind(*args,**kwargs);bound.apply_defaults();return scan(backend,entry,module,bound.arguments)
            return run
        backend.replace(module,entry,bind(entry,signature))


if __name__=='__main__':main(CHECK,'rerail.boundary-components/v1',cases,exercise,install,['boundary_components_contract.py','async_pattern_components_contract.py'])
