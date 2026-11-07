"""Recordable boundary contracts. All expected values come from the pinned source."""
import argparse,json
from pathlib import Path
from data_layer_rr_text import module


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True)
    parser.add_argument('--destination',type=Path,required=True)
    parser.add_argument('--reporters',type=Path,required=True)
    args=parser.parse_args();original=module(args.source.resolve(),'async_sync_boundary');api=[];reports=[]
    def add(identity,code,path='impl/module.py',paths=None,report=True):
        files=[dict(path=path,source=code)]
        api.append(dict(id=identity,check='async_sync_boundary',entry='scan_tree',files=files,
            arguments=dict(root=dict(path='.'),paths=paths if paths is not None else [path])))
        if report:reports.append(dict(id=identity,check='async_sync_boundary',files=files))
    bodies={
      'sync-run':'def f():\n    return asyncio.run(fetch())\n',
      'sync-threadsafe':'def f():\n    return asyncio.run_coroutine_threadsafe(fetch(), loop)\n',
      'async-thread':'async def f():\n    return asyncio.to_thread(fetch)\n',
      'async-bare-thread':'async def f():\n    return to_thread(fetch)\n',
      'async-executor':'async def f():\n    return loop.run_in_executor(None, fetch)\n',
      'async-bare-executor':'async def f():\n    return run_in_executor(None, fetch)\n',
      'executor-suffix':'async def f():\n    return object.not_run_in_executor(None, fetch)\n',
      'executor-call-receiver':'async def f():\n    return get_loop().run_in_executor(None, fetch)\n',
      'executor-subscript-receiver':'async def f():\n    return loops[0].run_in_executor(None, fetch)\n',
      'partial-dotted-name':'def f():\n    return factory().asyncio.run(fetch())\n',
      'module-run':'asyncio.run(fetch())\n',
      'module-thread':'asyncio.to_thread(fetch)\n',
      'module-executor':'loop.run_in_executor(None, fetch)\n',
      'sync-thread':'def f():\n    return asyncio.to_thread(fetch)\n',
      'async-run':'async def f():\n    return asyncio.run(fetch())\n',
      'sync-executor':'def f():\n    return loop.run_in_executor(None, fetch)\n',
      'async-lambda':'async def f():\n    return lambda: asyncio.to_thread(fetch)\n',
      'sync-lambda':'def f():\n    return lambda: asyncio.run(fetch())\n',
      'module-lambda':'x = lambda: asyncio.run(fetch())\n',
      'nested-sync':'async def outer():\n    def inner():\n        return asyncio.run(fetch())\n    return asyncio.to_thread(fetch)\n',
      'nested-async':'def outer():\n    async def inner():\n        return asyncio.to_thread(fetch)\n    return asyncio.run(fetch())\n',
      'class-in-async':'async def outer():\n    class Nested:\n        x = asyncio.to_thread(fetch)\n',
      'async-default':'async def f(x=asyncio.to_thread(fetch)):\n    pass\n',
      'sync-default':'def f(x=asyncio.run(fetch())):\n    pass\n',
      'async-decorator':'@deco(asyncio.to_thread(fetch))\nasync def f():\n    pass\n',
      'async-decorator-run':'@deco(asyncio.run(fetch()))\nasync def f():\n    pass\n',
      'sync-decorator':'@deco(asyncio.run(fetch()))\ndef f():\n    pass\n',
      'async-annotation':'async def f(x: asyncio.to_thread(fetch)):\n    pass\n',
      'sync-return-annotation':'def f() -> asyncio.run(fetch()):\n    pass\n',
      'parenthesized':'# asyncio.run\ndef f():\n    return ((asyncio) . run)(fetch())\n',
      'raw-trigger-absent':'def f():\n    return asyncio . run(fetch())\n',
      'qualified-clean':'def f():\n    return other.asyncio.run(fetch())\n',
      'alias-clean':'import asyncio as aio\ndef f():\n    return aio.run(fetch())\n',
      'nfkc':'# asyncio.run\ndef ｆ():\n    return ａｓｙｎｃｉｏ.run(fetch())\n',
      'unicode-columns':'async def f(): π = 0; return asyncio.to_thread(fetch)\n',
      'duplicates':'async def f():\n    return (to_thread(a), to_thread(a), loop.run_in_executor(None, a))\n',
      'parse-error':'async def broken(: # to_thread\n',
      'clean-await':'async def f():\n    return await fetch()\n',
      'source-comment':'# asyncio.run to_thread run_in_executor\nx = 1\n',
    }
    for identity,code in bodies.items():add(identity,code)
    bridge='async def bridge():\n    return asyncio.to_thread(fetch)\n'
    for path in ['api/input.py','impl/input.py','conform/input.py','public/input.py','scripts/input.py',
      'impl/tests/input.py','impl/test/input.py','impl/test_input.py','impl/not_tests/input.py',
      'impl/tmp/input.py','impl/venv/input.py','impl/.venv/input.py','impl/__pycache__/input.py']:
        add('scope-'+path,bridge,path)
    for index,(pattern,fn) in enumerate(sorted(original.ALLOWLIST)):
        path='impl/prefix/'+pattern.replace('*','input.py')
        name='bridge' if fn=='*' else fn
        code=f'async def {name}():\n    return asyncio.to_thread(fetch)\n'
        add(f'allow-{index}',code,path)
        if fn!='*':add(f'allow-wrong-function-{index}',code.replace(name+'(',name+'_other('),path)
    for index,part in enumerate(original.INFORMATIONAL_DIRS):
        add(f'informational-{index}',bridge,'impl/prefix/'+part+'input.py')
    for identity,files,paths in [
      ('empty',[],None),('empty-paths',[],[]),('missing-directory',[],['missing']),
      ('non-python-explicit',[dict(path='impl/source.txt',source=bodies['sync-run'])],['impl/source.txt']),
      ('overlap',[dict(path='impl/b.py',source=bodies['async-thread']),dict(path='impl/a.py',source=bodies['sync-run'])],['impl','impl/a.py']),
      ('default-inventory',[dict(path='impl/b.py',source=bodies['async-thread']),dict(path='api/a.py',source=bodies['sync-run']),dict(path='public/no.py',source=bodies['sync-run'])],None),
      ('bad-encoding',[dict(path='impl/bad.py',bytesHex='fffe')],['impl']),
    ]:
        api.append(dict(id=identity,check='async_sync_boundary',entry='scan_tree',files=files,
            arguments=dict(root=dict(path='.'),paths=paths)))
    for index,(pattern,fn) in enumerate(sorted(original.ALLOWLIST)):
        for tag,path,name in [('match','prefix/'+pattern.replace('*','file.py'),'bridge' if fn=='*' else fn),
                              ('miss','other.py','other')]:
            api.append(dict(id=f'helper-allow-{index}-{tag}',check='async_sync_boundary',entry='_is_allowlisted',files=[],
                arguments=dict(file_rel=path,function_name=name)))
    for index,path in enumerate(['impl/input.py','impl/test/input.py','impl/tests/input.py','prefix/scripts/input.py','not_tests/input.py','tests/input.py','tests']):
        api.append(dict(id=f'helper-info-{index}',check='async_sync_boundary',entry='_is_informational',files=[],arguments=dict(file_rel=path)))
    for index,expression in enumerate(['asyncio.run(x)','to_thread(f)','loop.run_in_executor(None,f)','factory().asyncio.run(x)',
        'factory().run_in_executor(None,f)','items[0].run_in_executor(None,f)','(lambda: 0)()','factory()()',
        '((asyncio).run)(f)','other.asyncio.run(x)','object.not_run_in_executor(f)']):
        api.append(dict(id=f'helper-name-{index}',check='async_sync_boundary',entry='_get_call_name',files=[],
            arguments=dict(node=dict(pythonAst=expression,mode='eval'))))
    args.destination.write_text(json.dumps(dict(cases=api),indent=2)+'\n')
    args.reporters.write_text(json.dumps(dict(cases=reports),indent=2)+'\n')
    print(len(api),'API cases;',len(reports),'reporter cases')


if __name__=='__main__':main()
