"""Source-first contracts for every syntax helper in the plan-scope collector."""
import ast
from pathlib import Path
from dto import canonical
from contract_runner import main
from side_effects_collector_contract import drive,scope
CHECK='plan_scope_side_effects'


def cases(module):
    rows=[]
    for helper in ['_call_name','_receiver_tokens','_assigned_names','_referenced_names']:
        for expr in ['open','requests.get','client.requests.get','factory(requests).get','requests[x].get','a.a.a','(a,b)','[a,[b,*c]]','(a:=b)','q.count()','lambda:requests.get(url)','ｒｅｑｕｅｓｔｓ.get']:
            rows.append(dict(kind='syntax',helper=helper,source=expr))
    for expr in ['open_write_plan()','obj.open_provider_read_plan()','normal()','open_write_plan','factory[open_write_plan]()']:
        for alias in ['p','(p)','(p,q)','[p]','obj.p','']:
            for helper in ['_is_plan_scope_item','_is_write_plan_scope_item','_scope_name']:
                rows.append(dict(kind='item',helper=helper,source='with '+expr+(' as '+alias if alias else '')+': pass\n'))
    for expr in ['open()','sleep(1)','requests.get(url)','client.requests.get(url)','factory(requests).get(url)','r.get(url)','p.execute()','p.tx.execute()','q.count()','not_call','Requests.get(url)']:
        for helper in ['_is_allowed_scope_execute','_is_typed_eager_read_call','_looks_like_python_side_effect']:
            if helper in ['_is_allowed_scope_execute','_looks_like_python_side_effect'] and expr=='not_call':continue
            rows.append(dict(kind='call',helper=helper,source=expr,names=['p'],write=True))
    for names,write,eager in [([],False,[]),([],True,[]),(['p'],False,['n']),(['p'],True,['n'])]:
        for expr in ['q.count()','n','n > 0','q.count() > 0','(n:=0)','other','lambda:n']:
            rows.append(dict(kind='branch',helper='_branches_on_eager_read',source=expr,names=names,write=write,eager=eager))
    return rows


def context(tree):
    from scripts.lint_lib.ast_collectors import AstTraversalContext
    parents={};children={};todo=[(tree,None)]
    while todo:
        n,p=todo.pop();parents[n]=p;children[n]=tuple(ast.iter_child_nodes(n));todo.extend((c,n)for c in reversed(children[n]))
    return AstTraversalContext(Path('caller.py'),'different source',parents,children)


def exercise(module,case,scratch):
    try:
        tree=ast.parse(case['source']).body[0].items[0] if case['kind']=='item' else ast.parse(case['source'],mode='eval').body
        name=case['helper']
        if case['kind']in {'syntax','item'}:
            result=getattr(module,name)(tree,context(tree))if name=='_referenced_names'else getattr(module,name)(tree)
        elif name=='_is_allowed_scope_execute':result=module._is_allowed_scope_execute(tree,set(case['names']))
        else:
            collector=module.FileCollector('caller.py',set(case['names']),is_write_plan=case['write'])
            if case['kind']=='branch' and collector._current_scope is not None:collector._current_scope.eager_read_names.update(case['eager'])
            result=getattr(collector,name)(tree,context(tree))if case['kind']=='branch'else getattr(collector,name)(tree)
        return dict(value=canonical(result,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from side_effects_port import install as configure
    configure(backend,module)

if __name__=='__main__':main(CHECK,'rerail.side-effects-helpers/v1',cases,exercise,install,['side_effects_helper_contract.py','side_effects_collector_contract.py','side_effects_port.py','ast_transport.py'])
