"""Generate async contracts; expected values come exclusively from the pinned source implementation."""
import argparse,json
from pathlib import Path
from data_layer_rr_text import module


def cases(source):
    original=module(source,'async_patterns');rows=[]
    def add(identity,code,path='api/input.py',tree=None,entry='analyze_source'):
        arguments=dict(filepath=dict(path=path))
        files=[dict(path=path,source=code)]
        if entry=='analyze_source':arguments.update(source=code,tree=dict(pythonAst=tree if tree is not None else code,mode='exec'))
        rows.append(dict(id=identity,check='async_patterns',entry=entry,files=files,arguments=arguments))
    for method in sorted(original.ALL_SYNC_TRANSACTION_METHODS):
        add('transaction-'+method,f'async def work(txn):\n    return txn.{method}()\n')
    for method in sorted(original.SYNC_INTERNAL_IO_METHODS):
        add('internal-'+method,f'async def work():\n    return self.{method}()\n')
    for method in sorted(original.SYNC_POLYMORPHIC_METADATA_METHODS):
        add('metadata-'+method,f'async def work(manager):\n    return manager.{method}()\n')
    for method in sorted(original.SYNC_CONNECTION_METHODS):
        add('connection-'+method,f'async def work():\n    return self._conn.{method}()\n')
    bodies={
      'query-chain': 'return txn.query(Model).filter(name="x").execute()',
      'query-parentheses': 'return (((txn.query(Model)))).filter(name="x").execute()',
      'wrapped-call-function': 'return (txn.select_one)(Model)',
      'query-alias': 'query = txn.query(Model).filter(active=True)\nreturn query.execute()',
      'query-alias-copy': 'q = txn.query(Model)\nr = q\nreturn r.execute()',
      'alias-before-seed': 'r = q\nq = txn.query(Model)\nreturn r.execute()',
      'query-reassigned': 'q = txn.query(Model)\nq = []\nreturn q.count()',
      'session': 's = conn.session(key)\nreturn s.read(callback)',
      'session-copy': 's = conn.session(key)\nr = s\nreturn r.write(callback)',
      'session-annassign': 's: Session = conn.session(key)\nreturn s.read(callback)',
      'query-annassign': 'q: Query = txn.query(Model)\nreturn q.execute()',
      'session-tuple-target': 's, other = conn.session(key)\nreturn s.read(callback)',
      'session-chained-target': 's = other = conn.session(key)\nreturn other.read(callback)',
      'attribute-target': 'self.s = conn.session(key)\nreturn self.s.read(callback)',
      'session-before-seed': 's.read(callback)\ns = conn.session(key)',
      'non-query-list': 'rows = list(txn.query(Model).execute())\nreturn rows.count(x)',
      'non-query-subscript': 'q = txn.query(Model)[0]\nreturn q.execute()',
      'non-query-attribute': 'q = txn.query(Model).part\nreturn q.execute()',
      'txn-not-aliased': 'copy = txn\nreturn copy.select_one()',
      'self-direct': 'return self.select_one()',
      'self-attribute': 'return self.other.select_one()',
      'session-attribute': 's = conn.session(key)\nreturn s.other.read(callback)',
      'query-attribute': 'q = txn.query(Model)\nreturn q.other.execute()',
      'branch-alias': 'if False:\n    s = conn.session(key)\nreturn s.read(callback)',
      'loop-alias': 'for x in xs:\n    q = txn.query(Model)\nreturn q.execute()',
      'class-keeps-async': 'class Inner:\n    x = txn.select_one()',
      'class-keeps-aliases': 'class Inner:\n    s = conn.session(key)\nreturn s.read(callback)',
      'lambda-clears-state': 'return lambda txn: txn.select_one()',
      'sync-clears-state': 'def nested():\n    return txn.select_one()\nreturn nested',
      'async-clears-aliases': 's = conn.session(key)\nasync def nested():\n    return s.read(callback)\nreturn nested',
      'async-new-params': 'async def nested(other: ReadTransaction):\n    return other.select_one()\nreturn nested',
      'async-outer-txn-not-carried': 'async def nested():\n    return txn.select_one()\nreturn nested',
      'receiver-attributes': 'return txn.member["a"].select_one()',
      'receiver-precedence': 'return (txn if ok else other).ensure_dynamic_polymorphic_metadata(conn)',
      'receiver-lambda': 'return (lambda: None).ensure_dynamic_polymorphic_metadata(conn)',
      'receiver-fstring': 'return f"{manager=}".ensure_dynamic_polymorphic_metadata(conn)',
      'receiver-subscript-quotes': 'return txn["a"].select_one()',
      'bare-sync-method': 'return select_one()',
      'bare-to-thread': 'return to_thread(work)',
      'async-variants': 's = conn.session(key)\nawait s.read_async(callback)\nreturn await txn.select_one_async()',
    }
    for name,body in bodies.items():
        code='async def work(txn, conn):\n'+''.join('    '+line+'\n' for line in body.splitlines())
        add(name,code)
    for annotation in ['ReadTransaction','db.WriteTransaction','Optional[ReadTransaction]','tuple[ReadTransaction, None]',
        'ReadTransaction | None',"'ReadTransaction | None'",'AsyncReadTransaction',"'AsyncReadTransaction'",'list[Other]',
        "'prefixReadTransactionSuffix'",'ReadTransaction + None',"'\\ud800ReadTransaction'"]:
        add('annotation-'+annotation,f'async def work(custom: {annotation}):\n    return custom.select_one()\n')
    patterns={
      'module-to-thread':'asyncio.to_thread(work)\n',
      'module-executor':'loop.run_in_executor(None, work)\n',
      'other-to-thread':'other.to_thread(work)\n',
      'sync-to-thread':'def f():\n    return x.to_thread(work)\n',
      'write-decorated':'@session.write_async()\nasync def f(other):\n    other.insert(x)\n    other.execute()\n',
      'read-decorated':'@session.read_async()\nasync def f(other):\n    other.insert(x)\n    other.execute()\n',
      'bare-decorator-ignored':'@write_async()\nasync def f(txn):\n    txn.insert(x)\n',
      'uncalled-decorator-ignored':'@session.write_async\nasync def f(txn):\n    txn.insert(x)\n',
      'nested-inherits-decorator':'@session.write_async()\nasync def f():\n    async def inner():\n        other.insert(x)\n',
      'nested-overrides-decorator':'@session.write_async()\nasync def f():\n    @s.read_async()\n    async def inner():\n        other.insert(x)\n        other.execute()\n',
      'decorator-entered-before-traversal':'@dec(self.select_one())\nasync def f():\n    pass\n',
      'default-entered-before-traversal':'async def f(x=self.select_one()):\n    pass\n',
      'api-abstract-name':'class MyQuery:\n    async def f(self):\n        return x.to_thread(work)\n',
      'api-abc':'class Owner(ABC):\n    async def f(self):\n        return self.select_one()\n',
      'api-abc-attribute':'class Owner(pkg.ABC):\n    async def f(self):\n        return self._fetchall()\n',
      'api-abc-call-not-base':'class Owner(ABC()):\n    async def f(self):\n        return self.select_one()\n',
      'nested-class-resets-abstract':'class MyQuery:\n    class Inner:\n        async def f(self):\n            return x.to_thread(work)\n',
      'api-class-decorator':'@dec(x.to_thread(work))\nclass MyQuery:\n    pass\n',
      'fallback-marker':'async def f(self):\n    # ASYNC_FALLBACK:\n    return self.select_one()\n',
      'todo-marker':'async def f(self):\n    # TODO: Implement\n    # native async\n    return self._fetchall()\n',
      'literal-marker':'async def f(self):\n    label = "ASYNC_FALLBACK:"\n    return self.select_one()\n',
      'marker-outside-radius':'# ASYNC_FALLBACK:\n'+'\n'*6+'async def f(self):\n    return self.select_one()\n',
      'executor-wrapper':'# _wrap_for_executor\nloop.run_in_executor(None, work)\n',
      'impl-explicit-marker':'# ASYNC_FALLBACK:\nx.to_thread(work)\n',
      'always-actionable-connection':'# ASYNC_FALLBACK:\nasync def f(self):\n    return self._conn.read(callback)\n',
      'always-actionable-metadata':'# ASYNC_FALLBACK:\nasync def f(manager):\n    return manager.ensure_dynamic_polymorphic_metadata(conn)\n',
      'nfkc-function':'async def 𝚠ork(txn):\n    return txn.select_one()\n',
      'unicode-splitlines':'# heading\x1c\nasync def f(self):\n    return self.select_one()\n',
      'no-trigger':'# source has no prefilter token\nx = 1\n',
    }
    for name,code in patterns.items():add(name,code,path='impl/common/input.py' if name.startswith('impl-') else 'api/input.py')
    # Execute representative state and scope cases through the real file collector too.
    for identity in ['query-chain','query-parentheses','query-alias-copy','alias-before-seed','session-chained-target','class-keeps-async',
                     'lambda-clears-state','nested-inherits-decorator','decorator-entered-before-traversal','default-entered-before-traversal',
                     'api-abstract-name','nested-class-resets-abstract','api-class-decorator','fallback-marker','executor-wrapper','unicode-splitlines']:
        row=next(row for row in rows if row['id']==identity);add('file-'+identity,row['arguments']['source'],row['files'][0]['path'],entry='lint_file')
    positive='async def supplied(txn):\n    return txn.select_one()\n'
    for identity,actual,tree in [('given-other-source','async def actual():\n    pass\n',positive),('given-empty','async def actual():\n    pass\n',''),
                                ('given-invalid-source','async def broken(:\n',positive),('given-offset','# async def\n','\n'*7+positive),
                                ('given-trigger-absent','x = 1\n',positive),('given-fallback-source','# ASYNC_FALLBACK: async def\n',positive)]:
        add(identity,actual,tree=tree)
    for path in ['other/input.py','myapi/input.py','api/test_input.py','api/tests/input.py','api/__pycache__/input.py','conform/input.py','api/input.txt']:
        add('path-'+path,positive,path=path)
    for identity,code in [('missing-tree-triggered',positive),('missing-tree-clean','x = 1\n')]:
        rows.append(dict(id=identity,check='async_patterns',entry='analyze_source',files=[],arguments=dict(filepath=dict(path='api/input.py'),source=code)))
    for identity,path in [('missing-file','api/missing.py'),('excluded-missing-file','api/test_missing.py')]:
        rows.append(dict(id=identity,check='async_patterns',entry='lint_file',files=[],arguments=dict(filepath=dict(path=path))))
    rows.append(dict(id='invalid-utf8',check='async_patterns',entry='lint_file',files=[dict(path='api/input.py',bytesHex='ff')],arguments=dict(filepath=dict(path='api/input.py'))))
    return rows


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--source',type=Path,required=True);parser.add_argument('--destination',type=Path,required=True);parser.add_argument('--reporters',type=Path,required=True);args=parser.parse_args();rows=cases(args.source.resolve())
    args.destination.write_text(json.dumps(dict(format='rerail.rr-fresh-cases/v1',cases=rows),indent=2,ensure_ascii=True)+'\n')
    reports=[dict(id=row['id'],check=row['check'],files=row['files']) for row in rows if row['entry']=='analyze_source' and row['files'] and row['id'].split('-')[0] not in {'given','path'}]
    positive='async def work(txn):\n    return txn.select_one()\n'
    for path in ['api/input.py','impl/common/input.py','impl/sqlite/input.py','impl/other/input.py','conform/input.py','l2050_transaction/input.py',
                 'api/test_input.py','api/test/input.py','api/tests/input.py','api/tmp/input.py','api/venv/input.py','myapi/input.py']:
        reports.append(dict(id='scope-'+path,check='async_patterns',files=[dict(path=path,source=positive)]))
    args.reporters.write_text(json.dumps(dict(format='rerail.rr-plugin-reporter-cases/v1',cases=reports),indent=2,ensure_ascii=True)+'\n');print(len(rows),len(reports))


if __name__=='__main__':main()
