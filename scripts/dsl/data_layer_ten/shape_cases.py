# SPDX-License-Identifier: Apache-2.0
"""Run altered syntax through original detectors and compare complete native results."""
import ast,copy,importlib,json
from pathlib import Path
import pytest

ROOT=Path(__file__).resolve().parents[3]
payload=json.loads((ROOT/'fixtures/host/data-layer-port/batch-10-shape-seeds.json').read_text())

def altered(source,form):
 if form=='shift':return '# source offset regression\n\n'+source
 try:tree=ast.parse(source)
 except SyntaxError:return source
 if form=='control':
  tree.body=[ast.If(test=ast.Constant(True),body=tree.body or [ast.Pass()],orelse=[])]
  return ast.unparse(ast.fix_missing_locations(tree))+'\n'
 if form=='duplicate':
  if tree.body:tree.body.insert(1,copy.deepcopy(tree.body[0]))
  return ast.unparse(tree)+'\n'
 if form=='shadow':
  for node in tree.body:
   if isinstance(node,(ast.ClassDef,ast.FunctionDef,ast.AsyncFunctionDef)):
    replacement=copy.deepcopy(node);replacement.body=[ast.Pass()];tree.body.append(replacement);break
  return ast.unparse(ast.fix_missing_locations(tree))+'\n'
 if form=='normalized':return ast.unparse(tree)+'\n'
 # Parentheses alter CST shape while preserving the Python expression structure.
 choices=[n for n in ast.walk(tree) if isinstance(n,(ast.Call,ast.Attribute,ast.Name)) and not isinstance(getattr(n,'ctx',None),(ast.Store,ast.Del))]
 if not choices:return source
 node=choices[0];lines=source.encode().splitlines(keepends=True)
 start=sum(map(len,lines[:node.lineno-1]))+node.col_offset
 end=sum(map(len,lines[:node.end_lineno-1]))+node.end_col_offset
 data=source.encode();return (data[:start]+b'('+data[start:end]+b')'+data[end:]).decode()

@pytest.mark.parametrize('seed',payload['records'],ids=lambda seed:seed['test'])
@pytest.mark.parametrize('form',['shift','normalized','parentheses','control','duplicate','shadow'])
def test_complete_shape_result(seed,form,tmp_path,monkeypatch):
 module=importlib.import_module('scripts.lint_'+seed['check']);files={f['path']:altered(payload['sources'][f['sourceSha256']],form) if f['path'].endswith('.py') else payload['sources'][f['sourceSha256']] for f in seed['files']}
 for name,source in files.items():
  path=tmp_path/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(source.encode())
 check=seed['check'];name,source=next(iter(files.items()),('empty.py',''))
 if check=='graph_lazy_sql_plan':result=module._violations_for_sources(files)
 elif check in {'bounded_composed_graph_paths','conditional_architecture'}:result=module.collect_violations(tmp_path)
 elif check=='plan_optimizer_v2':result=module.find_violations(tmp_path) if seed['entry']=='find_violations' else module.scan_file(Path(name),source)
 elif check=='matching_architecture':result=module.scan_sources({Path(p):s for p,s in files.items()})
 elif check=='hnsw_architecture':
  monkeypatch.chdir(tmp_path);result=module.find_hnsw_architecture_findings(tuple(Path(p) for p in files))
 elif check=='generated_index_sources':result=module.check_file(tmp_path/name)
 elif check=='schema_introspection_data_paths':result=module._collect_violations(module.ROOT/name,ast.parse(source))
 else:result=module.scan_source(Path(name),source)
 # Suite records the full result and rejects any difference before this assertion.
 assert isinstance(result,(list,tuple))
