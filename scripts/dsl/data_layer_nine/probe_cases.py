# SPDX-License-Identifier: Apache-2.0
"""Positive and clean controls for every batch-nine contract and API family."""
from pathlib import Path
from types import SimpleNamespace
import pytest


def write(root,path,source):
 p=root/path;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(source);return p

def backend(request):
 return next(p for p in request.config.pluginmanager.get_plugins() if type(p).__name__=='Suite')

@pytest.mark.parametrize('path,source,positive',[
 ('api/probe.py','import pkg.DatabaseConnection as SqlConnection\nfrom x import Thing as SessionHandle\nclass DatabaseConnection:\n pass\nx.SqlConnection\ny: _SqlConnectionCore\n',True),
 ('impl/probe.py','from x import _SqlConnectionCore\nclass SyncThingSessionHandle:\n async def go(self): pass\nclass AsyncThingVersionHandle:\n def go(self): pass\n',True),
 ('api/probe.py','class _VersionHandleCore:\n def read(self): pass\n async def write_async(self): pass\n',True),
 ('api/probe.py','class AsyncThingConnection:\n def branch_id(self): pass\n async def connect(self): pass\n def fetch_async(self): pass\n',False),
 ('impl/probe.py','from x import _SqlConnectionCore\n',False),
])
def test_mode_controls(tmp_path,path,source,positive):
 from scripts import lint_connection_mode_split as lint
 p=write(tmp_path,path,source);result=lint._violations_for_files(tmp_path,(p,));assert bool(result)==positive

@pytest.mark.parametrize('source,positive',[
 ('planning_callback: object\nself.planning_callback: object\nenqueue_post_ddl()\nplanning_callback()\ndef materialize_schema_operation_records():\n add_operation()\n def inner():\n  add_operation()\n flush_schema_plan_async()\n',True),
 ('# planning_callback\ndef ordinary():\n add_operation()\nother_callback: object\n',False),
])
def test_schema_architecture_controls(tmp_path,source,positive):
 from scripts import lint_schema_architecture as lint
 rows=lint.lint_file(write(tmp_path,'schema.py',source));assert bool(rows)==positive
 if positive:assert {r.rule_id for r in rows}=={'forbid-schema-planning-callback-field','forbid-schema-callback-queues','forbid-materialization-plan-mutation'}

@pytest.mark.parametrize('check',['bootstrap_lazy_sql','bootstrap_eager_resolve'])
@pytest.mark.parametrize('source,positive',[
 ("for r in sorted(tuple(list(rows))):\n c.query_lazy().execute_lazy()\nmodel.model_dump()\nc.table_exists_many_async()\n",True),
 ('for row in sorted(build(rows)):\n model_dump()\nquery_lazy().execute_lazy()\n',False),
])
def test_lazy_sql_and_alias_controls(tmp_path,request,check,source,positive):
 suite=backend(request);module=suite.modules['bootstrap_lazy_sql'];p=tmp_path/'bootstrap.py'
 original=next(original for owner,name,original in suite.originals if owner is module and name=='scan_source')
 rows=suite.wrap(check,'scan_source',module,original)(p,source);assert bool(rows)==positive
 if positive:assert {r.code for r in rows}=={'BLB-001','BLB-002','BLB-003','BLB-004'}

@pytest.mark.parametrize('source,positive',[
 ('@foo.pytest.fixture()\ndef widgets():\n bootstrap_test_schema_for_tests(c, manifest_key="x")\n create_tables(c)\n def helper():\n  create_tables(c)\n',True),
 ('def test_x():\n # bootstrap-cache-health: allow\n bootstrap_test_schema_for_tests(c, manifest_key="x")\n c.create_tables()\n def helper():\n  create_tables(c)\n',False),
])
def test_cache_controls(source,positive):
 from scripts import lint_bootstrap_cache_health as lint
 result=lint.scan_file(Path('conform/test_probe.py'),source);assert bool(result)==positive
 if positive:assert {r.code for r in result}=={'BCH001','BCH002','BCH003'}

@pytest.mark.parametrize('source,positive',[
 ('from other.executor import X\nfield: "Transaction"\nclass SemanticPlanBuilder:\n def add_fragment(self)->list[int]:\n  thing.commit()\n',True),
 ('from types import X\nclass SemanticPlanBuilder:\n def add_fragment(self)->tuple[int]:\n  RefRemap\n  rewrite_exact_plan_refs_payload\n',False),
])
def test_composition_source_controls(source,positive):
 from scripts import lint_plan_composition_architecture as lint
 rows=lint.scan_semantic_source(Path('semantic_plan.py'),source);assert bool(rows)==positive
 if positive:assert {r.rule for r in rows}=={'SPC-001','SPC-002','SPC-003','SPC-004','SPC-005'}

def test_live_composition_positive(monkeypatch):
 from l2070_batching import semantic_plan
 from scripts import lint_plan_composition_architecture as lint
 class Builder:
  ordinal=0
  def __init__(self,**kwargs):pass
  def allocate_ref(self,**kwargs):return 'source-ref'
  def add_step(self,*args):pass
  def add_return(self,*args):pass
  def freeze(self):
   type(self).ordinal+=1
   return SimpleNamespace(fingerprint=str(type(self).ordinal),as_fragment=lambda:'fragment')
  def add_fragment(self,*args):return SimpleNamespace(ref_map={'source':'colliding-destination'})
 monkeypatch.setattr(semantic_plan,'SemanticPlanBuilder',Builder)
 assert [r.rule for r in lint.run_behavior_probes()]==['SPC-012','SPC-005']

@pytest.mark.parametrize('source,positive',[
 ('def bad_lazy(a: "SyncSchemaTransaction", /, b, *args: Any, c: Mapping[str,int], **kw: AsyncSchemaTransaction) -> "SyncReadTransaction":\n x: "AsyncWriteTransaction"\n def nested(v: Any):\n  x: SyncWriteTransaction\n getattr(a,"x")\n isinstance(a,int)\n',True),
 ('@decorators.pure_lazy()\ndef good_lazy(self, rows: Sequence[Widget], /, *args: tuple[Widget], **kw: dict[str,Widget])->LazyResult[Widget]:\n return value\n',False),
 ('def _private(rows: Sequence[Widget]):\n pass\n',True),
 ('def forward(rows: "Sequence[Widget]"):\n pass\n',True),
])
def test_contract_annotation_controls(tmp_path,source,positive):
 from scripts import lint_schema_bootstrap_contracts as lint
 result=lint.lint_file(write(tmp_path,'contracts.py',source));assert bool(result)==positive

@pytest.mark.parametrize('source',[
 'class _DatabaseConnectionCore(Bad, T[Ignored], pkg.Other):\n pass\n',
 'class _DatabaseConnectionCore(Good):\n pass\nclass _DatabaseConnectionCore(_LazyConnectionBase):\n pass\n',
])
def test_structural_duplicate_and_base_controls(tmp_path,monkeypatch,source):
 from scripts import lint_connection_structural_boundaries as lint
 monkeypatch.setattr(lint,'ROOT',tmp_path)
 monkeypatch.setattr(lint,'CONNECTION_FILE',write(tmp_path,'l2000_connection/connection.py',source))
 monkeypatch.setattr(lint,'CONNECTION_LIFECYCLE_FILE',tmp_path/'missing.py')
 monkeypatch.setattr(lint,'SQL_CONNECTION_FILE',tmp_path/'missing-sql.py')
 monkeypatch.setattr(lint,'BACKEND_CONNECTION_FILES',())
 monkeypatch.setattr(lint,'LIFECYCLE_SCAN_FILES',())
 rows=lint.lint();assert bool(rows)==('Bad' in source)

def test_structural_uncovered_limits_and_lifecycle(tmp_path,monkeypatch):
 from scripts import lint_connection_structural_boundaries as lint
 monkeypatch.setattr(lint,'ROOT',tmp_path)
 connection=write(tmp_path,'l2000_connection/connection.py','pass\n'*901)
 sql=write(tmp_path,'impl/common/l2200_sql_dialect/sql_connection.py','class _SqlConnectionCore(A,B,C,D,E,F,G):\n pass\n'+'def f(): pass\n'*91+'# pad\n'*6001)
 pg=write(tmp_path,'impl/postgres/connection.py','class SyncPgConnection:\n async def connect_async(self):\n  self._conn,self._cursor=x\n  self._validator+=1\n def work(self):\n  def inner():\n   self.connect()\n'+'#pad\n'*3001)
 for name,value in [('CONNECTION_FILE',connection),('SQL_CONNECTION_FILE',sql),('CONNECTION_LIFECYCLE_FILE',tmp_path/'missing.py'),('BACKEND_CONNECTION_FILES',(pg,)),('LIFECYCLE_SCAN_FILES',(pg,))]:monkeypatch.setattr(lint,name,value)
 assert {r.code for r in lint.lint()}=={'CSB001','CSB010','CSB011','CSB013','CSB020','CSB032','CSB033'}

def service_tree(tmp_path,monkeypatch):
 from scripts import lint_connection_service_architecture as lint
 monkeypatch.setattr(lint,'ROOT',tmp_path)
 sql=write(tmp_path,'impl/common/l2200_sql_dialect/sql_connection.py','class _SqlConnectionCore:\n def create_table_from_schema(self): pass\nclass SqlSessionContextService:\n pass\n')
 schema=write(tmp_path,'l2000_connection/schema_transaction.py','class LazySchemaTransaction: pass\nclass SyncSchemaTransaction: pass\nclass AsyncSchemaTransaction: pass\n')
 paths={}
 for backend,owner in [('sqlite','_SqliteConnectionCore'),('postgres','_PostgresConnectionCore'),('mariadb','_MariaDBConnectionCore')]:
  path=write(tmp_path,'impl/'+backend+'/connection.py','class '+owner+':\n def direct(self): pass\n');paths[path]=owner
 for name,value in [('SQL_CONNECTION',sql),('SCHEMA_TRANSACTION',schema),('BACKEND_CORE_FILES',paths),('CONNECTION_TARGET_DIRS',(tmp_path/'l2000_connection',tmp_path/'impl/common/l2200_sql_dialect')),('CONNECTION_TARGET_FILES',tuple(paths)),('FORBIDDEN_CONNECTION_METHOD_FILES',())]:monkeypatch.setattr(lint,name,value)
 return lint

def test_service_repository_missing_owners(tmp_path,monkeypatch):
 lint=service_tree(tmp_path,monkeypatch)
 errors=lint.lint();assert any('missing service-owned methods' in r.detail for r in errors);assert any('missing backend service assembly' in r.detail for r in errors)

def test_service_cross_file_owners(tmp_path,monkeypatch):
 from scripts import lint_connection_service_architecture as lint
 monkeypatch.setattr(lint,'ROOT',tmp_path)
 source='class OWNER:\n def table_exists(self):\n  value=1\n  return value\n def trigger_exists(self):\n  value=1\n  return value\n'
 paths=[write(tmp_path,'a.py',source.replace('OWNER','A')),write(tmp_path,'b.py',source.replace('OWNER','BackendService'))]
 result=lint._lint_duplicate_service_ownership_across_files(paths);assert len(result)==2

@pytest.mark.parametrize('source,positive',[
 ('class SyncSchemaTransaction:\n @legacy_async(lazy_name="fetch_lazy")\n async def fetch_async(self):\n  await _legacy_delegate_stub_async()\nclass LazySchemaTransaction:\n @pure_lazy\n def fetch_lazy(self): pass\n',True),
 ('class Thing:\n @legacy_sync\n def fetch(self):\n  _legacy_delegate_stub()\n @pure_lazy\n def fetch_lazy(self): pass\n',False),
 ('class Thing:\n @module.legacy_sync\n def fetch(self):\n  raise AssertionError()\n',False),
])
def test_service_legacy_exact_controls(source,positive):
 from scripts import lint_connection_service_architecture as lint
 assert bool(lint.analyze_source(Path('example.py'),source))==positive

@pytest.mark.parametrize('path,source,positive',[
 ('bootstrap_interrogation.py','def _proof_hashes_from_payload(): return None\nvalue=request or proof_hashes\n',True),
 ('schema_transaction.py','@pure_lazy\ndef ensure_bootstrap_widgets_lazy(*,produced_ref,dependency_refs)->LazyResult[None]:\n del produced_ref,dependency_refs\n ensure_tables()\n',True),
 ('bootstrap_coordinator.py','def _transition_replay_plan_for_observation():\n artifact_hash\n backend_facts\n desired_hash\n observed_hash\n backend_fingerprint\n',False),
 ('bootstrap.py','value=field(default_factory=lambda: None)\n',False),
 ('other.py','x.labels\n',False),
 ('impl/common/l2200_sql_dialect/other.py','reconciliation=True\ndeferred_state.pending_unlisted\nc.introspect_table_lazy()\n',True),
])
def test_engine_remaining_controls(tmp_path,monkeypatch,path,source,positive):
 from scripts import lint_bootstrap_engine_invariants as lint
 monkeypatch.setattr(lint,'REPO_ROOT',tmp_path)
 p=write(tmp_path,path,source);monkeypatch.setattr(lint,'BOOTSTRAP_FILES',(p,));monkeypatch.setattr(lint,'BOOTSTRAP_ENGINE_FILES',frozenset({p}))
 assert bool(lint._collect_file_violations(p))==positive


def test_mode_source_gate_uses_supplied_probe(tmp_path):
 from scripts import lint_connection_mode_split as lint
 path=write(tmp_path,'api/probe.py','class DatabaseConnection: pass\n')
 assert lint.lint_source(path,'pass\n',root=tmp_path)==[]


def test_cache_ignores_supplied_ast():
 import ast
 from scripts import lint_bootstrap_cache_health as lint
 rows=lint.scan_file(Path('conform/test_probe.py'),'def test_x():\n create_table()\n',ast.parse('pass\n'))
 assert [row.code for row in rows]==['BCH003']


def test_engine_tree_and_raw_source_are_independent(tmp_path,monkeypatch):
 import ast
 from scripts import lint_bootstrap_engine_invariants as lint
 path=write(tmp_path,'bootstrap.py','x=1\n');monkeypatch.setattr(lint,'BOOTSTRAP_FILES',(path,))
 rows=lint._collect_file_violations(path,source='# SchemaTransactionSidecar\n',tree=ast.parse('x=1\n'))
 assert [row.code for row in rows]==['BEI035']


def test_engine_private_helper_uses_selected_function():
 import ast
 from scripts import lint_bootstrap_engine_invariants as lint
 source='def execute_application_plan():\n del conn\ndef execute_application_plan():\n del conn\n'
 tree=ast.parse(source)
 assert len(lint._collect_fake_executor_violations(Path('bootstrap.py'),tree.body[0]))==1


@pytest.mark.parametrize('decorators',['@legacy_sync\n @legacy_sync','@legacy_sync(lazy_name="")'])
def test_service_legacy_decorator_controls(decorators):
 from scripts import lint_connection_service_architecture as lint
 source='class Example:\n '+decorators+'\n def fetch(self):\n  _legacy_delegate_stub()\n @pure_lazy\n def fetch_lazy(self):\n  pass\n'
 assert lint.analyze_source(Path('example.py'),source)==[]


@pytest.mark.parametrize('check',['schema_bootstrap_contracts','bootstrap_engine_invariants','bootstrap_lazy_sql','plan_composition_architecture'])
def test_parser_errors_keep_the_original_api(tmp_path,request,check):
 suite=backend(request);module=suite.modules[check];source='def bootstrap_bad(\n';path=write(tmp_path,'bootstrap.py',source)
 with pytest.raises(SyntaxError):
  if check=='schema_bootstrap_contracts':module.lint_file(path)
  elif check=='bootstrap_engine_invariants':module._collect_file_violations(path)
  elif check=='bootstrap_lazy_sql':module.scan_source(path,source)
  else:module.scan_semantic_source(path,source)


def test_cache_parser_error_is_a_diagnostic():
 from scripts import lint_bootstrap_cache_health as lint
 result=lint.scan_file(Path('conform/test_probe.py'),'def test_x( # create_tables\n')
 assert [r.code for r in result]==['BCH000']


def test_service_parser_error_is_ignored_by_source_analyzer():
 from scripts import lint_connection_service_architecture as lint
 assert lint.analyze_source(Path('source.py'),'def broken(\n')==[]


def test_schema_probe_is_correlated_to_its_method():
 from scripts import lint_connection_service_architecture as lint
 source='class ExampleConnection:\n def bad(self):\n  return self.column_exists()\n def clean(self):\n  return 1\n'
 rows=lint.analyze_source(Path('example.py'),source)
 assert [(r.code,r.line) for r in rows]==[('CONN103',2)]


def test_legacy_structural_helper_reads_supplied_ast(tmp_path,monkeypatch):
 import ast
 from scripts import lint_connection_structural_boundaries as lint
 path=write(tmp_path,'connection.py','not valid Python !!!\n');monkeypatch.setattr(lint,'CONNECTION_FILE',path)
 rows=lint._connection_file_violations({path:ast.parse('class WritePlanWorkflow: pass\n')})
 assert [r.code for r in rows]==['CSB002']


def test_conflicting_legacy_modes_preserve_the_original_error():
 from scripts import lint_connection_service_architecture as lint
 source='class Example:\n @legacy_sync\n @legacy_async\n def fetch(self): pass\n'
 with pytest.raises(ValueError,match='too many values to unpack'):
  lint.analyze_source(Path('example.py'),source)


def test_legacy_lifecycle_helper_preserves_first_definition_and_method_order(tmp_path,monkeypatch):
 import ast
 from scripts import lint_connection_structural_boundaries as lint
 source='class _ConnectionLifecycleMixin:\n def __aenter__(self): self.connect()\n def __enter__(self): self.connect()\n def __enter__(self): self.connect()\nclass _ConnectionLifecycleMixin:\n def __enter__(self): self.connect()\n'
 path=write(tmp_path,'connection.py',source)
 monkeypatch.setattr(lint,'ROOT',tmp_path);monkeypatch.setattr(lint,'CONNECTION_FILE',path);monkeypatch.setattr(lint,'CONNECTION_LIFECYCLE_FILE',tmp_path/'missing.py');monkeypatch.setattr(lint,'LIFECYCLE_SCAN_FILES',())
 rows=lint._lifecycle_violations({path:ast.parse(source)})
 assert [(r.code,r.line) for r in rows]==[('CSB030',3),('CSB030',2)]


def test_executor_preserves_repeated_delete_targets_and_keywords(tmp_path,monkeypatch):
 from scripts import lint_bootstrap_engine_invariants as lint
 source='def execute_application_plan():\n del conn, conn\n return BootstrapExecutionFacts(mutation_counts=BootstrapMutationCounts(a=0,b=0,c=0,d=0),mutation_counts=BootstrapMutationCounts(a=0,b=0,c=0,d=0))\n'
 path=write(tmp_path,'bootstrap.py',source);monkeypatch.setattr(lint,'REPO_ROOT',tmp_path);monkeypatch.setattr(lint,'BOOTSTRAP_FILES',(path,));monkeypatch.setattr(lint,'BOOTSTRAP_ENGINE_FILES',frozenset({path}))
 rows=lint._collect_file_violations(path)
 assert [(r.code,r.line) for r in rows]==[('BEI007',2),('BEI007',2),('BEI007',3),('BEI007',3)]


def test_publication_preserves_repeated_empty_dependency_keywords(tmp_path,monkeypatch):
 from scripts import lint_bootstrap_engine_invariants as lint
 source='def _enqueue_application_schema_lazy():\n publish_bootstrap_metadata_lazy(dependency_refs=(),dependency_refs=())\n bootstrap_schema_tail_lazy()\n'
 path=write(tmp_path,'l3200_metadata/bootstrap_coordinator.py',source);monkeypatch.setattr(lint,'REPO_ROOT',tmp_path);monkeypatch.setattr(lint,'BOOTSTRAP_FILES',(path,));monkeypatch.setattr(lint,'BOOTSTRAP_ENGINE_FILES',frozenset({path}))
 rows=lint._collect_file_violations(path)
 assert [(r.code,r.line) for r in rows if r.code=='BEI016']==[('BEI016',2),('BEI016',2)]
