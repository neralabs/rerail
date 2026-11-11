"""Compare complete connection/transaction scanner results with the pinned linter."""
import contextlib
from functools import wraps
import inspect
from pathlib import Path
from contract_runner import main
from conn_union_port import scan
from dto import canonical

CHECK='conn_or_txn_union'
UNION='def load(owner: SyncDatabaseConnection | ReadTransaction):\n return owner\n'


def cases(module):
    rows=[]
    annotations={
        'union':'SyncDatabaseConnection | ReadTransaction',
        'nested':'Optional[Union[SyncDatabaseConnection, ReadTransaction]]',
        'attribute':'api.SyncDatabaseConnection | api.ReadTransaction',
        'parenthesized':'(SyncDatabaseConnection | ReadTransaction)',
        'tuple-slice':'Union[(SyncDatabaseConnection, ReadTransaction)]',
        'string-name':'Union["SyncDatabaseConnection", "ReadTransaction"]',
        'quoted-union':'"SyncDatabaseConnection | ReadTransaction"',
        'and-operator':'SyncDatabaseConnection & ReadTransaction',
        'tuple':'(SyncDatabaseConnection, ReadTransaction)',
        'list':'[SyncDatabaseConnection, ReadTransaction]',
        'call':'Union(SyncDatabaseConnection, ReadTransaction)',
        'connection':'SyncDatabaseConnection',
        'transaction':'ReadTransaction',
        'no-trigger':'int',
    }
    for name,annotation in annotations.items():rows.append(dict(id='annotation-'+name,source=f'def load(owner: {annotation}):\n return owner\n'))
    for name in sorted(module.CONNECTION_TYPES):rows.append(dict(id='connection-'+name,source=f'def load(owner: {name} | ReadTransaction):\n pass\n'))
    for name in sorted(module.TRANSACTION_TYPES):rows.append(dict(id='transaction-'+name,source=f'def load(owner: SyncDatabaseConnection | {name}):\n pass\n'))
    shapes={
        'separate':'def load(conn: SyncDatabaseConnection, txn: ReadTransaction):\n pass\n',
        'keyword-only':'def load(*, conn: SyncDatabaseConnection, txn: ReadTransaction):\n pass\n',
        'positional-only':'def load(conn: SyncDatabaseConnection, /, txn: ReadTransaction):\n pass\n',
        'argument-order':'def load(a: SyncDatabaseConnection | ReadTransaction, /, b: Connection | WriteTransaction):\n pass\n',
        'vararg':'def load(*args: SyncDatabaseConnection | ReadTransaction):\n pass\n',
        'kwarg':'def load(**kwargs: SyncDatabaseConnection | ReadTransaction):\n pass\n',
        'self':'def load(self: SyncDatabaseConnection | ReadTransaction):\n pass\n',
        'async':'async def load(owner: SyncDatabaseConnection | ReadTransaction):\n pass\n',
        'nested':'class Outer:\n class Inner:\n  def load(owner: SyncDatabaseConnection | ReadTransaction):\n   pass\n',
        'nested-function':'class Outer:\n def method(self):\n  def load(owner: SyncDatabaseConnection | ReadTransaction):\n   pass\n',
        'bare-allow':'def execute_vector_query(owner: SyncDatabaseConnection | ReadTransaction):\n pass\n',
        'qualified-allow':'class SqlQuery:\n def __init__(owner: SyncDatabaseConnection | ReadTransaction):\n  pass\n',
        'multiple':'def a(conn: Connection | Transaction):\n pass\ndef b(conn: Connection | Transaction):\n pass\n',
        'invalid':'def broken(: # Connection ReadTransaction\n',
        'unicode':'def ｌoad(ｏwner: Ｃonnection | Ｔransaction):\n pass\n# Connection ReadTransaction\n',
    }
    rows.extend(dict(id='shape-'+name,source=text) for name,text in shapes.items())
    for path in ['impl/a.py','api/a.py','outside/a.py','a.py','tests/a.py','test/a.py','impl/test_a.py','impl/a_test.py',
                 'tmp/a.py','.hidden/a.py','impl/a.pyi','impl/a.PY','impl/a.txt','impl/.py','impl/.hidden.py']:
        rows.append(dict(id='path-'+path,path=path,source=UNION))
    rows.extend([dict(id='missing',source=UNION,missing=True),dict(id='invalid-utf8',source=UNION,rawBytes='ff'),
        dict(id='outside-owner-root',source=UNION,path='outside/a.py',ownerRoot='repo/impl'),
        dict(id='default-owner-root',source=UNION,path='outside/a.py',ownerRoot=None)])
    return [dict(kind='scanner',path='impl/a.py',**row) if 'path' not in row else dict(kind='scanner',**row) for row in rows]


def exercise(module,case,scratch):
    root=scratch/'repo';root.mkdir();path=root/case['path'];path.parent.mkdir(parents=True,exist_ok=True)
    if not case.get('missing'):path.write_bytes(bytes.fromhex(case['rawBytes']) if 'rawBytes' in case else case['source'].encode())
    owner=scratch/case['ownerRoot'] if case.get('ownerRoot') else root
    try:
        with contextlib.chdir(scratch):
            value=module.analyze_file(path,**({} if 'ownerRoot' in case and case['ownerRoot'] is None else dict(root=owner)))
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    if detectors:
        original=module.analyze_file;signature=inspect.signature(original)
        @wraps(original)
        def analyze(*args,**kwargs):
            bound=signature.bind(*args,**kwargs);bound.apply_defaults()
            return scan(backend,'analyze_file',module,bound.arguments)
        backend.replace(module,'analyze_file',analyze)


if __name__=='__main__':
    main(CHECK,'rerail.conn-union-scanner/v1',cases,exercise,install,['conn_union_contract.py','conn_union_port.py'])
