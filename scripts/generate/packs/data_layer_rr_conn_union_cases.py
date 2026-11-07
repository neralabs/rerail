"""Generate connection/transaction contracts; the unchanged source provides all expected values."""
import argparse,json
from pathlib import Path


def cases():
    rows=[]
    def add(name,text,path='input.py'):
        rows.append(dict(id=name,check='conn_or_txn_union',entry='analyze_file',files=[dict(path=path,source=text)],arguments=dict(file_path=dict(path=path),root=dict(path='.'))))
    conns=['Connection','SyncDatabaseConnection','SyncSqliteConnection','SyncPostgresConnection','MariaDbConnection','MongoDbConnection','SyncNeo4jConnection','DoltDbConnection']
    txns=['Transaction','ReadTransaction','WriteTransaction','SchemaTransaction','LazyVersionedReadTransaction','LazyVersionedWriteTransaction','AsyncReadTransaction','AsyncWriteTransaction']
    for x in conns:add('connection-'+x,f'def f(x: {x} | Transaction):\n    pass\n')
    for x in txns:add('transaction-'+x,f'def f(x: Connection | {x}):\n    pass\n')
    expressions={
      'qualified':'db.Connection | api.ReadTransaction', 'generic':'Optional[Union[Connection, ReadTransaction]]',
      'generic-head':'Connection[ReadTransaction]', 'tuple-slice':'Box[Connection, ReadTransaction]',
      'quoted-members':"Union['Connection', 'ReadTransaction']",'quoted-union':"'Connection | ReadTransaction'",
      'triple-quoted':"Union['''Connection''', \"ReadTransaction\"]", 'concat-strings':"Union['Con' 'nection', 'Read' 'Transaction']",
      'raw-string':"Union[r'Connection', u'ReadTransaction']", 'comment-spacing':'(Connection # comment\n        | ReadTransaction)',
      'parens':'(((Connection | (ReadTransaction))))', 'duplicate-names':'Connection | Connection | ReadTransaction',
      'unsupported-tuple':'(Connection, ReadTransaction)', 'unsupported-list':'[Connection, ReadTransaction]',
      'unsupported-plus':'Connection + ReadTransaction', 'unsupported-bool':'Connection or ReadTransaction',
      'unsupported-call':'Union(Connection, ReadTransaction)', 'attribute-stops':'ReadTransaction.Connection',
      'generic-list-slice':'Callable[[Connection], ReadTransaction]', 'generic-tuple-slice':'Callable[(Connection, ReadTransaction)]',
      'complex-siblings':"Box[Connection, ReadTransaction, factory(1, x='a'), {2:3}, lambda x: x]",
      'fstring-sibling':'Box[Connection, ReadTransaction, f"{value=}"]',
      'numeric-siblings':"Box[Connection, ReadTransaction, 0xff, 1.0j, 1e999, b'\\xff', '\\ud800']",
      'comprehension-sibling':'Box[Connection, ReadTransaction, [x for x in other]]',
      'nfkc-name':'𝙲onnection | ReadTransaction', 'nfkc-transaction-no-trigger':'Connection | 𝚁eadTransaction',
      'nfkc-transaction-with-trigger':'Connection | 𝚁eadTransaction',
      'unknown-transaction':'Connection | OtherTransaction','connection-only':'Connection','transaction-only':'ReadTransaction',
      'empty-string':"Union['', ReadTransaction]", 'literal-false':'False',
    }
    for name,ann in expressions.items():add(name,('# ReadTransaction\n' if name == 'nfkc-transaction-with-trigger' else '')+'def f(value: '+ann+'):\n    pass\n')
    functions={
      'separate':'def f(conn: Connection, txn: ReadTransaction):\n    pass\n',
      'separate-required':'def f(conn: Connection, txn: ReadTransaction, other):\n    pass\n',
      'separate-optionals':'def f(conn: Connection | None = None, txn: ReadTransaction | None = None):\n    pass\n',
      'multiple-separate':'def f(z:Connection, a:Connection, txn:Transaction, txn2:ReadTransaction):\n    pass\n',
      'union-wins':'def f(conn:Connection,txn:Transaction,later:Connection|ReadTransaction):\n    pass\n',
      'first-union':'def f(a:Connection|Transaction,b:Connection|ReadTransaction):\n    pass\n',
      'args-before-posonly':'def f(first:Connection|Transaction,/,later:Connection|ReadTransaction):\n    pass\n',
      'separate-argument-order':'def f(posonly:Connection,/,pos:Connection,*,kw:Transaction):\n    pass\n',
      'kwonly-union':'def f(*,both:Connection|Transaction):\n    pass\n',
      'varargs-ignored':'def f(*both:Connection|Transaction):\n    pass\n',
      'kwargs-ignored':'def f(**both:Connection|Transaction):\n    pass\n',
      'self-ignored':'def f(self:Connection|Transaction):\n    pass\n',
      'cls-not-ignored':'def f(cls:Connection|Transaction):\n    pass\n',
      'return-ignored':'def f()->Connection|Transaction:\n    pass\n',
      'default-ignored':'def f(x=Connection|Transaction):\n    pass\n',
      'async':'async def f(x:Connection|Transaction):\n    pass\n',
      'nested-function':'def outer():\n    def inner(x:Connection|Transaction):\n        pass\n',
      'method':'class Owner:\n    def f(self,x:Connection|Transaction):\n        pass\n',
      'inner-class-name':'class Outer:\n    class Inner:\n        def f(self,x:Connection|Transaction):\n            pass\n',
      'nested-function-class-context':'class Owner:\n    def outer(self):\n        def inner(x:Connection|Transaction):\n            pass\n',
      'allowlist-bare':'def execute_vector_query(x:Connection|Transaction):\n    pass\n',
      'allowlist-qualified':'class SqlQuery:\n    def __init__(self,x:Connection|Transaction):\n        pass\n',
      'allowlist-not-other-class':'class Other:\n    def __init__(self,x:Connection|Transaction):\n        pass\n',
      'all-allowlisted':'class SqlQuery:\n    def __init__(self,x:Connection|Transaction):\n        def later(y:Connection|Transaction):\n            pass\n',
      'event-order':'@dec\ndef first(x:Connection|Transaction):\n    def inner(x:Connection|ReadTransaction):\n        pass\nasync def last(conn:Connection,txn:Transaction):\n    pass\n',
      'invalid-source':'def broken(:\n# Connection Transaction\n',
      'clean-source':'x = 1\n',
    }
    for name,text in functions.items():add(name,text)
    for path in ['input.txt','test_input.py','tests/input.py','test/input.py','tmp/input.py','conform/input.py']:
        add('path-'+path,'def f(x:Connection|Transaction):\n    pass\n',path)
    rows.append(dict(id='missing-file',check='conn_or_txn_union',entry='analyze_file',files=[],arguments=dict(file_path=dict(path='missing.py'),root=dict(path='.'))))
    rows.append(dict(id='invalid-utf8',check='conn_or_txn_union',entry='analyze_file',files=[dict(path='input.py',bytesHex='ff')],arguments=dict(file_path=dict(path='input.py'),root=dict(path='.'))))
    return rows


def main():
    p=argparse.ArgumentParser();p.add_argument('--destination',type=Path,required=True);p.add_argument('--reporters',type=Path,required=True);args=p.parse_args();rows=cases()
    args.destination.write_text(json.dumps(dict(format='rerail.rr-fresh-cases/v1',cases=rows),indent=2)+'\n')
    reports=[dict(id=row['id'],check=row['check'],files=[dict(f,path='impl/'+f['path']) for f in row['files']]) for row in rows if row['files'] and all('source' in f for f in row['files'])]
    for path in ['api/input.py','conform/input.py','data_layer/input.py','_internal/input.py','l1234_types/input.py','l１２３４_types/input.py','l123_types/input.py','l1234_\n/input.py','l1234_a\n/input.py','other/input.py','input.py','.git/input.py','impl/.venv/input.py']:
        reports.append(dict(id='scope-'+path,check='conn_or_txn_union',files=[dict(path=path,source='def f(x:Connection|Transaction):\n    pass\n')]))
    args.reporters.write_text(json.dumps(dict(format='rerail.rr-plugin-reporter-cases/v1',cases=reports),indent=2)+'\n');print(len(rows),len(reports))


if __name__=='__main__':main()
