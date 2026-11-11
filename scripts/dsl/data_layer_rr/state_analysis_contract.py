"""Compare actual function-analysis scopes and complete State Tripwires findings."""
from pathlib import Path

from contract_runner import main
from dto import canonical
from state_tripwires_contract import install as install_scanners

CHECK='serverplan_state_tripwires'


def cases(_module):
    result=[]
    def add(name,source,path='impl/common/example.py'):
        result.append(dict(id=name,kind='analysis',path=path,source=source))
    add('reachable-only','def compile():\n    return helper()\ndef helper():\n    return 1\n'+
        ''.join(f'def unrelated_{i}():\n    return {i}\n' for i in range(50)))
    add('leaf-name-selection','def helper():\n    return 1\ndef compile(obj):\n    return obj.helper()\n')
    add('same-name-methods','class A:\n    def helper(self):\n        return 1\nclass B:\n    def helper(self):\n        return 2\nclass Step:\n    def compile(self):\n        return self.helper()\n')
    add('module-if-is-not-a-context','if enabled:\n    def compile():\n        return 1\n')
    add('class-if-is-not-a-context','class Step:\n    if enabled:\n        def compile(self):\n            return 1\n')
    add('nested-function-stays-in-owner','def compile():\n    def nested():\n        return helper()\n    return nested()\ndef helper():\n    return 1\n')
    for name,deco in [('bare','pure_lazy'),('qualified','pkg.pure_lazy'),('call','pure_lazy()'),
                      ('qualified-call','pkg.pure_lazy(value=True)'),('spaced','pkg . pure_lazy')]:
        add('decorator-'+name,f'@{deco}\ndef build():\n    return 1\n')
    add('runtime-builder-spec','def convert(spec: RuntimeBuilderSpec, runtime_refs):\n    return 1\n')
    add('runtime-builder-name','def materialize_x_builder(runtime_refs):\n    return 1\n')
    add('quoted-builder-type-is-not-an-ast-name','def convert(spec: "RuntimeBuilderSpec", runtime_refs):\n    return 1\n')
    add('context-helper','def make_sql_builder(ctx: CompilationContext):\n    return 1\n')
    add('quoted-context-type-is-not-an-ast-name','def make_sql_builder(ctx: "CompilationContext"):\n    return 1\n')
    add('provider-helper','def get_components():\n    return 1\n','impl/common/l2070_server_plan/backend_profiles.py')
    for name,call in [('plain','json.dumps(value)'),('spaced','json . dumps(value)'),('parenthesized','(json.dumps)(value)')]:
        add('native-work-'+name,f'def payload(value):\n    return {call}\n')
    add('decorator-native-work','@json.dumps(value)\ndef ordinary():\n    return 1\n')
    add('dead-authority-function','def compile():\n    return helper()\ndef helper():\n    return 1\ndef unrelated(hydrator):\n    return hydrator.txn\n')
    add('non-root-function-before-root','def before():\n    return 1\ndef compile():\n    return 2\n')
    add('nfkc-function-name','# def compile\ndef ｃompile():\n    return helper()\ndef helper():\n    return 1\n')
    for name,call in [('array','np.array(value)'),('spaced-array','np . array(value)'),
                      ('parenthesized-array','(np.array)(value)'),('factory','create_usearch_index_class()')]:
        add('native-root-'+name,f'# np.array\ndef ordinary(value):\n    return {call}\n')
    add('native-decorator-root','@np.array(value)\ndef ordinary():\n    return 1\n')
    return result


def exercise(module,case,_scratch):
    path=Path(case['path']);source=case['source']
    if backend:=getattr(module,'_rr_analysis_backend',None):
        from state_tripwires_port import analysis_contexts
        scopes=analysis_contexts(backend,module,path,source)
        findings=module.scan_source(path,source)
    else:
        facts=[];runtime=set()
        original_facts=module._collect_function_facts
        original_runtime=module._function_returns_runtime_authority
        def capture_facts(*args,**kwargs):
            nodes=kwargs['function_nodes']
            assert isinstance(nodes,list)
            facts.extend([fn.lineno,fn.name,'.'.join(classes)] for fn,classes in nodes)
            return original_facts(*args,**kwargs)
        def capture_runtime(fn,**kwargs):
            runtime.add((fn.lineno,fn.name))
            return original_runtime(fn,**kwargs)
        module._collect_function_facts=capture_facts
        module._function_returns_runtime_authority=capture_runtime
        try:findings=module.scan_source(path,source)
        finally:
            module._collect_function_facts=original_facts
            module._function_returns_runtime_authority=original_runtime
        scopes=dict(facts=sorted(facts),runtime=[list(v) for v in sorted(runtime)])
    return dict(scopes=scopes,findings=canonical(findings))


def install(backend,module,*,detectors):
    install_scanners(backend,module,detectors=detectors)
    module._rr_analysis_backend=backend


if __name__=='__main__':
    main(CHECK,'rerail.state-analysis-contract/v1',cases,exercise,install,
         ['state_analysis_contract.py','state_tripwires_port.py','state_tripwires_contract.py'])
