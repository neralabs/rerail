"""Record standalone boundary rendering, classifications, ordering and context windows."""
import contextlib
import io
from pathlib import Path
from contract_runner import main
from dto import canonical
from replay import write_fixture
from boundary_production_contract import CHECK,BAD


def cases(_module):
    def v(file='impl/a.py',line=2,category='SYNC_WRAPS_ASYNC',name='work',async_=False):return dict(file=file,line=line,col=4,category=category,function_name=name,call_text='asyncio.run()',is_async_ctx=async_)
    sets=[[],[v()],[v(category='ASYNC_WRAPS_SYNC',async_=True)],[v(category='EXECUTOR_ESCAPE',async_=True)],
        [v('impl/tests/a.py')],[v('impl/sqlite/native_async_wrapper.py')],
        [v(),v('impl/tests/a.py'),v('impl/sqlite/native_async_wrapper.py')],
        [v('z.py',5),v('a.py',4),v('a.py',2),v('a.py',2,category='CUSTOM')],
        [v(line=-1),v(line=0),v(line=2**100),v(line=1)],
        [v(file='impl/機能.py',name='機能',category='CUSTOM')]]
    rows=[]
    for i,values in enumerate(sets):
        for show in [False,True]:rows.append(dict(kind='results',id=f'results-{i}-{int(show)}',violations=values,checked=7,show_allowed=show))
    rows.extend([dict(kind='violations',id='violations-order',violations=sets[7]),
        dict(kind='results',id='context',violations=[v()],checked=1,context=True,files=[dict(path='impl/a.py',source=BAD)]),
        dict(kind='results',id='context-no-root',violations=[v()],checked=1,context=True,no_root=True),
        dict(kind='violations',id='context-missing',violations=[v()],context=True),
        dict(kind='violations',id='context-duplicate',violations=[v(),v()],context=True,files=[dict(path='impl/a.py',source=BAD)])])
    for i,(line,radius) in enumerate([(1,2),(3,1),(0,2),(-2,2),(3,-2),(100,2)]):rows.append(dict(kind='context',id=f'window-{i}',line=line,radius=radius,files=[dict(path='impl/a.py',source='one\ntwo\nthree\nfour\nfive\n')]))
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case.get('files',[]));out=io.StringIO();err=io.StringIO()
    with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
        try:
            if case['kind']=='context':value=module._show_source_context(root/'impl/a.py',case['line'],case['radius'])
            else:
                rows=[module.Violation(**v) for v in case['violations']];options=dict(show_context=case.get('context',False),root=None if case.get('no_root') else root)
                if case['kind']=='results':value=module.print_results(module.LintResult(rows,case['checked']),show_allowlisted=case.get('show_allowed',False),**options)
                else:value=module._print_violations(rows,**options)
            result=dict(value=canonical(value,scratch))
        except Exception as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    return dict(result,stdout=out.getvalue(),stderr=err.getvalue())


def install(backend,module,detectors=False):
    from boundary_components_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.boundary-display/v1',cases,exercise,install,['boundary_display_contract.py','boundary_production_contract.py'])
