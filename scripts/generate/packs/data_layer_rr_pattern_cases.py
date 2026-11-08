"""Generate transaction-pattern contracts; record expected values with the unchanged source."""
import argparse,json
from pathlib import Path
from data_layer_rr_text import module


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['source','destination','reporters']:parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();original=module(args.source.absolute(),'transaction_patterns');api=[];reports=[]
    path='l5200_vector_index/example.py'
    def add(identity,source,file=path,cats=None,baseline=None):
        cats={} if cats is None else cats
        api.append(dict(id=identity+'-graph',check='transaction_patterns',entry='finder',files=[],arguments=dict(
            file_path=dict(path=file),tree=dict(pythonAst=source,mode='exec'),categorized_functions=cats)))
        api.append(dict(id=identity+'-file',check='transaction_patterns',entry='analyze_file',files=[dict(path=file,source=source)],
            arguments=dict(file_path=dict(path=file),categorized_functions=cats)))
        files=[dict(path=file,source=source)]
        if baseline is not None:files.append(dict(path=original.BASELINE_FILE,source=json.dumps(dict(categories=baseline))))
        reports.append(dict(id=identity,check='transaction_patterns',files=files))
    for tag,annotation in [('plain','SyncDatabaseConnection'),('attr','pkg.SyncDatabaseConnection'),('optional','Optional[SyncDatabaseConnection]'),
        ('union-left','SyncDatabaseConnection | None'),('union-right','None | SyncDatabaseConnection'),
        ('all-binops','SyncDatabaseConnection + None'),('connection-id','ConnectionId'),('id-before-connection','ConnectionId | SyncDatabaseConnection'),
        ('conn-union','conn | None'),('quoted','"SyncDatabaseConnection"'),('bare-conn','conn'),('quoted-conn','"conn"'),
        ('bytes','b"Connection"'),('bytes-id','b"ConnectionId"'),('bytes-conn','b"conn"'),('bytes-unicode-escape',r'b"\u0043onnection"'),
        ('bytes-hex-escape',r'b"\x43onnection"'),('bytes-adjacent','b"Connec" b"tion"'),
        ('adjacent','"Connec" "tion"'),('fstring','f"Connection"'),('surrogate-tail',r'"Connection\ud800"'),
        ('surrogate-id',r'"\ud800ConnectionId"'),('unicode-identifier','ＳｙｎｃＤａｔａｂａｓｅＣｏｎｎｅｃｔｉｏｎ'),
        ('subscript','SyncDatabaseConnection[int]'),('attribute-id','pkg.ConnectionId')]:
        add('annotation-'+tag,f'def f(conn: {annotation}):\n    pass\n')
    for tag,parameters in [('posonly','conn: Connection, /'),('pos-after-only','a: Connection, /, conn: Connection'),
        ('keyword-only','*, conn: Connection'),('vararg','*conn: Connection'),('kwarg','**conn: Connection'),
        ('self','self: Connection'),('first-only','a: Connection, b: Connection'),('default','conn: Connection = None')]:
        add('parameters-'+tag,f'def f({parameters}):\n    b.write()\n    conn.read()\n')
    for tag,body in [
        ('ordered-duplicates','conn.write(); conn.read(); conn.write(); conn.schema_transaction()'),
        ('nested-calls','conn.read(conn.write())'),('lambda-transparent','f(lambda: conn.write())'),
        ('other-receiver','other.write(); self.conn.read(); get().schema_transaction()'),
        ('keyword-then-star','f(k=conn.read(), *(conn.write(),))'),
        ('dictionary-field-order','{conn.read(): conn.write(), conn.schema_transaction(): conn.read()}'),
        ('conditional-field-order','conn.read() if conn.write() else conn.schema_transaction()')]:
        add('calls-'+tag,'def register_queue(conn: Connection):\n    '+body+'\n')
    add('nested-functions','def register_queue(conn: Connection):\n    def register_table(conn: Connection):\n        conn.read()\n    conn.write()\n')
    add('nested-same-end','def f(conn: Connection):\n    def g(conn: Connection):\n        conn.write()\n')
    add('class-boundaries','class MyConnection:\n    def __init__(self, conn: Connection): conn.write()\n    def register_queue(self, conn: Connection): conn.read()\n')
    add('async','async def register_queue(conn: Connection):\n    await conn.read()\n')
    add('class-decorator-in-function','def register_queue(conn: Connection):\n    @conn.read()\n    class Nested: pass\n')
    add('function-decorator-and-default','def outer(conn: Connection):\n    @conn.write()\n    def inner(x: Connection = conn.read()): pass\n')
    for tag,cats in [('allowed',{'BOOTSTRAP':['register_queue']}),('denied',{'SCHEMA_OPS':['register_queue']}),
        ('first-allowed',{'BACKGROUND':['register_queue'],'SCHEMA_OPS':['register_queue']}),
        ('first-denied',{'SCHEMA_OPS':['register_queue'],'BACKGROUND':['register_queue']})]:
        add('override-'+tag,'def register_queue(conn: Connection):\n    conn.write()\n',cats=cats,baseline=cats)
    for tag,cats in [('needs-review-first',{'NEEDS_REVIEW':['f'],'ZZZ_ALLOWED':['f']}),
        ('unknown-first',{'ZZZ_ALLOWED':['f'],'NEEDS_REVIEW':['f']}),
        ('bridge-first',{'TS_BRIDGE':['f'],'NEEDS_REVIEW':['f']}),('empty-before',{'EMPTY':[],'NEEDS_REVIEW':['f']})]:
        add('baseline-'+tag,'def f(conn: Connection):\n    pass\n',cats=cats,baseline=cats)
    for tag,file in [('impl','impl/sqlite/example.py'),('api','api/example.py'),('other','other/example.py'),
        ('tests','l5200_vector_index/tests/example.py'),('test-prefix','l5200_vector_index/test_example.py'),
        ('unicode-layer','l１２３４_example/example.py'),('short-layer','l123_example/example.py')]:
        add('scope-'+tag,'def register_queue(conn: Connection):\n    conn.write()\n',file=file)
    for i,(name,file) in enumerate([('f','l1000_foo/m.py'),('f','impl/sqlite/m.py'),('f','l1000_foo/impl/sqlite/m.py'),
        ('f','impl/l1000_foo/sqlite/m.py'),('f','other/m.py'),('f','l１２３４_foo/m.py'),('f','l1234_\n/m.py'),
        ('f','l1234_a\nrest/m.py'),('ensure_validate_handler','l9900_ts_models/m.py'),('BOOTSTRAP','other/m.py')]):
        api.append(dict(id='inference-'+str(i),check='transaction_patterns',entry='_infer_category',files=[],arguments=dict(func_name=name,file_path=file)))
    for i,baseline in enumerate([None,{},dict(categories={}),dict(categories={'UNKNOWN':[],'LEGITIMATE':['f','f']}),
        dict(categories={'Z':['x'],'A':['x'],'BOOTSTRAP':[]})]):
        api.append(dict(id='merged-'+str(i),check='transaction_patterns',entry='_get_categorized_functions',files=[],arguments=dict(baseline=baseline)))
    for i,value in enumerate([{}, {'Z':['z','a','a'],'A':[]},{'é':['非','a'],'A':['x']},{'a/b':['\\','"','\n']} ]):
        api.append(dict(id='signature-'+str(i),check='transaction_patterns',entry='_compute_signature_message',files=[],arguments=dict(categories=value)))
    for i,(name,cls) in enumerate([('__init__','MyConnection'),('__init__','connection'),('__init__',None),('close','Connection')]):
        api.append(dict(id='init-'+str(i),check='transaction_patterns',entry='_is_connection_class_init',files=[],arguments=dict(func_name=name,class_name=cls)))
    for i,provided in enumerate(['pass\n','# conn\n']):
        api.append(dict(id='provided-gate-'+str(i),check='transaction_patterns',entry='analyze_source',
            files=[dict(path=path,source='def f(conn: Connection): pass\n')],arguments=dict(file_path=dict(path=path),source=provided,categorized_functions={})))
    for i,text in enumerate(['def f(): pass\n','def f(): conn.read()\n','def f(): conn.read(); conn.read()\n']):
        api.append(dict(id='internal-'+str(i),check='transaction_patterns',entry='internal',files=[],arguments=dict(
            connectionName='conn',tree=dict(pythonAst=text,mode='exec'),initial=['existing'])))
    for path_arg,rows in [(args.destination,api),(args.reporters,reports)]:
        path_arg.parent.mkdir(parents=True,exist_ok=True);path_arg.write_text(json.dumps(dict(cases=rows),indent=2,ensure_ascii=False)+'\n')
    print(len(api),'API cases;',len(reports),'reporter cases')


if __name__=='__main__':main()
