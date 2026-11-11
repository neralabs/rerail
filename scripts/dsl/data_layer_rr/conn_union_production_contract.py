"""Record complete connection-union repository facts and reports from source."""
from dataclasses import asdict
import contextlib
import io
from pathlib import Path
from unittest.mock import patch
from contract_runner import main
from conn_union_contract import CHECK,UNION
from dto import canonical
from first_ten_production_contract import normalize
from replay import write_fixture


def cases(_module):
    def file(path,source=UNION):return dict(path=path,source=source)
    groups=[('empty',[]),('positive',[file('impl/a.py')]),('clean',[file('impl/a.py','pass\n')]),
        ('roots',[file(p+'/a.py') for p in ['api','impl','data_layer','_internal','conform','l1000_a','l9000_z','other']]),
        ('unicode-layers',[file(p+'/a.py') for p in ['l١٢٣٤_x','l1234_a\nb','l1234_\nb','l1234_']]),
        ('nested',[file('impl/'+p+'/a.py') for p in ['normal','tests','test','conform','tmp','venv','.venv','.git','__pycache__','.hidden','node_modules','.pytest_cache']]),
        ('suffixes',[file('impl/'+p) for p in ['.py','.hidden.py','a.py','a.pyi','a.PY','a.txt','test_a.py','a_test.py']]),
        ('invalid',[file('impl/a.py','def broken(: # ReadTransaction\n')]),
        ('no-trigger',[file('impl/a.py','def load(owner: SyncDatabaseConnection):\n pass\n')]),
        ('duplicates',[file('impl/a.py',UNION+UNION)]),
        ('allowlisted',[file('impl/a.py','class SqlQuery:\n def __init__(self, owner: Connection | Transaction):\n  pass\n')]),
        ('order',[file('impl/'+p) for p in ['z.py','a/z.py','a-.py','a.py']]),
        ('file-links',[file('impl/a.py'),dict(path='impl/alias.py',symlink='a.py')]),
        ('directory-links',[file('impl/sub/a.py'),dict(path='impl/link',symlink='sub')]),
        ('linked-root',[file('shared/a.py'),dict(path='impl',symlink='shared')]),
        ('invalid-utf8',[dict(path='impl/a.py',rawBytes='ff')])]
    rows=[dict(kind='production',id=name,files=files) for name,files in groups]
    rows.extend([dict(kind='production',id='unsigned-signature',files=[file('impl/a.py')],signature='UNSIGNED'),
                 dict(kind='production',id='changed-allowlist',files=[file('impl/a.py')],entries=['changed'])])
    return rows


def production_value(module,case,scratch):
    root=scratch/'repo';root.mkdir();write_fixture(root,[f for f in case['files'] if 'rawBytes' not in f])
    for f in case['files']:
        if 'rawBytes' in f:
            p=root/f['path'];p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(bytes.fromhex(f['rawBytes']))
    try:
        with patch.object(module,'ALLOWLIST',set(case['entries']) if 'entries' in case else module.ALLOWLIST),patch.object(module,'_ALLOWLIST_SIGNATURE',case.get('signature',module._ALLOWLIST_SIGNATURE)):
            if hasattr(module,'_rerail_union_production'):value=module._rerail_union_production(root)
            else:
                from scripts.lint_lib.fact_engine import FactEngine
                from scripts.lint_lib.fact_store import JsonFactStore
                plugin=module.conn_or_txn_union_plugin(root)
                engine=FactEngine(root=root,fact_store=JsonFactStore(root/'cache'),extractors=module.conn_or_txn_union_fact_extractors(root),enable_fact_cache=False)
                plan=engine.plan([plugin]);view=engine.materialize((plugin,),max_workers=1)
                value=normalize(dict(selectedFiles=[p.relative_to(root).as_posix() for p in plan.files],facts={p.relative_to(root).as_posix():b.facts for p,b in sorted(view.files.items())},findings=[asdict(v) for v in plugin.reporter(view)],plugin={k:getattr(plugin,k) for k in ['name','enforcement','description']}),root)
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def exercise(module,case,scratch):
    out=io.StringIO();err=io.StringIO()
    with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):value=production_value(module,case,scratch)
    return dict(value,stdout=out.getvalue(),stderr=err.getvalue())


def install(backend,module,detectors=False):
    from conn_union_port import install as native_install,production
    native_install(backend,module,detectors=detectors)
    module._rerail_union_production=lambda root:production(backend,module,root)


if __name__=='__main__':
    main(CHECK,'rerail.conn-union-production/v1',cases,exercise,install,['conn_union_production_contract.py','conn_union_port.py','first_ten_production_contract.py'],guard_modules=('scripts.lint_lib.signing',))
