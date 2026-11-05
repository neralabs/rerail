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
