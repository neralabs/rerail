from pathlib import Path
import pytest


@pytest.mark.parametrize('source', ['pass\n\n# COLD_APPLICATION\n','\n\n# COLD_APPLICATION\npass\n',"text = '''first\né COLD_APPLICATION\n'''\n"])
def test_raw_surface_locations_preserve_the_complete_source(tmp_path,source):
 from scripts import lint_bootstrap_engine_invariants as lint
 path=tmp_path/'l3200_metadata/bootstrap_state.py';path.parent.mkdir(parents=True)
 path.write_text(source)
 result=lint._collect_file_violations(path)
 assert [(row.code,row.line) for row in result]==[('BEI035',2 if source.startswith('text') else 3)]


def test_cache_health_configured_plugin_scans_helper_modules(tmp_path):
 from scripts import lint_bootstrap_cache_health as lint
 from scripts.lint_lib.fact_engine import FactEngine
 from scripts.lint_lib.fact_store import JsonFactStore
 path=tmp_path/'conform/helpers.py';path.parent.mkdir(parents=True)
 path.write_text('def test_probe():\n    create_tables()\n')
 plugin=lint.bootstrap_cache_health_plugin(tmp_path)
 engine=FactEngine(root=tmp_path,fact_store=JsonFactStore(tmp_path/'cache'),extractors=lint.bootstrap_cache_health_fact_extractors(tmp_path))
 view=engine.materialize((plugin,),max_workers=1)
 assert [row.code for row in plugin.reporter(view)]==['BCH003']


def test_schema_contracts_do_not_treat_lambda_arguments_as_function_parameters(tmp_path):
 from scripts import lint_schema_bootstrap_contracts as lint
 path=tmp_path/'bootstrap.py'
 path.write_text('def outer(value: int):\n    return lambda untyped: untyped\n')
 assert lint.lint_file(path)==()


def test_schema_contracts_do_not_treat_lambda_defaults_as_function_parameters(tmp_path):
 from scripts import lint_schema_bootstrap_contracts as lint
 path=tmp_path/'bootstrap.py'
 path.write_text('def outer(value: int = (lambda untyped: untyped)(1)):\n    pass\n')
 assert lint.lint_file(path)==()


def test_bootstrap_tail_lambda_does_not_satisfy_the_required_function_parameter(tmp_path,monkeypatch):
 from scripts import lint_bootstrap_engine_invariants as lint
 path=tmp_path/'l2000_connection/schema_transaction.py';path.parent.mkdir(parents=True)
 path.write_text('@pure_lazy\ndef bootstrap_schema_tail_lazy() -> LazyResult[object]:\n    return lambda produced_ref: produced_ref\n')
 monkeypatch.setattr(lint,'REPO_ROOT',tmp_path)
 result=lint._collect_file_violations(path)
 assert 'BEI024' in [row.code for row in result]


def test_signature_checks_ignore_lambda_keyword_only_parameters(tmp_path,monkeypatch):
 from scripts import lint_bootstrap_engine_invariants as lint
 path=tmp_path/'l2000_connection/schema_transaction.py';path.parent.mkdir(parents=True)
 path.write_text('@pure_lazy\ndef ensure_bootstrap_tables_lazy() -> LazyResult[object]:\n    return lambda *, produced_ref, dependency_refs: None\n')
 monkeypatch.setattr(lint,'REPO_ROOT',tmp_path)
 result=lint._collect_file_violations(path)
 assert sum(row.code=='BEI039' and 'keyword-only' in row.message for row in result)==2


def test_publication_signature_does_not_include_lambda_arguments(tmp_path,monkeypatch):
 from scripts import lint_bootstrap_engine_invariants as lint
 path=tmp_path/'bootstrap.py'
 path.write_text('def publish_bootstrap_metadata_lazy():\n    return lambda bootstrap_plan, model_classes: None\n')
 monkeypatch.setattr(lint,'REPO_ROOT',tmp_path)
 assert not any(row.code=='BEI017' for row in lint._collect_file_violations(path))


def test_import_discovery_includes_all_mcp_composition_cases(request):
 from data_layer_nine.discovery import upstream_tests
 suite=next(p for p in request.config.pluginmanager.get_plugins() if type(p).__name__=='Suite')
 selectors=upstream_tests(suite.root,('plan_composition_architecture',))
 names={s.split('::')[-1] for s in selectors if 'tests/l9875_mcp/test_mcp_architecture_contracts.py::' in s}
 assert names=={'test_mcp_semantic_plans_reject_runtime_imports','test_mcp_semantic_plans_reject_runtime_handle_fields','test_only_the_provider_executor_can_execute_mcp_plan_scopes','test_mcp_repository_plan_composition_probes_pass'}


def test_evidence_verifier_rejects_a_changed_complete_result(request):
 import copy,json
 import pytest
 from data_layer_nine.evidence import verify
 suite=next(p for p in request.config.pluginmanager.get_plugins() if type(p).__name__=='Suite')
 oracle=json.loads((suite.workspace/'fixtures/host/data-layer-port/batch-09-complete-api.json').read_text())
 report=copy.deepcopy(oracle)
 report.update(exitCode=0,mode='snapshot',semanticFallbacks=[],calls=[],instrumentationTranslations=[])
 assert verify(oracle,[report])['cases']==len(oracle['cases'])
 record=next(r for r in report['records'] if isinstance(r['expected'],list) and r['expected'])
 record['expected'][0]['message']='a deliberately incorrect diagnostic'
 with pytest.raises(ValueError,match='complete-result parity'):
  verify(oracle,[report])


@pytest.mark.parametrize('number,positive',[('0',True),('0.0',True),('False',True),('0j',True),('1',False),('1.0',False),('1j',False),('1e999',False),('1e999j',False)])
def test_executor_zero_counts_preserve_python_numeric_equality(tmp_path,number,positive):
 import ast
 from scripts import lint_bootstrap_engine_invariants as lint
 path=tmp_path/'l3200_metadata/bootstrap_state.py';path.parent.mkdir(parents=True)
 source=f'def execute_application_plan():\n    return BootstrapExecutionFacts(mutation_counts=BootstrapMutationCounts(a={number},b={number},c={number},d={number}))\n'
 path.write_text(source)
 result=lint._collect_fake_executor_violations(path,ast.parse(source).body[0])
 assert bool(result)==positive


@pytest.mark.parametrize('lifecycle',['absent','invalid','empty'])
def test_lifecycle_fallback_depends_on_file_presence(tmp_path,monkeypatch,lifecycle):
 from scripts import lint_connection_structural_boundaries as lint
 connection=tmp_path/'l2000_connection/connection.py';connection.parent.mkdir(parents=True)
 connection.write_text('class _ConnectionLifecycleMixin:\n    def __enter__(self):\n        self.connect()\n')
 path=tmp_path/'l2000_connection/connection_lifecycle.py'
 if lifecycle!='absent':path.write_text('def broken(\n' if lifecycle=='invalid' else '')
 for name,value in [('ROOT',tmp_path),('CONNECTION_FILE',connection),('CONNECTION_LIFECYCLE_FILE',path),('SQL_CONNECTION_FILE',tmp_path/'missing.py'),('BACKEND_CONNECTION_FILES',()),('LIFECYCLE_SCAN_FILES',())]:monkeypatch.setattr(lint,name,value)
 result=lint.lint()
 assert [row.code for row in result]==(['CSB030'] if lifecycle=='absent' else [])
