from pathlib import Path


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
