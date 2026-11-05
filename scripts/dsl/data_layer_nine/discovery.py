# SPDX-License-Identifier: Apache-2.0
"""Discover upstream tests by checker imports and local helper dependencies."""
from ast import parse as parse_python,walk as walk_python
import ast,subprocess
from functools import lru_cache
from pathlib import Path


def imports(nodes,checks):
 result={}
 for node in nodes:
  if isinstance(node,ast.ImportFrom):
   for alias in node.names:
    module=(node.module+'.'+alias.name) if node.module=='scripts' else node.module or ''
    if module.startswith('scripts.lint_'):
     check=module.removeprefix('scripts.lint_')
     if check=='schema_architecture_plugin':check='schema_architecture'
     result[alias.asname or alias.name]=check if check in checks else None
  if isinstance(node,ast.Import):
   for alias in node.names:
    if alias.name.startswith('scripts.lint_'):
     check=alias.name.removeprefix('scripts.lint_')
     result[alias.asname or alias.name.split('.')[0]]=check if check in checks else None
 return result


@lru_cache(maxsize=4)
def upstream_tests(root,checks):
 root=Path(root)
 dedicated={Path('scripts/tests/test_lint_'+check+'.py') for check in checks if (root/('scripts/tests/test_lint_'+check+'.py')).exists()}
 selected=[str(root/path) for path in sorted(dedicated)]
 tracked=subprocess.check_output(['git','-C',str(root),'ls-files','-z','*.py']).decode().split('\0')
 for name in sorted(tracked):
  path=Path(name)
  if not name or path in dedicated or not (path.name.startswith('test_') or path.name.endswith('_test.py')):continue
  try:tree=parse_python((root/path).read_text())
  except SyntaxError:continue
  globals_=imports(tree.body,checks)
  functions={n.name:n for n in tree.body if isinstance(n,(ast.FunctionDef,ast.AsyncFunctionDef))}
  def relevant(function):
   pending=[function];seen=set()
   while pending:
    current=pending.pop()
    if current.name in seen:continue
    seen.add(current.name)
    nodes=list(walk_python(current));bindings={**globals_,**imports(nodes,checks)}
    names={n.id for n in nodes if isinstance(n,ast.Name)}|{arg.arg for arg in current.args.args}
    if any(bindings.get(name) in checks for name in names):return True
    pending.extend(functions[name] for name in names&functions.keys())
   return False
  for function in functions.values():
   if function.name.startswith('test_') and relevant(function):selected.append(str(root/path)+'::'+function.name)
 return tuple(selected)


def independent_collections(selectors):
 """Keep distinct owners of the same upstream MCP contract in separate runs."""
 groups=[];claims=[]
 for selector in selectors:
  filename,_,selected=selector.partition('::')
  tree=parse_python(Path(filename).read_text())
  owned=set()
  for node in tree.body:
   if not isinstance(node,(ast.FunctionDef,ast.AsyncFunctionDef)) or not node.name.startswith('test_') or selected and node.name!=selected:continue
   for decorator in node.decorator_list:
    if isinstance(decorator,ast.Call) and isinstance(decorator.func,ast.Attribute) and decorator.func.attr=='ner1435_case':
     owned.update(arg.value for arg in decorator.args if isinstance(arg,ast.Constant) and isinstance(arg.value,str))
  for index,existing in enumerate(claims):
   if not existing&owned:
    groups[index].append(selector);existing.update(owned);break
  else:groups.append([selector]);claims.append(owned)
 return groups
