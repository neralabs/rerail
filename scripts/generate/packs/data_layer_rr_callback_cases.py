"""Build callback-wrapper inputs; the original linter records all expected values."""
import argparse,json
from pathlib import Path
from data_layer_rr_text import module


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ['source','destination','reporters']:parser.add_argument('--'+key,type=Path,required=True)
    args=parser.parse_args();original=module(args.source.absolute(),'transaction_callback_wrappers');api=[];reports=[]
    path='l5200_vector_index/example.py'
    def add(identity,text,file=path,raw=None,override=None,report=True,relative=False):
        values=dict(path=dict(path=file,**(dict(relative=True) if relative else {})),source=text if raw is None else raw,tree=dict(pythonAst=text,mode='exec'))
        if override is not None:values['check_inline']=override
        api.append(dict(id=identity,check='transaction_callback_wrappers',entry='collector',files=[],arguments=values))
        if report:reports.append(dict(id=identity,check='transaction_callback_wrappers',files=[dict(path=file,source=text)]))
    def helper(annotation='Callable[[ReadTransaction], T]',call='conn.read(fn=fn)',params=None,name='helper'):
        return f'def {name}({params or "conn, fn: "+annotation}):\n    return {call}\n'
    for method in sorted(original.TRANSACTION_ENTRYPOINTS):add('entry-'+method,helper(call=f'conn.{method}(fn=fn)'))
    for tag,ann in [('bare','Callable'),('alias','CallbackType'),('qualified','typing.Callable[[ReadTransaction], T]'),
      ('quoted','"Callable[[ReadTransaction], T]"'),('callable-name','CallableTransaction'),('transaction-only','ReadTransaction'),
      ('lower-case','callable[[ReadTransaction], T]'),('substring','NotCallableTransaction'),('space-literal','"Callable Transaction"'),
      ('unicode','Ｃａｌｌａｂｌｅ[[ＲｅａｄＴｒａｎｓａｃｔｉｏｎ], T]'),('dict','dict["Callable", "Transaction"]')]:
        add('annotation-'+tag,helper(ann))
    for tag,params in [('posonly','fn: Callable[[ReadTransaction], T], /, conn'),('kwonly','conn, *, fn: Callable[[ReadTransaction], T]'),
      ('vararg','conn, *fn: Callable[[ReadTransaction], T]'),('kwarg','conn, **fn: Callable[[ReadTransaction], T]'),
      ('separate-annotations','conn: Callable, fn: Transaction'),('none','conn, fn')]:add('parameter-'+tag,helper(params=params))
    for tag,call in [('bare','read(fn=fn)'),('not-entry','conn.execute(fn=fn)'),('parentheses','((conn.read))(fn=fn)'),
      ('receiver-call','factory().read(fn=fn)'),('missing-callback','conn.read()')]:add('callee-'+tag,helper(call=call))
    for cls in ['SyncDatabaseConnection','SyncVersionedDatabaseConnection','AsyncDatabaseConnection','Other']:
        for method in ['read','versioned_read_async','ordinary']:
            text='class '+cls+':\n    '+helper(name=method).replace('\n','\n    ').rstrip()+'\n'
            add('boundary-'+cls+'-'+method,text)
    for tag,text in [
      ('class-qualname','class Outer:\n    class Inner:\n        '+helper().replace('\n','\n        ').rstrip()+'\n'),
      ('nested-qualname','def outer():\n    '+helper().replace('\n','\n    ').rstrip()+'\n'),
      ('class-in-function','def outer():\n    class C:\n        '+helper().replace('\n','\n        ').rstrip()+'\n'),
      ('boundary-nested','class SyncDatabaseConnection:\n    def outer(self):\n        '+helper(name='read').replace('\n','\n        ').rstrip()+'\n'),
      ('postorder','def outer(conn, fn: Callable[[ReadTransaction], T]):\n    '+helper(name='inner').replace('\n','\n    ').rstrip()+'\n    return conn.read(fn=fn)\n')]:add(tag,text)
    def named(body='return txn.versioned_read(domain)',call='conn.read(fn=worker)',definition='def worker(txn)',outer_params='conn'):
        return f'def process({outer_params}):\n    {definition}:\n        '+body.replace('\n','\n        ')+f'\n    return {call}\n'
    for method in sorted(original.REENTRANT_VERSIONED_ENTRYPOINTS):
        add('named-'+method,named(f'return txn.{method}(domain)'))
        add('lambda-'+method,f'def process(conn):\n    return conn.read(fn=lambda txn: txn.{method}(domain))\n')
    for tag,body in [('getattr','return getattr(txn, "versioned_read", None)'),('default-arg','return getattr(txn, "other", "versioned_read")'),
      ('first-arg','return getattr("versioned_read", "other")'),('keyword','return getattr(txn, name="versioned_read")'),
      ('attribute-getattr','return owner.getattr(txn, "versioned_read")'),('concat','return getattr(txn, "versioned_" + "read")'),
      ('adjacent','return getattr(txn, "versioned_" "read")'),('escaped','return getattr(txn, "versioned_\\u0072ead")'),
      ('bytes','return getattr(txn, b"versioned_read")'),('f-string','return getattr(txn, f"versioned_read")'),
      ('starred','return getattr(txn, *["versioned_read"])'),('plain-read','return txn.read(domain)')]:add('reentry-'+tag,named(body))
    for tag,call in [('position','conn.read(0, worker)'),('wrong-position','conn.read(worker)'),('unresolved','conn.read(fn=unknown)'),
      ('qualified','conn.read(fn=owner.worker)'),('keyword-wins','conn.read(0, worker, fn=other)'),
      ('starred','conn.read(*args, worker)'),('no-callback','conn.read()')]:add('callback-'+tag,named(call=call))
    for tag,text in [
      ('defined-after','def process(conn):\n    conn.read(fn=worker)\n    def worker(txn):\n        return txn.versioned_read(domain)\n'),
      ('duplicate-functions','def process(conn):\n    def worker(txn):\n        return txn.versioned_read(domain)\n    def worker(txn):\n        return None\n    conn.read(fn=worker)\n'),
      ('grandchild-only','def process(conn):\n    def worker(txn):\n        def grandchild():\n            return txn.versioned_read(domain)\n    conn.read(fn=worker)\n'),
      ('lambda-nested','def process(conn):\n    return conn.read(fn=lambda txn: (lambda: txn.versioned_read(domain)))\n'),
      ('lambda-elsewhere','def process(conn):\n    other = lambda txn: txn.versioned_read(domain)\n    return conn.read(fn=lambda t: t)\n'),
      ('nested-class','def process(conn):\n    class C:\n        def worker(txn):\n            return txn.versioned_read(domain)\n    return conn.read(fn=worker)\n'),
      ('two-reasons',named(outer_params='conn, fn: Callable[[ReadTransaction], T]')),
      ('module-call','conn.read(fn=lambda txn: txn.versioned_read(domain))\n'),
      ('two-parents','def first(conn):\n    def worker(txn):\n        return txn.versioned_read(domain)\n    conn.read(fn=worker)\ndef second(conn):\n    return conn.read(fn=worker)\n'),
      ('decorator-call','@conn.read(fn=lambda txn: txn.versioned_read(domain))\ndef process(conn):\n    pass\n'),
      ('default-call','def process(conn=conn.read(fn=lambda txn: txn.versioned_read(domain))):\n    pass\n'),
      ('nested-decorator',named(definition='@txn.versioned_read(domain)\n    def worker(txn)',body='return None')),
      ('unicode-raw-disabled','def process(conn):\n    return conn.read(fn=lambda txn: txn.ｖｅｒｓｉｏｎｅｄ＿ｒｅａｄ(domain))\n'),
      ('unicode-raw-enabled','# versioned_read\ndef process(conn):\n    return conn.read(fn=lambda txn: txn.ｖｅｒｓｉｏｎｅｄ＿ｒｅａｄ(domain))\n')]:add(tag,text)
    reentrant=named()
    for tag,raw,override in [('source-disabled','pass\n',None),('forced-enabled','pass\n',True),('forced-disabled',reentrant,False)]:
        add(tag,reentrant,raw=raw,override=override,report=False)
    for allowed in sorted(original.ALLOWED_RELATIVE_PATHS):
        add('allowed-relative-'+allowed,helper(),file=allowed,relative=True,report=False)
        add('outside-allowed-'+allowed,helper(),file=allowed)
    for where in ['impl/example.py','docs/example.py','conform/example.py','l5200_vector_index/tests/example.py','l5200_vector_index/test_example.py','data_layer/example.py','_internal/example.py']:
        reports.append(dict(id='scope-'+where,check='transaction_callback_wrappers',files=[dict(path=where,source=helper())]))
    for tag,text in [('helper',helper()),('inline',reentrant),('clean','pass\n'),('syntax','# .read\ndef f(:\n'),('null','# Callable\n\x00\n')]:
        api.append(dict(id='scan-file-'+tag,check='transaction_callback_wrappers',entry='scan_file',files=[dict(path='example.py',source=text)],arguments=dict(file_path=dict(path='example.py'))))
    for tag,provided,disk in [('provided-only','Callable', 'pass\n'),('disk-only','pass\n',helper()),('same',helper(),helper())]:
        api.append(dict(id='scan-source-'+tag,check='transaction_callback_wrappers',entry='scan_source',files=[dict(path='example.py',source=disk)],
            arguments=dict(file_path=dict(path='example.py'),source=provided,root=dict(path='.'))))
    api.append(dict(id='scan-file-missing',check='transaction_callback_wrappers',entry='scan_file',files=[],arguments=dict(file_path=dict(path='absent.py'))))
    api.append(dict(id='scan-file-decode',check='transaction_callback_wrappers',entry='scan_file',files=[dict(path='bad.py',bytesHex='fffe')],arguments=dict(file_path=dict(path='bad.py'))))
    for folder in ['tests','test']:
        file=folder+'/example.py'
        api.append(dict(id='scan-file-'+folder+'-folder',check='transaction_callback_wrappers',entry='scan_file',files=[dict(path=file,source=helper())],arguments=dict(file_path=dict(path=file))))
    api.append(dict(id='scan-file-relative',check='transaction_callback_wrappers',entry='scan_file',cwd='fixture',
        files=[dict(path='example.py',source=helper())],arguments=dict(file_path=dict(path='example.py',relative=True))))
    api.append(dict(id='scan-file-non-python',check='transaction_callback_wrappers',entry='scan_file',
        files=[dict(path='example.txt',source=helper())],arguments=dict(file_path=dict(path='example.txt'))))
    api.append(dict(id='scan-source-relative-root',check='transaction_callback_wrappers',entry='scan_source',cwd='fixture',
        files=[dict(path='example.py',source=helper())],arguments=dict(file_path=dict(path='example.py',relative=True),root=dict(path='.'),source='Callable')))
    api.append(dict(id='scan-source-root-named-tests',check='transaction_callback_wrappers',entry='scan_source',
        files=[dict(path='tests/example.py',source=helper())],arguments=dict(file_path=dict(path='tests/example.py'),root=dict(path='tests'),source='Callable')))
    for out,cases in [(args.destination,api),(args.reporters,reports)]:out.write_text(json.dumps(dict(cases=cases),indent=2)+'\n')
    print(len(api),'API cases;',len(reports),'reporter cases')


if __name__=='__main__':main()
