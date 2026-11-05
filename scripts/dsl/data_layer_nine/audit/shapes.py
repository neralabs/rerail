# SPDX-License-Identifier: Apache-2.0
"""Small source variations for differential checks against the original linters."""


def cases():
    results=[]
    def add(check,entry,source,path):
        results.append(dict(test='shape-'+str(len(results)),check=check,entry=entry,files=[dict(path=path,source=source)],parameters={}))
    for annotation in ['Sequence[Widget]','typing.Sequence[Widget]','Sequence[Widget | Other]','tuple[Any, Mapping[str, Any]]','Sequence[Literal["Widget"]]','"SyncSchemaTransaction"','"typing.Sequence[Widget]"','tuple[AsyncSchemaTransaction,SyncSchemaTransaction]','Annotated[Sequence[Widget], Other]']:
        for name in ['public','private_lazy']:
            add('schema_bootstrap_contracts','lint_file',f'@pure_lazy\ndef {name}(value: {annotation}) -> None:\n    pass\n','l3200_metadata/bootstrap_state.py')
    for body in ['pass','...','return 1','raise RuntimeError()','return helper()','first()\n        return last()','x = 1\n        return x','class Nested:\n            def other(self):\n                return 1']:
        add('connection_service_architecture','analyze_source','class One:\n    def table_exists(self):\n        '+body+'\nclass Two:\n    def table_exists(self):\n        value = 1\n        return value\n','l2000_connection/probe.py')
    for statement in ["key = '_current_schema_transaction'; getattr(obj,key)","key = '_current_schema_transaction'; key = 'other'; getattr(obj,key)","key: str = '_current_schema_transaction'; getattr(obj,key)","getattr(obj, key := '_current_schema_transaction')","class Nested:\n        key = '_current_schema_transaction'\n        getattr(obj,key)","def nested():\n        key = '_current_schema_transaction'\n    getattr(obj,key)"]:
        add('connection_service_architecture','analyze_source','def outer():\n    '+statement+'\n','l2000_connection/probe.py')
    for decorator in ['fixture','pytest.fixture','suite.pytest.fixture','notpytest.fixture','pytest.fixture()']:
        for name in ['helper','test_example']:
            add('bootstrap_cache_health','scan_file',f'@{decorator}\ndef {name}():\n    (create_tables)()\n','conform/test_probe.py')
    for annotation in ['SchemaTransaction','"SchemaTransaction"','list[SchemaTransaction]','FooSchemaTransaction','SchemaTransaction | None']:
        add('schema_architecture','scan_source',f'def outer(txn: {annotation}):\n    plan = txn._schema_operation_plan\n    plan.append(1)\n    execute()\n','l2000_connection/probe.py')
    for annotation in ['dict','list[int]','typing.dict[str,int]','"list[int]"','set[int]']:
        add('plan_composition_architecture','scan_semantic_source',f'class SemanticPlanBuilder:\n    def build(self) -> {annotation}:\n        self.execute()\n','l2070_batching/semantic_plan.py')
    for number in ['0','0.0','False','0j','1','1.0','1j','1e999','1e999j']:
        add('bootstrap_engine_invariants','_collect_fake_executor_violations',f'def execute_application_plan():\n    return BootstrapExecutionFacts(mutation_counts=BootstrapMutationCounts(a={number},b={number},c={number},d={number}))\n','l3200_metadata/bootstrap_state.py')
    for lifecycle in ['absent','invalid','empty']:
        files=[dict(path='l2000_connection/connection.py',source='class _ConnectionLifecycleMixin:\n    def __enter__(self):\n        self.connect()\n')]
        if lifecycle!='absent':files.append(dict(path='l2000_connection/connection_lifecycle.py',source='def broken(\n' if lifecycle=='invalid' else ''))
        results.append(dict(test='lifecycle-'+lifecycle,check='connection_structural_boundaries',entry='lint',files=files,parameters={}))
    return results
