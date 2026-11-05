# SPDX-License-Identifier: Apache-2.0
"""Exercise rule boundaries beyond the mostly clean repository corpus."""
from pathlib import Path
import ast,pytest
from scripts import lint_generated_index_sources as generated
from scripts import lint_graphql_plan_ownership as graphql
from scripts import lint_hnsw_architecture as hnsw
from scripts import lint_embedding_vector_orchestration as embedding
from scripts import lint_matching_architecture as matching

@pytest.mark.parametrize('source',[
'def _generated_index(*, source: IndexSource):\n    return Index(source=source)\n',
'def _generated_index(*, source: "IndexSource"):\n    return Index(source=source)\n',
'async def _generated_index(*, source):\n    return Index(source=source)\n',
'from l1000_schema.schema_def import Index as Idx\nIdx()\n',
'import l1000_schema.schema_def as sd\nsd.Index()\n',
'def outer():\n    def _generated_index(*, source: IndexSource):\n        return Index(source=source)\n',
])
def test_generated_index_forms(tmp_path,source):
 path=tmp_path/'generic_sparse.py';path.write_text(source)
 result=generated.check_file(path)
 assert isinstance(result,list)

def test_generated_check_source_reads_the_file(tmp_path):
 path=tmp_path/'generic_sparse.py';path.write_text('Index()\n')
 assert generated.check_source(path,'pass\n')

@pytest.mark.parametrize('source',[
'def f(scope: PlanScope):\n    a=b\n    b=scope.child()\n    return a._secret\n',
'def f(scope: PlanScope):\n    def nested():\n        return scope._private\n    return scope.public\n',
'@reusable_template\nclass Template:\n    field: Transaction|None\n',
'def f(scope: PlanScope):\n    owner=scope.child()\n    return object.__getattribute__(owner,"private")\n',
'def f(scope: PlanScope):\n    return (\n',
])
def test_graphql_alias_and_parser_forms(source):
 result=graphql.scan_source(Path('l9750_graphql/probe.py'),source)
 assert isinstance(result,list)

@pytest.mark.parametrize('source',[
'"json route_partition"\njsonb_set(state)\n',
'def f():\n    "json"\n    return "json"\n',
'class A:\n    "JSONB"\n    pass\n',
'from grammar import SqlJsonBuild as build\n',
'json = (\n',
])
def test_hnsw_text_and_parser_forms(tmp_path,source):
 path=tmp_path/'sql_table_hnsw/state.py';path.parent.mkdir();path.write_text(source)
 assert isinstance(hnsw.find_hnsw_architecture_findings((path,)),tuple)

@pytest.mark.parametrize('source',[
'def f(conn):\n    return conn.session().read()\n',
'class A:\n    class B:\n        def f(self,conn):\n            return conn.write()\n',
'def f_lazy(txn):\n    def inner():\n        return txn.query().execute().resolve()\n',
])
def test_embedding_nested_receivers(source):
 assert isinstance(embedding.scan_source(Path('l5200_vector_index/embedding_worker.py'),source),list)

@pytest.mark.parametrize('source',[
'from pydantic import BaseModel,ConfigDict\nclass MatchSpec(BaseModel):\n    model_config = ConfigDict(frozen=True)\n    name: str|None\n',
'from dataclasses import dataclass\n@dataclass(frozen=True)\nclass MatchSpec:\n    x: object\n',
'from pydantic import BaseModel\nclass MatchSpec(BaseModel):\n    x: list[int]\n',
])
def test_matching_public_annotation_forms(source):
 assert isinstance(matching.scan_sources({Path('l1470_matching_contracts/model.py'):source}),tuple)
