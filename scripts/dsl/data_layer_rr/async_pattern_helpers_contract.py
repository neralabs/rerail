"""Record async collector helper results, state aliases, path rooting, and canonical rendering."""
import ast
from pathlib import Path
from contract_runner import main
from dto import canonical
from replay import write_fixture
from async_pattern_components_contract import CHECK,BAD

STATE=['excluded','in_scope','in_impl','in_api','current_class','in_abstract_class','in_async_write_txn','in_async_read_txn','in_async_function','current_async_function_name','async_txn_name_stack','async_session_name_stack','async_query_name_stack','_class_state_stack','_async_function_state_stack']


def cases(_module):
    rows=[]
    for i,path in enumerate(['api/a.py','impl/a.py','conform/a.py','other/a.py','api/test_a.py',r'api\a.py','myapi/a.py','tests/api/a.py']):rows.append(dict(kind='initial',id='initial-'+str(i),path=path))
    for line in [-2,0,1,3,8]:
        for radius in [-2,0,1,2,5]:rows.append(dict(kind='context',id=f'context-{line}-{radius}',line=line,radius=radius))
    for i,text in enumerate(['ReadTransaction','pkg.ReadTransaction','list[ReadTransaction | None]','tuple[Other, ReadTransaction]','"prefixReadTransactionSuffix"','"Other"','A + ReadTransaction','(ReadTransaction,)','list[Other]','None','"読み取り"','読み取り']):
        rows.append(dict(kind='annotation',id='annotation-'+str(i),expression=text,names=['ReadTransaction','読み取り']))
    rows.append(dict(kind='annotation',id='annotation-none',expression=None,names=['ReadTransaction']))
    for i,signature in enumerate(['txn','other','tx, /, x: Other, *args: ReadTransaction, named: ReadTransaction, **kwargs: Other','transaction: "ReadTransaction | None"','x: pkg.WriteTransaction','x: list[ReadTransaction], *, rtxn','x: AsyncReadTransaction','x: "prefixReadTransactionSuffix"']):rows.append(dict(kind='infer',id='infer-'+str(i),source='async def work('+signature+'):\n    pass\n'))
    expressions=['txn','txn.query(Row).filter(active=True)','txn[0].x()','other','make()[0].x','(a,b)','conn.session()','conn.session','conn.session().query(Row)','txn.query(Row).execute()','other.query(Row)','txn.cross_branch(name).query(Row)','[x for x in txn]']
    for i,text in enumerate(expressions):
        for kind in ['receiver','session','query']:rows.append(dict(kind=kind,id=kind+'-'+str(i),expression=text,txn=['txn']))
    for i,text in enumerate(['s.write_async()','s.read_async()','write_async()','s.write_async','s.write_async().other()','pkg.s.write_async(x)']):
        for name in ['write_async','read_async']:rows.append(dict(kind='decorator',id=f'decorator-{i}-{name}',expression=text,name=name))
    for kind in ['txn','session','query']:
        for i,stack in enumerate([[],[[]],[['outer'],['inner']]]):rows.append(dict(kind='getter',id=f'getter-{kind}-{i}',getter=kind,stack=stack))
    for i,text in enumerate(['txn.query(Row).filter(active=True)','(a, b)','lambda x: x + 1','{x: y for x,y in z}','f"hello {x!r}"','(x := y)','[a, *b]','obj[1:3:2]','"☃"','',None]):rows.append(dict(kind='render',id='render-'+str(i),expression=text))
    for i,line in enumerate([0,1,3,6,30]):rows.append(dict(kind='marked',id='marked-'+str(i),line=line))
    for i,markers in enumerate([[],['pyproject.toml'],['api/pyproject.toml'],['api/sub/pyproject.toml'],['api/pyproject.toml','api/sub/pyproject.toml']]):rows.append(dict(kind='root',id='root-'+str(i),files=[dict(path=p,source='') for p in markers],path='api/sub/a.py'))
    rows.append(dict(kind='root',id='root-directory-marker',files=[dict(path='api/pyproject.toml',directory=True)],path='api/sub/a.py'))
    rows.append(dict(kind='root',id='root-file-link',files=[dict(path='other/a.py',source='pass\n'),dict(path='other/pyproject.toml',source=''),dict(path='api/link.py',symlink='../other/a.py')],path='api/link.py'))
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();source='line one\n# ASYNC_FALLBACK:\nline three\nline four\nline five\n';path=root/case.get('path','api/a.py')
    try:
        if case['kind']=='root':write_fixture(root,case['files']);value=module._analysis_root_for_file(path)
        else:
            owner=module.AsyncPatternCollector(path,source,root)
            def expression():return ast.parse(case['expression'],mode='eval').body
            if case['kind']=='initial':value={k:getattr(owner,k) for k in STATE}
            elif case['kind']=='context':value=owner._get_context(case['line'],case['radius'])
            elif case['kind']=='annotation':value=owner._annotation_contains_type_name(None if case['expression'] is None else expression(),frozenset(case['names']))
            elif case['kind']=='infer':value=owner._infer_transaction_param_names(ast.parse(case['source']).body[0])
            elif case['kind']=='receiver':value=owner._receiver_root_name(expression())
            elif case['kind']=='session':value=owner._is_session_factory_expr(expression())
            elif case['kind']=='query':owner.async_txn_name_stack=[set(case['txn'])];value=owner._is_query_chain_from_txn(expression())
            elif case['kind']=='decorator':value=owner._is_async_decorator(expression(),case['name'])
            elif case['kind']=='getter':
                key='async_'+case['getter']+'_name_stack';setattr(owner,key,[set(v) for v in case['stack']]);result=getattr(owner,'_get_async_'+case['getter']+'_names')();result.add('added');value=dict(value=result,stack=getattr(owner,key),same_object=bool(getattr(owner,key)) and result is getattr(owner,key)[-1])
            elif case['kind']=='render':
                node=ast.Name() if case['expression'] is None else ast.AST() if case['expression']=='' else expression();value=owner._expr_text(node)
            elif case['kind']=='marked':value=owner._is_marked_fallback(case['line'])
            else:raise ValueError('Unknown async helper contract')
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from async_pattern_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.async-pattern-helpers/v1',cases,exercise,install,['async_pattern_helpers_contract.py','async_pattern_components_contract.py'])
