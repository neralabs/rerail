"""Construct independent transaction inputs; source execution records the expectations."""
import argparse,json
from pathlib import Path
from data_layer_rr_text import module


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ['source','destination','reporters']:parser.add_argument('--'+key,type=Path,required=True)
    args=parser.parse_args();original=module(args.source.absolute(),'transaction_closures');api=[];reports=[]
    def add(identity,source,mode='both',outer=(),initial=(),raw=None,report=True):
        values=dict(mode=mode,tree=dict(pythonAst=source,mode='exec'),file_path=dict(path='impl/example.py'),source=source if raw is None else raw)
        if mode=='detector':values.update(outer=list(outer),initial=list(initial))
        api.append(dict(id=identity,check='transaction_closures',entry='probe',files=[],arguments=values))
        if report and mode!='detector':reports.append(dict(id=identity,check='transaction_closures',files=[dict(path='impl/example.py',source=source)]))
    for method in sorted(original.MUTATING_METHODS):
        for tag,body in [('outer',f'items.{method}(1)'),('shadow-before',f'items = []; items.{method}(1)'),
             ('shadow-after',f'items.{method}(1); items = []'),('unknown',f'other.{method}(1)'),('chain',f'items.child.{method}(1)')]:
            add('detector-'+method+'-'+tag,body+'\n','detector',['items'],report=False)
    detector_cases=[('names','nonlocal x, y\nglobal a, b\n'),('duplicates','global x, x\n'),
      ('nested-functions','def a():\n    items.append(1)\n    def b(items):\n        items.append(2)\nitems.append(3)\n'),
      ('lambda-stop','f = lambda: items.append(1)\nitems.append(2)\n'),
      ('headers-skip','@items.append(1)\ndef f(p: items.append(2) = items.append(3)) -> items.append(4):\n    items.append(5)\n'),
      ('parameter-kinds','def f(items, /, *, other=0, **kwargs):\n    items.append(1)\n    other.append(2)\n    kwargs.update(x=1)\n'),
      ('assign-order','items = items.append(1)\n'),('walrus-order','(items := items.append(1))\n'),
      ('annotated-order','items: list[items.append(1)] = items.append(2)\n'),
      ('tuple-assign','items, other = value\nitems.append(1)\n'),('for-binding','for items in seq:\n    items.append(1)\n'),
      ('subscript','items[0] = 1\nitems[1] += 1\ndel items[2]\n'),
      ('tuple-target','items[0], other = seq\n'),('subscript-before-rhs','items[(items := [])] = items.append(1)\n'),
      ('unicode','nonlocal café, π\ncafé.append(1)\n'),('class-body','class C(items.append(1)):\n    items.append(2)\n')]
    for tag,source in detector_cases:add('detector-'+tag,source,'detector',['items','other','café'],report=False)
    add('detector-initial-locals','items.append(1)\n','detector',['items'],['items'],report=False)
    for tag,first,second in [('locals','items = []\n','items.append(1)\n'),('duplicates','items.append(1)\n','items.append(2)\n')]:
        add('detector-repeated-'+tag,first,'detector',['items'],report=False)
        api[-1]['arguments']['additionalTrees']=[dict(pythonAst=second,mode='exec')]
    for method in sorted(original.TRANSACTION_METHODS):
        for tag,args_text in [('keyword','fn=lambda t: items.append(t)'),('positional','0, lambda t: items.append(t)'),
          ('precedence','0, lambda t: items.append(t), fn=lambda t: t'),('first-pos','lambda t: items.append(t)'),
          ('spread','*args, lambda t: items.append(t)'),('keywords-first','fn=lambda t: items.append(t), **kwargs')]:
            source=f'def caller(conn):\n    items = []\n    conn.{method}({args_text})\n'
            add(method+'-'+tag,source)
    named=[('nonlocal','nonlocal items\nitems = []'),('global','global items\nitems = []'),('mutator','items.append(1)'),('subscript','items[0] = 1')]
    for tag,body in named:
        for mode in ['inline','named','both','source']:
            add('named-'+tag+'-'+mode,'def caller(conn):\n    items = []\n    def worker(t):\n        '+body.replace('\n','\n        ')+'\n    conn.write(fn=worker)\n',mode,report=mode=='both')
    variants={
      'defined-after':'def caller(conn):\n    items = []\n    conn.write(fn=worker)\n    def worker(t):\n        items.append(t)\n',
      'duplicate-last':'def caller(conn):\n    items = []\n    def worker(t):\n        items.append(t)\n    conn.write(fn=worker)\n    def worker(t):\n        pass\n',
      'repeat-calls':'def caller(conn):\n    items = []\n    def worker(t):\n        nonlocal items\n        items.append(t)\n        items[0] = t\n    conn.write(fn=worker)\n    conn.read(fn=worker)\n',
      'module-import':'import items\nconn.write(fn=lambda t: items.append(t))\n',
      'module-dotted-import':'import items.child\nconn.write(fn=lambda t: items.append(t))\n',
      'module-alias':'import package as items\nconn.write(fn=lambda t: items.append(t))\n',
      'module-from':'from .package import items\nconn.write(fn=lambda t: items.append(t))\n',
      'module-from-alias':'from package import thing as items\nconn.write(fn=lambda t: items.append(t))\n',
      'module-class':'class items: pass\nconn.write(fn=lambda t: items.append(t))\n',
      'module-function':'def items(): pass\nconn.write(fn=lambda t: items.append(t))\n',
      'module-later':'conn.write(fn=lambda t: items.append(t))\nitems = []\n',
      'nested-binding':'def caller(conn):\n    def nested():\n        items = []\n    conn.write(fn=lambda t: items.append(t))\n',
      'named-ignores-module':'items = []\ndef caller(conn):\n    def worker(t):\n        items.append(t)\n    conn.write(fn=worker)\n',
      'named-ignores-tuple':'def caller(conn):\n    items, other = values\n    def worker(t):\n        items.append(t)\n    conn.write(fn=worker)\n',
      'enclosing-argument':'def caller(conn, items):\n    conn.write(fn=lambda t: items.append(t))\n',
      'kwonly-not-local':'def caller(conn, *, items):\n    conn.write(fn=lambda t: items.append(t))\n',
      'decorator-scope':'@conn.write(fn=lambda t: items.append(t))\ndef caller(conn):\n    items = []\n',
      'default-scope':'def caller(conn=conn.write(fn=lambda t: items.append(t))):\n    items = []\n',
      'lambda-shadow':'def caller(conn):\n    items = []\n    conn.write(fn=lambda items: items.append(1))\n',
      'lambda-nested':'def caller(conn):\n    items = []\n    conn.write(fn=lambda t: (lambda: items.append(t)))\n',
      'parenthesized':'def caller(conn):\n    items = []\n    ((conn.write))(fn=((lambda t: (items.append(t)))))\n',
      'unicode-column':'def caller(conn):\n    café = []; conn.write(fn=lambda t: café.append(t))\n',
      'trigger-space':'def caller(conn):\n    items = []\n    conn.write (fn=lambda t: items.append(t))\n',
      'clean':'def caller(conn):\n    conn.write(fn=lambda t: t.append(1))\n'}
    for tag,source in variants.items():
        add(tag,source)
        add('source-'+tag,source,'source',report=False)
    # The caller's tree and raw source deliberately disagree.
    add('supplied-tree-positive',variants['repeat-calls'],'source',raw='# .write(\npass\n',report=False)
    add('supplied-tree-trigger-absent',variants['repeat-calls'],'source',raw='pass\n',report=False)
    for mode in ['inline','named','both']:
        add('supplied-function-callback-'+mode,variants['repeat-calls'],mode,report=False)
        api[-1]['arguments']['edits']=[dict(target=['body',0,'body',2,'value','keywords',0,'value'],source=['body',0,'body',1])]
    for tag,raw in [('clean','pass\n'),('trigger','# .write(\npass\n')]:
        api.append(dict(id='source-missing-tree-'+tag,check='transaction_closures',entry='probe',files=[],
            arguments=dict(mode='source',tree=None,file_path=dict(path='impl/example.py'),source=raw)))
    for tag,text in [('positive',variants['repeat-calls']),('spaced',variants['trigger-space']),('clean','pass\n'),
        ('syntax','def f(:\n'),('null','\x00\n'),('multiline','s = """a\nb"""\n')]:
        api.append(dict(id='file-'+tag,check='transaction_closures',entry='lint_file',
            files=[dict(path='example.py',source=text)],arguments=dict(file_path=dict(path='example.py'))))
    api.append(dict(id='file-decode',check='transaction_closures',entry='lint_file',
        files=[dict(path='example.py',bytesHex='fffe00')],arguments=dict(file_path=dict(path='example.py'))))
    api.append(dict(id='file-missing',check='transaction_closures',entry='lint_file',files=[],cwd='fixture',arguments=dict(file_path=dict(path='absent.py',relative=True))))
    for tag,path in [('regular','a.py'),('test','tests/test_a.py'),('tmp','tmp/a.py'),('venv','.venv/a.py'),('dotgit','.git/a.py')]:
        for excluded in [False,True]:
            api.append(dict(id='paths-'+tag+'-'+str(excluded),check='transaction_closures',entry='lint_paths',
                files=[dict(path=path,source=variants['repeat-calls'])],arguments=dict(paths=[dict(path='.')],exclude_tests=excluded)))
    for path in ['impl/tests/test_a.py','impl/test/test_a.py','impl/test_a.py','conform/a.py','docs/a.py','l7000_example/a.py','impl/tmp/a.py']:
        reports.append(dict(id='scope-'+path,check='transaction_closures',files=[dict(path=path,source=variants['repeat-calls'])]))
    reports.extend([dict(id='parse-'+str(i),check='transaction_closures',files=[dict(path='impl/bad.py',source=s)])
        for i,s in enumerate(['# .write(\ndef f(:\n','# .read(\n\x00\n'])])
    for path,cases in [(args.destination,api),(args.reporters,reports)]:path.write_text(json.dumps(dict(cases=cases),indent=2)+'\n')
    print(len(api),'API cases;',len(reports),'reporter cases')


if __name__=='__main__':main()
