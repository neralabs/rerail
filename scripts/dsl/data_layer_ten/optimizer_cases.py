# SPDX-License-Identifier: Apache-2.0
"""Original-evaluated positives and clean controls where upstream has no tests."""
from pathlib import Path
import pytest
from scripts import lint_plan_optimizer_v2 as lint
CASES=[
('PV2-001','node.py','import socket\n'),
('PV2-002','node.py','import impl.common.l2066_plan_optimizer\n'),
('PV2-003','../l2066_plan_optimizer/node.py','import l2070_batching\n'),
('PV2-010','node.py','x = scope._bindings\n'),
('PV2-011','node.py',"x = value.split('.')\n"),
('PV2-012','node.py','binding_by_table = {}\n'),
('PV2-012','node.py','def f(x_by_table):\n    pass\n'),
('PV2-020','node.py',"if x == 'count': pass\n"),
('PV2-021','node.py',"x = 'foo_count'\n"),
('PV2-021','node.py',"x = f'{name}_count'\n"),
('PV2-022','node.py','x = set(a) & b\n'),
('PV2-022','node.py','x = a.intersection(b)\n'),
('PV2-023','node.py',"x = ColumnSchema('key')\n"),
('PV2-030','node.py','x = M.model_fields\n'),
('PV2-031','node.py',"def broken():\n    raise ValueError('broken')\n"),
('PV2-060','node.py','def placement_after():\n    pass\n'),
('PV2-060','node.py','x = effects.estimated_param_count\n'),
('PV2-060','node.py','effects.can_share_statement_after(other)\n'),
('PV2-040','passes.py','class BrokenPass:\n    pass\n'),
('PV2-041','passes.py','class PhysicalPass:\n    phase: object = OptimizerPhase.PHYSICAL\n    name: str = "physical"\n    def run(self): pass\nclass NormalizePass:\n    phase: object = OptimizerPhase.NORMALIZE\n    name: str = "normalize"\n    def run(self): pass\nDEFAULT_V2_PASSES = (PhysicalPass(), NormalizePass())\n'),
('PV2-042','node.py','def mutate(x):\n    x.value = 1\n'),
('PV2-043','passes.py','def run_verified_passes():\n    pass\n'),
('PV2-044','node.py','\n'*601),
]
@pytest.mark.parametrize('code,name,source',CASES)
def test_original_optimizer_positive(code,name,source):
 path=Path('impl/common/l2066_plan_optimizer')/name.removeprefix('../l2066_plan_optimizer/') if name.startswith('../') else Path('impl/common/l2065_plan_core')/name
 findings=lint.scan_file(path,source)
 assert any(f.code==code for f in findings)
@pytest.mark.parametrize('name,source',[('scope.py',"x=scope._bindings\nx=value.split('.')\n"),('effects.py','x = effects.estimated_param_count\neffects.can_share_statement_after(other)\n'),('node.py','def __init__(self):\n    self.value=1\n'),('node.py',"def bad():\n    raise ValueError('Fix the producer')\n"),('node.py','value=ColumnSchema(ColumnId(name))\n'),('passes.py','class Pass(Protocol):\n    pass\n')])
def test_original_optimizer_clean(name,source):
 assert lint.scan_file(Path('impl/common/l2065_plan_core')/name,source)==[]
def test_original_optimizer_missing_document(tmp_path):
 assert any(f.code=='PV2-050' for f in lint.find_violations(tmp_path))
def test_original_optimizer_registry_and_document(tmp_path):
 path=tmp_path/lint.VERIFIER_RELPATH;path.parent.mkdir(parents=True);path.write_text("OptimizerInvariant(code='scope.new')\n")
 doc=tmp_path/lint.INVARIANT_DOC_RELPATH;doc.parent.mkdir(parents=True);doc.write_text('`schema.stale`\n')
 result=lint.find_violations(tmp_path)
 assert {r.code for r in result}=={'PV2-050','PV2-051','PV2-052'}
def test_original_optimizer_registry_clean(tmp_path):
 path=tmp_path/lint.VERIFIER_RELPATH;path.parent.mkdir(parents=True);path.write_text("OptimizerInvariant(code='scope.new')\n# (scope.new)\n")
 doc=tmp_path/lint.INVARIANT_DOC_RELPATH;doc.parent.mkdir(parents=True);doc.write_text('`scope.new`\n')
 assert lint.find_violations(tmp_path)==[]
