"""Record connection-union configuration, collector, helper, and reporter contracts."""
import ast
import contextlib
import io
from dataclasses import asdict,fields
from pathlib import Path
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from conn_union_contract import CHECK,UNION
from replay import write_fixture


def cases(_module):
    rows=[dict(id='metadata-'+str(i),kind='metadata',directories=names) for i,names in enumerate([
        [],['api'],['impl','data_layer','_internal','conform','l9000_z','l1000_a','other'],
        ['l١٢٣٤_x','l1234_a\nb','l1234_\nb','l1234_']])]
    for name,text in [('positive',UNION),('clean','pass\n'),('no-trigger','def load(x: int):\n pass\n'),
        ('invalid','def broken(: # ReadTransaction\n'),('allowlisted','class SqlQuery:\n def __init__(self, owner: Connection | Transaction):\n  pass\n'),
        ('duplicates',UNION+UNION),('bare-connections','def load(owner: SyncDatabaseConnection):\n pass\n')]:
        rows.append(dict(id='extract-'+name,kind='extract',source=text))
    for name,text in [('positive',UNION),('clean','pass\n'),('no-trigger','def load(x: int):\n pass\n')]:
        rows.append(dict(id='registry-extract-'+name,kind='registry-extract',source=text))
    annotations=['Connection','ReadTransaction','api.Connection','"Connection"','"Connection | Transaction"',
        'Connection | ReadTransaction','Connection & ReadTransaction','Union[Connection,ReadTransaction]',
        'Union[(Connection,ReadTransaction)]','(Connection,ReadTransaction)','[Connection,ReadTransaction]',
        'Union(Connection,ReadTransaction)','Optional[api.Connection | api.ReadTransaction]','42','None']
    for i,text in enumerate(annotations):
        for entry in ['_get_all_type_names','_has_connection_type','_has_transaction_type','_has_connection_and_transaction']:
            rows.append(dict(id=f'helper-{entry}-{i}',kind='helper',entry=entry,annotation=text))
    for i,(stack,name) in enumerate([([], 'load'),(['Outer'],'load'),(['Outer','Inner'],'load'),(['SqlQuery'],'__init__'),([], 'execute_vector_query')]):
        for entry in ['_get_qualified_name','_is_allowlisted']:
            rows.append(dict(id=f'helper-{entry}-{i}',kind='helper',entry=entry,stack=stack,name=name))
    for i,value in enumerate([0,True,False,-1,2**100,None,1.5,'line']):rows.append(dict(id='project-'+str(i),kind='project',line=value))
    hit=dict(line_number=3,function_name='supplied',violation_type='CUSTOM',details='caller details')
    values=[None,{},'bad',0,[],[None,1,[],{}],[hit],[hit,hit],
        [dict(hit,line_number=l) for l in [True,False,-1,0,2**100,-2**100,'3',1.5,None]],
        [dict(hit,**{key:value}) for key in ['function_name','violation_type','details'] for value in [None,True,1,[],{},'']],
        [dict(hit,line_number=l,details=m) for l,m in [(9,'b'),(1,'z'),(9,'a'),(9,'a')]]]
    rows.extend(dict(id='report-'+str(i),kind='report',bundles=[dict(key='a.py',path='ignored.py',value=v)]) for i,v in enumerate(values))
    rows.extend([dict(id='report-path-order',kind='report',bundles=[dict(key=p,value=[hit]) for p in ['z.py','a/z.py','a-.py','a.py']]),
        dict(id='report-line-field-collision',kind='report',bundles=[dict(key='a.py',value=[dict(hit,line=999)])]),
        dict(id='report-nonfinite-mixed',kind='report',bundles=[dict(key='a.py',value=[dict(hit,line_number='$NAN'),hit,dict(hit,details='$INF')])]),
        dict(id='report-tuple-namespace',kind='report',bundles=[dict(key='a.py',value={'$tuple':[hit]})]),
        dict(id='report-tuple-row-mixed',kind='report',bundles=[dict(key='a.py',value=[{'$tuple':[]},hit])]),
        dict(id='report-test-basename',kind='report',bundles=[dict(key='test_a.py',value=[hit]),dict(key='nested/a.py',value=[hit])]),
        dict(id='report-test-directory',kind='report',bundles=[dict(key='tests/a.py',value=[hit])]),
        dict(id='report-absolute',kind='report',bundles=[dict(key='$ROOT/a.py',value=[hit])]),
        dict(id='report-missing-family',kind='report',bundles=[dict(key='a.py',facts={})])])
    for i,(entries,signature) in enumerate([(None,'UNSIGNED'),(None,'bad'),([],None),(['changed'],None),([],''),([], 'NEEDS_SIGNING')]):
        rows.append(dict(id='report-signature-'+str(i),kind='report',bundles=[dict(key='a.py',value=[hit])],entries=entries,signature=signature))
    return rows


def component_value(module,case,scratch):
    root=scratch/'repo';root.mkdir()
    for name in case.get('directories',[]):(root/name).mkdir(parents=True)
    family=module.CONN_OR_TXN_UNION_EXTRACTOR.extractor_id
    try:
        if case['kind']=='metadata':
            plugin=module.conn_or_txn_union_plugin(root)
            value={f.name:getattr(plugin,f.name) for f in fields(plugin) if f.name!='reporter'}
            value.update(registry=list(module.conn_or_txn_union_fact_extractors(root)),allowlist=sorted(module.ALLOWLIST),
                signature=module._ALLOWLIST_SIGNATURE,connections=sorted(module.CONNECTION_TYPES),transactions=sorted(module.TRANSACTION_TYPES),source_type_triggers=module.SOURCE_TYPE_TRIGGERS)
        elif case['kind'] in {'extract','registry-extract'}:
            from scripts.lint_lib.fact_engine import materialize_source_facts
            bundle=(module._rerail_union_extract(root/'impl/a.py',case['source']) if case['kind']=='extract' and hasattr(module,'_rerail_union_extract') else materialize_source_facts(path=root/'impl/a.py',source=case['source'],specs=(module.CONN_OR_TXN_UNION_EXTRACTOR,),extractors=module.conn_or_txn_union_fact_extractors(root)))
            value=dict(facts=bundle.facts,errors=bundle.errors)
        elif case['kind']=='helper':
            collector=module.ConnectionTransactionCollector(root/'impl/a.py',_class_stack=case.get('stack',[]))
            argument=ast.parse(case['annotation'],mode='eval').body if 'annotation' in case else case['name']
            value=getattr(collector,case['entry'])(argument)
        elif case['kind']=='project':
            value=module._violation_to_fact(module.Violation(Path('a.py'),case['line'],'function','TYPE','details'))
        elif case['kind']=='report':
            from scripts.lint_lib.linter_plugin import PerFileFactBundle,ProjectFactView
            def hydrate(value):
                if isinstance(value,str):return {'$NAN':float('nan'),'$INF':float('inf')}.get(value,value)
                if isinstance(value,list):return [hydrate(v) for v in value]
                if isinstance(value,dict):
                    if list(value)==['$tuple']:return tuple(hydrate(v) for v in value['$tuple'])
                    return {k:hydrate(v) for k,v in value.items()}
                return value
            bundles={Path(b['key'].replace('$ROOT',str(root))):PerFileFactBundle(Path(b.get('path',b['key'])),'fixture',facts=hydrate(b.get('facts',{family:b.get('value')}))) for b in case['bundles']}
            with patch.object(module,'ALLOWLIST',set(case['entries']) if case.get('entries') is not None else module.ALLOWLIST),patch.object(module,'_ALLOWLIST_SIGNATURE',case['signature'] if case.get('signature') is not None else module._ALLOWLIST_SIGNATURE):
                plugin=module.conn_or_txn_union_plugin(root);value=plugin.reporter(ProjectFactView(root,bundles,plugin.required_facts))
        else:raise AssertionError(case['kind'])
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def exercise(module,case,scratch):
    out=io.StringIO();err=io.StringIO()
    with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):value=component_value(module,case,scratch)
    return dict(value,stdout=out.getvalue(),stderr=err.getvalue())


def install(backend,module,detectors=False):
    from conn_union_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':
    main(CHECK,'rerail.conn-union-components/v1',cases,exercise,install,['conn_union_components_contract.py','conn_union_contract.py','conn_union_port.py','ast_transport.py','fact_transport.py'],guard_modules=('scripts.lint_lib.signing',))
