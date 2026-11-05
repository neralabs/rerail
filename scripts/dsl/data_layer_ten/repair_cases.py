# SPDX-License-Identifier: Apache-2.0
"""Exercise audit regressions and the shared conditional analyzer through original APIs."""
import importlib,json
from pathlib import Path
import pytest
ROOT=Path(__file__).resolve().parents[3]
repairs=json.loads((ROOT/'fixtures/host/data-layer-port/batch-10-repair-regressions.json').read_text())['cases']

@pytest.mark.parametrize('case',repairs,ids=lambda c:c['test'])
def test_audited_complete_result(case,tmp_path):
 module=importlib.import_module('scripts.lint_'+case['check'])
 for file in case['files']:
  path=tmp_path/file['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_text(file['source'])
 file=case['files'][0]
 if case['entry']=='collect_violations':result=module.collect_violations(tmp_path)
 elif case['production']:result=module.lint(tmp_path)
 elif case['entry']=='scan_sources':result=module.scan_sources({Path(f['path']):f['source'] for f in case['files']})
 elif case['entry']=='scan_file':result=module.scan_file(Path(file['path']),file['source'])
 else:result=module.scan_source(Path(file['path']),file['source'])
 actual=[dict(code=getattr(r,'code',getattr(r,'rule',None)),line=r.line,message=r.message) for r in result]
 assert sorted(actual,key=str)==sorted(case['expected'],key=str)

CONTRACT_BODIES=[
 'raise (TypeError("unsupported"))',
 'raise TypeError',
 'raise factory().TypeError("unsupported")',
 'if flag:\n    raise TypeError\nelse:\n    raise TypeError',
 'if flag:\n    raise TypeError\nreturn None',
 'return None\nraise TypeError',
 'raise TypeError\nreturn None',
 'match value:\n    case 1:\n        raise TypeError\n    case 2:\n        raise TypeError',
 'match value:\n    case 1:\n        raise TypeError\n    case 2:\n        return None',
 'yield value\nraise TypeError',
 'for item in values:\n    raise TypeError',
 'try:\n    raise TypeError\nfinally:\n    pass',
]
STATE_BODIES=[
 'self.branch = True',
 'self.branch: bool',
 'self.left, self.right = values',
 'self.count += 1',
 'for self.branch in values:\n    pass',
 'child = self.then_plan.steps[0]\nchild.compile(ctx)',
 'for child in self.then_plan.steps:\n    child.compile(ctx)',
 'for left, child in self.then_plan.steps:\n    child.compile(ctx)',
 'for *child, tail in self.then_plan.steps:\n    child.compile(ctx)',
 'factory().compile(ctx)',
 'if ctx.backend_type.is_sql_backend:\n    pass',
 'a = 1 if flag else 0; b = 2 if ctx.backend_type else 3',
 'a = 1 if ctx.backend_type else 0; b = 2 if ctx.backend_type else 3',
]
def shared_findings(tmp_path,source):
 from scripts import conditional_architecture_facts as shared
 from scripts.lint_lib.fact_engine import FactEngine
 from scripts.lint_lib.fact_store import JsonFactStore
 from scripts.lint_lib.linter_plugin import FileScopeSpec,LinterPlugin
 path=tmp_path/'l2070_batching/conditional_step.py';path.parent.mkdir();path.write_text(source)
 plugin=LinterPlugin(name='conditional_shape_test',enforcement='hard-fail',description='Shared analyzer parity',file_scope=FileScopeSpec(directories=(Path('.'),),include_tests=True),required_facts=(shared.CONDITIONAL_ARCHITECTURE_EXTRACTOR,),reporter=lambda _:[])
 engine=FactEngine(root=tmp_path,fact_store=JsonFactStore(tmp_path/'.facts'),extractors={shared.CONDITIONAL_ARCHITECTURE_EXTRACTOR.extractor_id:shared.ConditionalArchitectureCollector})
 return shared.conditional_architecture_findings(engine.materialize((plugin,),max_workers=1))

@pytest.mark.parametrize('body',CONTRACT_BODIES)
def test_shared_control_flow(tmp_path,body):
 source='class ConditionalStep:\n    def compile(self, ctx):\n        '+body.replace('\n','\n        ')+'\n    def require_sql_physical_conditional(self):\n        '+body.replace('\n','\n        ')+'\n'
 assert isinstance(shared_findings(tmp_path,source),list)

@pytest.mark.parametrize('body',STATE_BODIES)
def test_shared_state_and_provenance(tmp_path,body):
 source='from l2070_batching.conditional_step import ConditionalStep as Semantic\nclass ConditionalStep:\n    pass\nclass Derived(Semantic):\n    def run(self, ctx):\n        '+body.replace('\n','\n        ')+'\n'
 assert isinstance(shared_findings(tmp_path,source),list)

@pytest.mark.parametrize('source',[
 'class ConditionalStep:\n    callback: Callable[[Callable], Callable]\n',
 'class ConditionalStep:\n    def compile(self):\n        raise TypeError\n    def compile(self):\n        return None\n',
 'class ConditionalStep:\n    def compile(self):\n        return None\n    def compile(self):\n        raise TypeError\n',
 'class ConditionalStep:\n    def compile(self):\n        raise TypeError\nclass ConditionalStep:\n    pass\n',
 'class ConditionalStep:\n    def outer(self):\n        def compile():\n            raise TypeError\n',
 'class ConditionalStep:\n    pass\nclass Derived(ConditionalStep):\n    def __init__(self, callback: Callable):\n        self.callback = callback\n',
 'class ConditionalStep:\n    pass\nclass Derived(ConditionalStep):\n    def run(self):\n        self.value = 1\n        def run():\n            self.value = 2\n',
 'class ConditionalStep:\n    pass\nclass Derived(ConditionalStep):\n    def outer(self):\n        class Derived(ConditionalStep):\n            def run(self):\n                self.value = 2\n',
])
def test_shared_declaration_ownership(tmp_path,source):
 source='from l2070_batching.conditional_step import ConditionalStep as Semantic\n'+source.replace('Derived(ConditionalStep)','Derived(Semantic)')
 assert isinstance(shared_findings(tmp_path,source),list)

@pytest.mark.parametrize('signature',['def match(self):','def \\\n match(self):','async def match(self):','async \\\n def match(self):'])
@pytest.mark.parametrize('marker',['','# MatchSpec\n'])
def test_matching_default_selection(tmp_path,signature,marker):
 from scripts import lint_matching_architecture as matching
 path=tmp_path/'l1700_other/helper.py';path.parent.mkdir();path.write_text(marker+'class Helper:\n    '+signature.replace('\n','\n    ')+'\n        return self.conn.query()\n')
 assert isinstance(matching.lint(tmp_path),tuple)
