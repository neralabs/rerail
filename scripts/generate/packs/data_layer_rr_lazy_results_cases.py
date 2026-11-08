"""Source-evaluated lazy-result eligibility, wrapper, traversal and scope witnesses."""
import argparse
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api', type=Path, required=True)
    parser.add_argument('--reporters', type=Path, required=True)
    args = parser.parse_args()
    api, reports = [], []
    path = 'l2050_transaction/module.py'
    def add(identity, source, filename=path, report=True):
        api.append(dict(id=identity+'-file', check='lazy_result_usage', entry='_scan_source',
            files=[dict(path=filename, source=source)], arguments=dict(path=dict(path=filename),
                _source='pass\n', root=dict(path=''), tree=dict(pythonAst='pass\n', mode='exec'))))
        api.append(dict(id=identity+'-collector', check='lazy_result_usage', entry='collector', files=[],
            arguments=dict(filepath=dict(path=filename, relative=True), source='pass\n',
                tree=dict(pythonAst=source, mode='exec'))))
        if report:
            reports.append(dict(id=identity, check='lazy_result_usage', files=[dict(path=filename, source=source)]))
    def cls(name='MyTransaction', method='select_one', returns='Row', body='return Row()', extra='', base=''):
        ann = '' if returns is None else ' -> '+returns
        return f'class {name}{base}:\n    def {method}(self){ann}:\n        '+body.replace('\n', '\n        ')+'\n'+extra
    for method in ['select_one', 'select_one_or_raise', 'first', 'one', 'one_or_none', 'execute', '_execute', 'get']:
        add('method-'+method, cls(method=method))
    for name in ['UserQuery', 'CrossBranchTable', 'TransactionService', 'Service', 'query', 'AsyncQuery', '__LazyTable',
            '_AsyncQuery', '_LazyQuery', 'LazyishQuery', 'PreAsyncQuery', 'BaseDeferredQuery', 'DeferredInsertFromQueryBuilder',
            'Cursor', 'MigrationExecutorProtocol', '𝑸uery']:
        add('class-'+name, cls(name=name))
    for i, ret in enumerate([None, 'None', 'list [ Row ]', 'Row | None', '"Row"', 'LazyResult[Row]',
            'pkg.LazySeq[Row]', 'DeferredFuture[Row]', 'NotLazyResultish', '"DeferredFuture[Row]"',
            'tuple[Row, ...]', 'Callable[[A, B], C]', 'Literal["é", b"\\u0041"]']):
        add('return-'+str(i), cls(returns=ret))
    for i, body in enumerate(['pass', '...', '(...)', '"doc"\npass', '"doc"\n...',
            'return None', 'raise NotImplementedError', 'return self.select_one_lazy()',
            'return self.execute_lazy()', 'execute_lazy = 1\nreturn 1', '"execute_lazy"\nreturn 1',
            'def nested():\n    return execute_lazy()\nreturn 1', 'return object.other_lazy()',
            'class Nested:\n    execute_lazy = 1\nreturn 1']):
        add('body-'+str(i), cls(body=body))
    for method in ['select_one', 'select_one_or_raise', 'execute', 'first']:
        for sibling in dict.fromkeys(['execute_lazy', method+'_lazy', 'other_lazy']):
            add('sibling-'+method+'-'+sibling, cls(method=method, extra=f'    @decorator\n    async def {sibling}(self): pass\n'))
    for i, decorator in enumerate(['legacy_sync', 'legacy_async()', 'pkg.abstractmethod',
            'pkg.legacy_sync()(1)', 'other', '(legacy_sync)', 'legacy_sync[0]']):
        add('decorator-'+str(i), cls().replace('    def ', '    @'+decorator+'\n    def ', 1))
    for i, base in enumerate(['Protocol', 'typing.Protocol[T]', 'abc.ABC', '(Protocol)', 'Base[Protocol]',
            'object', 'ProtocolFactory()', 'Protocol | Other', 'metaclass=Protocol']):
        add('base-'+str(i), cls(base='('+base+')'))
    add('annotation-reference', 'class UserQuery:\n    def execute(self, x: execute_lazy) -> Row:\n        return 1\n')
    add('decorator-reference', 'class UserQuery:\n    @decorate(execute_lazy)\n    def execute(self) -> Row:\n        return 1\n')
    add('nested-functions', cls(body='def one() -> Row:\n    return 1\nreturn 2'))
    add('nested-classes', cls(body='class InnerQuery:\n    def one(self) -> Row: return 1\nreturn 2'))
    add('conditional-sibling', cls(extra='    if flag:\n        def execute_lazy(self): pass\n'))
    add('duplicate-methods', 'class MyTransaction:\n    def one(self) -> A: return 1\n    def one(self) -> B: return 2\n')
    add('async', cls().replace('def select_one', 'async def select_one'))
    add('module-level', 'def one() -> Row: return Row()\n')
    for i, filename in enumerate(['impl/sqlite/m.py', 'data_layer/m.py', '_internal/m.py', 'api/m.py', 'other/m.py',
            'l2055_lazy_result/m.py', 'l2070_batching/m.py', 'l2050_transaction/tests/m.py',
            'l2050_transaction/test/m.py', 'l2050_transaction/test_m.py', 'l2050_transaction/context_test_value/m.py',
            'l2050_transaction/__pycache__/m.py', 'l2050_transaction/.git/m.py', 'l2050_transaction/tmp/m.py',
            'l１２３４_unic/m.py', 'l123_foo/m.py']):
        add('scope-'+str(i), cls(), filename)
        api.append(dict(id='scope-'+str(i)+'-scan', check='lazy_result_usage', entry='find_violations',
            files=[dict(path=filename, source=cls())], arguments=dict(root=dict(path=''))))
    for i, filename in enumerate(['test_m.py', 'm.py', 'contest_/m.py', 'tests/x.py', 'x/l2070_batching/a.py']):
        api.append(dict(id='skip-'+str(i), check='lazy_result_usage', entry='should_skip', files=[],
            arguments=dict(filepath=dict(path=filename, relative=True))))
    api.append(dict(id='disabled-collector', check='lazy_result_usage', entry='collector', files=[],
        arguments=dict(filepath=dict(path=path, relative=True), disabled=True,
            tree=dict(pythonAst=cls(), mode='exec'))))
    for i, filename in enumerate([path, 'test_m.py', 'tests/m.py', 'l2070_batching/m.py', '/outside/m.py']):
        api.append(dict(id='factory-'+str(i), check='lazy_result_usage', entry='collector', files=[],
            arguments=dict(filepath=dict(path=filename, relative=True), factory=True,
                tree=dict(pythonAst=cls(), mode='exec'))))
    for budget in [-1, 0, 1, 2, '1', True, False, 0.9, 1.5, -0.9, 18446744073709551617]:
        reports.append(dict(id='ratchet-'+str(budget)+'-'+type(budget).__name__, check='lazy_result_usage', files=[
            dict(path=path, source=cls()), dict(path='scripts/lazy_result_baseline.json', source=json.dumps(dict(total=budget)))]))
    reports.append(dict(id='ratchet-count-across-files', check='lazy_result_usage', files=[
        dict(path=path, source=cls()), dict(path='impl/z.py', source=cls(method='one')),
        dict(path='scripts/lazy_result_baseline.json', source='{"total":1}')] ))
    for target, rows in [(args.api, api), (args.reporters, reports)]:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(dict(cases=rows), indent=2, ensure_ascii=False)+'\n')
    print(len(api), 'API cases;', len(reports), 'reporter cases')


if __name__ == '__main__':
    main()
