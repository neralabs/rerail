"""Record exact plan-scope collector traversal, receiver spelling, audit state and fact rows."""
import ast
from dataclasses import MISSING,asdict,fields
from pathlib import Path
from contract_runner import main
from dto import canonical

CHECK='plan_scope_side_effects'
CONSTANTS=['SKIP_DIRS','SIDE_EFFECT_CALLS','SIDE_EFFECT_RECEIVERS','LEGACY_CALLBACK_METHODS','EAGER_VALUE_ATTRS',
           'EAGER_CALLS','TYPED_EAGER_QUERY_TERMINALS','RAW_DB_CALLS','PLAN_OPENERS','WRITE_PLAN_OPENERS']
BODY='lazy.resolved_value\nlazy.resolve()\nconn.execute_raw()\ntxn.write(cb)\nrequests.get(url)\nn=query.count()\nif n:\n pass\np.execute()\n'


def scope(body,opener='open_write_plan',alias='p',async_=False):
    return ('async 'if async_ else '')+'with '+opener+'()'+(' as '+alias if alias else '')+':\n'+''.join(' '+line+'\n'for line in body.splitlines())


def cases(module):
    rows=[dict(kind='metadata',id='metadata')]
    for opener in sorted(module.PLAN_OPENERS):
        for async_ in [False,True]:rows.append(dict(kind='extract',id=f'opener-{opener}-{async_}',source=scope(BODY,opener,async_=async_)))
    for receiver in sorted(module.SIDE_EFFECT_RECEIVERS):
        for name in sorted(module.SIDE_EFFECT_CALLS):rows.append(dict(kind='extract',id=f'effect-{receiver}-{name}',source=scope(receiver+'.'+name+'(value)\n')))
    for name in sorted(module.SIDE_EFFECT_CALLS):rows.append(dict(kind='extract',id='bare-'+name,source=scope(name+'(value)\n')))
    patterns=dict(
        clean='pass\n',trigger_comment='# open_write_plan\nrequests.get(url)\n',syntax='with open_write_plan(:\n pass\n',
        outside=BODY,alias='import requests as r\n'+scope('r.get(url)\n'),
        from_alias='from time import sleep\n'+scope('sleep(1)\n'),
        unbound=scope('requests.get(url)\n'),shadowed_open=scope('open = custom\nopen(value)\n'),
        receiver_arguments=scope('factory(requests).get(url)\n'),receiver_subscript=scope('requests["x"].get(url)\n'),
        receiver_method=scope('client.requests.get(url)\n'),receiver_case=scope('Requests.get(url)\n'),
        receiver_unicode=scope('ｒｅｑｕｅｓｔｓ.get(url)\n'),
        rebinding=scope('n=q.count()\nn=0\nif n: pass\n'),
        late_assignment=scope('if n: pass\nn=q.count()\n'),
        chained_assignment=scope('a=b=q.count()\nif a or b: pass\n'),
        unpacking=scope('a,(b,*rest)=q.execute()\nif a or b or rest: pass\n'),
        starred_only=scope('*rest,=q.count()\nif rest: pass\n'),
        annotated=scope('n:int=q.count()\nif n: pass\n'),
        annotated_empty=scope('n:int\nif n: pass\n'),
        walrus=scope('if (n:=q.count()): pass\nif n: pass\n'),
        direct_test=scope('if q.count(): pass\n'),
        nested_test=scope('if q.count() > 0: pass\n'),
        allowed_execute=scope('n=p.execute()\nif n: pass\n'),
        direct_scope_test=scope('if p.execute(): pass\n'),
        elif_=scope('n=q.count()\nif False: pass\nelif n: pass\n'),
        function_scope=scope('def nested():\n n=q.count()\nif n: pass\n'),
        function_defaults=scope('def nested(x=lazy.resolve()):\n pass\n'),
        class_scope=scope('class Nested:\n n=q.count()\nif n: pass\n'),
        normal_with=scope('with normal():\n lazy.resolve()\n'),
        nested_read=scope('n=q.count()\n'+scope('if n: pass\n','open_read_plan','r')+'if n: pass\n'),
        nested_write=scope('n=q.count()\n'+scope('if n: pass\nm=q.count()\nif m: pass\n','open_write_plan','r')+'if n: pass\nif m: pass\n'),
        header=scope('pass\n',opener='obj.open_write_plan',alias='p')+ 'pass\n',
        header_effect='with open_write_plan(lazy.resolve()) as p:\n pass\n',
        header_name='with open_write_plan() as p, normal(lazy.resolve()) as other:\n pass\n',
        tuple_alias=scope('p.execute()\n',alias='(p,other)'),
        same_line=scope('lazy.resolve(); lazy.resolve()\n'),
        nested_attribute=scope('lazy.resolve().resolved_value\n'),
        spelling_gate=scope('lazy.resolve()\n',opener='open_wr𝚒te_plan'),
        spelling_gate_present='# open_write_plan\n'+scope('lazy.resolve()\n',opener='open_wr𝚒te_plan'),
    )
    rows.extend(dict(kind='extract',id='shape-'+name,source=source)for name,source in patterns.items())
    for names,write in [([],False),(['p'],False),([],True),(['p'],True)]:
        for name,tree in [('plain',BODY),('nested',scope(BODY,'open_read_plan','r')),('no-read','if n: pass\n')]:
            rows.append(dict(kind='collector',id=f'seed-{names}-{write}-{name}',names=names,write=write,source='caller source differs\n',tree=tree))
    rows.extend(dict(kind='collector',id='provided-'+name,names=[],write=False,source=source,tree=tree)for name,source,tree in [
        ('empty',scope(BODY),''),('positive','pass\n',scope(BODY)),('syntax','not valid Python !!!',scope(BODY))])
    return rows


def drive(collector,tree,path,source):
    from scripts.lint_lib.ast_collectors import AstTraversalContext
    parents={};children={};todo=[(tree,None)]
    while todo:
        node,parent=todo.pop();parents[node]=parent;children[node]=tuple(ast.iter_child_nodes(node));todo.extend((c,node)for c in reversed(children[node]))
    context=AstTraversalContext(path,source,parents,children);collector.start_file(path,source,context)
    pending=[(tree,False)]
    while pending:
        node,exit_=pending.pop();name=('on_exit_'if exit_ else 'on_')+type(node).__name__
        generic=collector.on_exit_node if exit_ else collector.on_node;generic(node,context)
        specific=getattr(collector,name,None)
        if specific:specific(node,context)
        if not exit_:pending.append((node,True));pending.extend((c,False)for c in reversed(children[node]))


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();path=root/'impl/a.py'
    try:
        if case['kind']=='metadata':
            value=dict(constants={n:getattr(module,n)for n in CONSTANTS},extractor=asdict(module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR),
                violation=[dict(name=f.name,type=str(f.type),required=f.default is MISSING,default=None if f.default is MISSING else f.default)for f in fields(module.Violation)],
                state=[dict(name=f.name,type=str(f.type))for f in fields(module._PlanScopeState)])
        elif case['kind']=='extract':
            from scripts.lint_lib.fact_engine import materialize_source_facts
            result=materialize_source_facts(path=path,source=case['source'],specs=(module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR,),extractors=module.plan_scope_side_effects_fact_extractors(root))
            value=dict(facts=result.facts,errors=result.errors)
        else:
            collector=module.FileCollector('caller.py',set(case['names']),is_write_plan=case['write'])
            drive(collector,ast.parse(case['tree']),path,case['source']);facts=collector.finish_file()
            value=dict(facts=facts,violations=collector.violations,stack=collector._scope_stack,current=collector._current_scope,path=collector.rel_path)
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    pass


if __name__=='__main__':main(CHECK,'rerail.side-effects-collector/v1',cases,exercise,install,['side_effects_collector_contract.py'])
