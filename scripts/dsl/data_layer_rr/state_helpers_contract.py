"""Source-recorded State helper selection, identity, ordering, and cache effects."""
import ast
from pathlib import Path

from contract_runner import main

CHECK='serverplan_state_tripwires'


def cases(_module):
    result=[]
    def add(name,source,**extra):
        result.append(dict(id=name,kind='helpers',source=source,**extra))
    add('empty','pass\n')
    add('class-scope','def first():\n    pass\nclass A:\n    async def method(self):\n        pass\n    class B:\n        def nested(self):\n            pass\ndef last():\n    pass\n',stack=['outer.name','quoted"name'])
    add('control-and-function-bodies','if enabled:\n    def hidden():\n        pass\nclass A:\n    if enabled:\n        def hidden(self):\n            pass\ndef compile():\n    def nested():\n        pass\n    return 1\n')
    add('closure','def compile():\n    return helper()\ndef helper():\n    return last()\ndef last():\n    return 1\ndef unused():\n    return 2\n')
    add('leaf-name-methods','class A:\n    def helper(self):\n        pass\nclass B:\n    def helper(self):\n        pass\ndef compile(obj):\n    return obj.helper()\n')
    add('cycle','def compile():\n    return helper()\ndef helper():\n    return compile()\n')
    add('no-roots','def a():\n    return b()\ndef b():\n    return 2\n')
    for deco in ['pure_lazy','pkg.pure_lazy','pkg.pure_lazy(flag=True)']:
        add('decorator-'+deco,'@'+deco+'\ndef build():\n    return helper()\ndef helper():\n    return 1\n')
    add('pending-and-effects','def pending_pipeline_effects():\n    pass\ndef apply_write_effect():\n    pass\ndef ordinary_lazy():\n    pass\n')
    for annotation in ['CompilationContext','pkg.CompilationContext','"CompilationContext"']:
        add('helper-'+annotation,f'def make_sql_builder(ctx: {annotation}):\n    return 1\n')
    for signature in ['runtime_refs, spec: RuntimeBuilderSpec','runtime_refs, spec: "RuntimeBuilderSpec"','*runtime_refs, spec: RuntimeBuilderSpec','**runtime_refs','runtime_refs, /, *, spec: pkg.RuntimeBuilderSpec']:
        add('materializer-'+signature,f'def convert({signature}):\n    pass\n')
    add('materializer-name','def materialize_x_builder(runtime_refs):\n    pass\n')
    add('provider','def score_components():\n    pass\ndef ordinary():\n    pass\n',path='impl/common/l2070_server_plan/backend_profiles.py')
    for path in ['impl/common/example.py','impl/common/l2070_server_plan/example.py']:
        add('native-'+path,'def compute():\n    return np.array(value)\ndef other():\n    return 1\n',path=path)
    add('explicit-native-empty','def compute():\n    return np.array(value)\n',path='impl/common/l2070_server_plan/example.py',native=[])
    add('explicit-native-outside-path','def compute():\n    return helper()\ndef helper():\n    return 1\n',native=['compute'])
    add('supplied-calls','def compile():\n    return wrong()\ndef helper():\n    return 1\ndef wrong():\n    return 2\n',called={'compile':['helper'],'helper':[]},native=[])
    add('supplied-empty-calls','def compile():\n    return helper()\ndef helper():\n    return 1\n',called={},native=[])
    add('cached-empty-body','def compile():\n    return helper()\ndef helper():\n    return 1\n',emptyCache=['compile'],native=[])
    add('cached-alternate-body','def compile():\n    return wrong()\ndef helper():\n    return 1\ndef wrong():\n    return 2\n',alternateCache={'compile':'helper()'},native=[])
    add('duplicate-identity-last-class','def compile():\n    return helper()\ndef helper():\n    return 1\n',duplicate=True)
    add('mutated-tree-name','def ordinary():\n    return helper()\ndef helper():\n    return 1\n',rename={'ordinary':'compile'})
    return result


def exercise(module,case,_scratch):
    tree=ast.parse(case['source'])
    # Apply caller AST changes before invoking either implementation.
    for node in ast.walk(tree):
        if isinstance(node,(ast.FunctionDef,ast.AsyncFunctionDef)):
            node.name=case.get('rename',{}).get(node.name,node.name)
    collected=module._collect_function_nodes(tree.body,class_stack=tuple(case.get('stack',[])))
    identities={id(fn):i for i,(fn,_) in enumerate(collected)}
    functions=list(collected)
    if case.get('duplicate'):functions.append((functions[0][0],('last','scope')))
    cache={fn:[] for fn,_ in functions if fn.name in case.get('emptyCache',[])}
    for fn,_ in functions:
        if fn.name in case.get('alternateCache',{}):
            cache[fn]=list(ast.walk(ast.parse(case['alternateCache'][fn.name])))
    initial={fn:list(nodes) for fn,nodes in cache.items()}
    originals=[];pending=[tree]
    while pending:
        node=pending.pop();originals.append(node);pending.extend(reversed(list(ast.iter_child_nodes(node))))
    for values in initial.values():
        for root in values:
            pending=[root]
            while pending:
                node=pending.pop();originals.append(node);pending.extend(reversed(list(ast.iter_child_nodes(node))))
    # Identity labels compare source input objects, never a reconstructed AST.
    refs={id(n):i for i,n in enumerate(originals)}
    kwargs={}
    if 'native' in case:kwargs['native_python_work_functions']=frozenset(fn for fn,_ in functions if fn.name in case['native'])
    if 'called' in case:kwargs['called_names_by_fn']={fn:frozenset(case['called'][fn.name]) for fn,_ in functions if fn.name in case['called']}
    original_walk=ast.walk
    if 'called' in case and 'native' in case:
        def reject_walk(_):raise AssertionError('supplied facts must not trigger ast.walk')
        ast.walk=reject_walk
    try:
        selected=module._select_reachable_function_nodes(Path(case.get('path','impl/common/example.py')),functions,cache,**kwargs)
    finally:ast.walk=original_walk
    return dict(collected=[[identities[id(fn)],fn.name,list(stack)] for fn,stack in collected],
        selected=[[identities[id(fn)],fn.name,list(stack)] for fn,stack in selected],
        cache=sorted([identities[id(fn)],[refs[id(n)] for n in nodes]] for fn,nodes in cache.items()),
        retainedCaches=all(cache[fn] is not None and all(a is b for a,b in zip(cache[fn],nodes)) and len(cache[fn])==len(nodes) for fn,nodes in initial.items()))


def install(backend,module,*,detectors):
    from state_tripwires_port import install as install_helpers
    install_helpers(backend,module)


if __name__=='__main__':
    main(CHECK,'rerail.state-helpers-contract/v1',cases,exercise,install,
         ['state_helpers_contract.py','state_tripwires_port.py','ast_transport.py'])
