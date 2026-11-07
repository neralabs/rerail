"""Generate recordable ownership contracts without synthesizing expected findings."""
import argparse,ast,json
from pathlib import Path
from data_layer_rr_text import module


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True)
    parser.add_argument('--destination',type=Path,required=True)
    parser.add_argument('--reporters',type=Path,required=True)
    args=parser.parse_args();source=args.source.resolve();original=module(source,'plan_scope_ownership');api=[];reports=[]
    base='l2000_connection/connection_plan_workflows.py'
    def add(identity,text,path=base,report=True,collector=False,tree=None):
        values=dict(root=dict(path='.'),path=dict(path=path),source=text)
        if collector:values['tree']=dict(pythonAst=tree if tree is not None else text,mode='exec')
        api.append(dict(id=identity,check='plan_scope_ownership',entry='collector' if collector else '_source_facts',
            files=[dict(path=path,source=text)],arguments=values))
        if report:reports.append(dict(id=identity,check='plan_scope_ownership',files=[dict(path=path,source=text)]))
    test=ast.parse((source/'scripts/tests/test_lint_plan_scope_ownership.py').read_text())
    original_cases=next(n.decorator_list[0].args[1] for n in test.body if isinstance(n,ast.FunctionDef) and n.name=='test_rejects_plan_scope_ownership_violations')
    for code,path,text in ast.literal_eval(original_cases):
        add('original-'+code,text,path)
        add('triggered-original-'+code,'# PlanScope\n'+text,path)
    for index,name in enumerate(sorted(original.BASE_SCOPE_RUNNERS)):
        expected='AsyncPlanScope' if name.endswith('_async') else 'SyncPlanScope'
        for tag,params in [('valid',f'scope: {expected}, workflow: W[T]'),('missing',''),('untyped','scope'),
            ('posonly',f'scope: {expected}, /'),('posonly-plus',f'ignored, /, scope: {expected}, workflow: W[T]'),
            ('no-workflow',f'scope: {expected}'),('plain-workflow',f'scope: {expected}, workflow: W'),
            ('quoted-workflow',f'scope: {expected}, workflow: "["')]:
            add(f'runner-{index}-{tag}',f'def {name}({params}):\n    return None\n')
    runner='run_read_plan_scope_workflow'
    for index,ann in enumerate(['Any','object','VersionedScope','Union[SyncPlanScope, Any]','SyncPlanScope | object',
        'typing.SyncPlanScope','"SyncPlanScope"','Anyish','list[int]','ｏｂｊｅｃｔ']):
        add(f'annotation-{index}',f'def {runner}(scope: {ann}, workflow: W[T]):\n    pass\n')
    for method in sorted(original.BASE_TERMINAL_FALLBACKS):
        add('terminal-'+method,f'def {runner}(scope: SyncPlanScope, workflow: W[T]):\n    return scope.{method}()\n')
    for method in ['getattr','hasattr','callable']:
        add('reflect-'+method,f'def {runner}(scope: SyncPlanScope, workflow: W[T]):\n    return namespace.{method}(scope, "x")\n')
    add('nested-runner-duplicates',f'def {runner}(scope: SyncPlanScope, workflow: W[T]):\n    def {runner}(scope: SyncPlanScope, workflow: W[T]):\n        return getattr(scope, "execute_read_workflow")\n')
    add('decorator-descendant',f'@getattr(scope,"x")\ndef {runner}(scope: SyncPlanScope, workflow: W[T]):\n    pass\n')
    for cls in ['SyncVersionedPlanScope','AsyncVersionedPlanScope']:
        for method in sorted(original.BASE_SCOPE_METHODS):
            add('versioned-'+cls+'-'+method,f'class {cls}:\n    @decorator\n    async def {method}(self):\n        pass\n','l7000_versioned_schema/versioned_connection.py')
        add('nested-method-'+cls,f'class {cls}:\n    class Inner:\n        def execute_read_workflow(self):\n            pass\n','l7000_versioned_schema/versioned_connection.py')
    for cls in ['SyncReadPlanWorkflow','SyncWritePlanWorkflow','AsyncReadPlanWorkflow','AsyncWritePlanWorkflow']:
        expected=cls.removesuffix('PlanWorkflow')+'Transaction'
        for tag,ann in [('valid',f'Callable[[{expected}], T]'),('qualified',f'typing.Callable[[{expected}], T]'),
            ('untyped','object'),('broad','Callable[[Any], T]'),('quoted',f'"Callable[[{expected}], T]"')]:
            add('workflow-'+cls+'-'+tag,f'class {cls}:\n    build: {ann}\n')
    owned='l7000_versioned_schema/versioned_connection.py'
    for method in sorted(original.NESTED_VERSIONED_TERMINALS):
        add('nested-terminal-'+method,f'def nested_versioned_context(scope):\n    return scope.{method}()\n',owned)
    for index,path in enumerate(sorted(original.PLAN_SCOPE_OWNERSHIP_FILES)):
        add(f'owned-path-{index}','def nested_versioned_context(scope):\n    return scope.fetch()\n',path)
    for tag,code in [('lambda','def nested_versioned_context(scope):\n    return lambda: scope.commit()\n'),
        ('nested-sync','def nested_versioned_context(scope):\n    def ordinary():\n        return scope.commit()\n'),
        ('class','def nested_versioned_context(scope):\n    class Inner:\n        value = scope.commit()\n')]:add('owner-'+tag,code,owned)
    for state in sorted(original.LIFECYCLE_NAMES):
        add('lifecycle-'+state,f'def helper(txn):\n    return getattr(txn, {state!r}, False)\n',owned)
    add('lifecycle-comment','def helper(txn):\n    return getattr(txn, "other", False) # _plan_owner\n',owned)
    add('lifecycle-escaped','def helper(txn):\n    return getattr(txn, "_pl\\u0061n_owner", False)\n',owned)
    add('lifecycle-split','def helper(txn):\n    return getattr(txn, "_base" + "_scope", False)\n',owned)
    for name in sorted(original.CALLBACK_OWNER_WRITERS):
        add('writer-'+name,f'def {name}(txn, value):\n    txn._callback_versioned_txn = value\n',owned)
    for tag,body in [('assign','txn._callback_versioned_txn = value'),('annotate','txn._callback_versioned_txn: object = value'),
      ('augment','txn._callback_versioned_txn += value'),('tuple','txn._callback_versioned_txn, x = value'),
      ('list','[txn._callback_versioned_txn, x] = value'),('for','for txn._callback_versioned_txn in value:\n    pass'),
      ('with','with value as txn._callback_versioned_txn:\n    pass'),('comprehension','result = [x for txn._callback_versioned_txn in value]'),
      ('load','return txn._callback_versioned_txn'),('delete','del txn._callback_versioned_txn')]:
        add('store-'+tag,'def random_owner(txn, value):\n    '+body.replace('\n','\n    ')+'\n',owned)
    for tag,call in [('name','factory(VersionedScope)'),('literal','factory("Versioned")'),
      ('comment','factory(x) # VersionedScope'),('escape','factory("\\x56ersioned")')]:add('versioned-'+tag,'# PlanScope\n'+call+'\n')
    for tag,callee in [('name','imported_contract_fn'),('attribute','owner.imported_contract'),
      ('subscript','registry["imported_contract"]'),('call','factory("imported_contract")')]:
        add('cte-'+tag,f'{callee}(CtePublicationContract)\n',owned)
    add('cte-dual','imported_contract = imported_contract_fn("CtePublicationContract") if callable(imported_contract_fn) else None\n',owned)
    add('cte-unrelated-else','# imported_contract = imported_contract_fn\ntext = "else None"\n',owned)
    original_tree=ast.parse((source/'scripts/lint_plan_scope_ownership.py').read_text())
    patterns=next(ast.literal_eval(n.value) for n in ast.walk(original_tree) if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='mismatch_patterns' for t in n.targets))
    for index,pattern in enumerate(patterns):
        for tag,text in [('exact',pattern),('double',pattern+'\n'+pattern),('indent',pattern.replace('            ','           '))]:
            add(f'raw-mode-{index}-{tag}','# PlanScope\ntext = """π '+text+'\n"""\n','conform/input.py')
    marker='if not txn.is_committed and not txn.is_rolled_back and not txn.did_publish_branch'
    add('raw-lifecycle-first-prefix','# if not txn.is_committed\n# PlanScope\ntext = """'+marker+'\n"""\n','conform/input.py')
    for index,text in enumerate(['def broken(:\n','# PlanScope\ndef broken(:\n','# PlanScope\nx = (\n','\x00# PlanScope\n']):
        add(f'parse-{index}',text,'impl/broken.py')
    positive='def run_read_plan_scope_workflow():\n    pass\n'
    for path in ['api/input.py','impl/input.py','data_layer/input.py','_internal/input.py','l1234_layer/input.py','l١٢٣٤_layer/input.py',
      'l1234_/input.py','l1234_layer\nmore/input.py','conform/input.py','data_layer_testing/input.py','scripts/input.py',
      'impl/tests/input.py','impl/test_input.py','impl/.mypy_cache/input.py','impl/venv/input.py','impl/__pycache__/input.py']:
        api.append(dict(id='scope-'+path,check='plan_scope_ownership',entry='lint',files=[dict(path=path,source=positive)],arguments=dict(root=dict(path='.'))))
        reports.append(dict(id='scope-'+path,check='plan_scope_ownership',files=[dict(path=path,source=positive)]))
    for identity,files in [('empty',[]),('multiple',[dict(path='conform/b.py',source=positive),dict(path='l2000_layer/a.py',source=positive)]),
      ('bad-encoding',[dict(path='impl/bad.py',bytesHex='fffe')]),('excluded-bad-encoding',[dict(path='impl/.venv/bad.py',bytesHex='fffe')])]:
        api.append(dict(id=identity,check='plan_scope_ownership',entry='lint',files=files,arguments=dict(root=dict(path='.'))))
    for index,(text,tree) in enumerate([(positive,positive),('not valid python',positive),('# PlanScope\n'+marker+'\n','pass\n'),
      ('# imported_contract = imported_contract_fn\n# else None\n','imported_contract_fn(CtePublicationContract)\n')]):
        add(f'collector-{index}',text,'impl/collector.py',report=False,collector=True,tree=tree)
    for code,path,tree in ast.literal_eval(original_cases):
        add('collector-original-'+code,'pass\n',path,report=False,collector=True,tree=tree)
    args.destination.write_text(json.dumps(dict(cases=api),indent=2)+'\n');args.reporters.write_text(json.dumps(dict(cases=reports),indent=2)+'\n')
    print(len(api),'API cases;',len(reports),'reporter cases')


if __name__=='__main__':main()
