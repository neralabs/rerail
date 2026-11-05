# SPDX-License-Identifier: Apache-2.0
"""Record string and path contracts by executing the unchanged original rules."""
import importlib,tempfile
from pathlib import Path

def record(source,seeds):
 from data_layer_nine.suite import normalize
 matching=importlib.import_module('scripts.lint_matching_architecture');cases=[]
 def observe(label,path,text,production=False):
  if production:
   with tempfile.TemporaryDirectory() as temporary:
    root=Path(temporary);target=root/path;target.parent.mkdir(parents=True);target.write_bytes(text.encode())
    expected=[dict(path=str(f.path),line=f.line,code=f.code,message=f.message) for f in matching.lint(root)]
  else:expected=normalize(matching.scan_sources({Path(path):text}),source)
  cases.append(dict(test=label,check='matching_architecture',entry='all' if production else 'scan_sources',parameters={},production=production,files=[dict(path=path,source=text)],expected=expected))
 def sql(label,prefix):
  observe('matching-sql-prefix-'+label,'data_layer/matching.py','from data_layer.matching import MatchSpec\nsql = f'+repr(prefix+' {value}')+'\n')
 for point in range(0x110000):
  if chr(point).isspace():sql('space-'+hex(point),chr(point)+'SELECT')
 for word in ['SELECT','INSERT','UPDATE','DELETE','CREATE','ALTER']:sql('lower-'+word,word.lower())
 for label,prefix in [('long-s','ſELECT'),('dotless-i','ıNSERT'),('terminal-expansion','SELECẗ'),('internal-expansion','SELEẗT'),('bom','\ufeffSELECT'),('literal-prefix','value SELECT')]:sql(label,prefix)
 for name in ['STAGES','STAGES_δ','STAGES_Δ','STAGES_ǰ','STAGES_ı']:
  observe('matching-uppercase-counter-'+name,'impl/common/l5250_matching/probe.py',f'count = MatchDatabaseRoundTripCount(len({name}))\n')
 for label,path in [('arabic-low','l١٧٠٠_other/helper.py'),('arabic-high','l٥٢٥٠_other/helper.py'),('first-unicode','l٧٠٠٠_owner/l1700_other/helper.py'),('fullwidth','l１７００_other/helper.py'),('supplementary','l𝟙𝟟𝟘𝟘_other/helper.py')]:
  observe('matching-layer-'+label,path,'class Helper:\n    def match(self):\n        pass\n')
 observe('matching-layer-default-arabic','impl/common/l١٧٠٠_other/helper.py','class Helper:\n    def match(self):\n        pass\n',True)
 bounded=importlib.import_module('scripts.lint_bounded_composed_graph_paths')
 clean=next(r for r in seeds['records'] if r['test']=='bounded_composed_graph_paths-clean')
 for point in range(0x1c,0x20):
  files=[dict(path=f['path'],source=seeds['sources'][f['sourceSha256']]) for f in clean['files']]
  target=next(f for f in files if f['path']==bounded.VERSIONED_LAZY_MIXIN.as_posix())
  target['source']+='\n'+chr(point)+'def '+sorted(bounded.BANNED_TRANSACTION_ENDPOINTS)[0]+'():\n    pass\n'
  with tempfile.TemporaryDirectory() as temporary:
   root=Path(temporary)
   for file in files:
    path=root/file['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(file['source'].encode())
   expected=[dict(path=(f.path.relative_to(root) if f.path.is_absolute() else f.path).as_posix(),line=f.line,code=f.code,message=f.message) for f in bounded.collect_violations(root)]
  assert len(expected)==1 and expected[0]['code']=='BCGP002'
  cases.append(dict(test='bounded-python-space-'+hex(point),check='bounded_composed_graph_paths',entry='all',parameters={},production=True,files=files,expected=expected))
 graphql=importlib.import_module('scripts.lint_graphql_plan_ownership')
 for label,annotation in [('closure',''),('shadow', 'scope'),('both','scope: PlanScope')]:
  path='l9750_graphql/probe.py';text='def outer(scope: PlanScope):\n    def inner('+annotation+'):\n        return scope._private\n    return inner\n'
  cases.append(dict(test='graphql-nested-'+label,check='graphql_plan_ownership',entry='scan_source',parameters={},production=False,files=[dict(path=path,source=text)],expected=normalize(graphql.scan_source(Path(path),text),source)))
 generated=importlib.import_module('scripts.lint_generated_index_sources')
 for label,text in [
  ('alias-before','from l1000_schema.schema_def import Index as Ix\ndef use():\n    return Ix()\n# _generated_index\n'),
  ('alias-after','def use():\n    return Ix()\nfrom l1000_schema.schema_def import Index as Ix\n# _generated_index\n'),
  ('lambda','def _generated_index(*,source: IndexSource):\n    return lambda: Index(source=source)\n'),
  ('async','async def _generated_index():\n    return Index(source=source)\n')]:
  path='impl/common/l7245_versioned_backends/generic_sparse.py'
  with tempfile.TemporaryDirectory() as temporary:
   root=Path(temporary);target=root/path;target.parent.mkdir(parents=True);target.write_bytes(text.encode())
   expected=[text.removeprefix(str(root)+'/') for text in generated.check_file(target)]
  cases.append(dict(test='generated-event-'+label,check='generated_index_sources',entry='check_file',parameters={},production=False,files=[dict(path=path,source=text)],expected=expected))
 for label,prefix in [('nul','# \x00\n\n'),('unicode','# é\n\n')]:
  files=[dict(path=f['path'],source=seeds['sources'][f['sourceSha256']]) for f in clean['files']]
  target=next(f for f in files if f['path']==bounded.VERSIONED_LAZY_MIXIN.as_posix())
  target['source']=prefix+target['source']+'\ndef '+sorted(bounded.BANNED_TRANSACTION_ENDPOINTS)[0]+'():\n    pass\n'
  with tempfile.TemporaryDirectory() as temporary:
   root=Path(temporary)
   for file in files:
    path=root/file['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(file['source'].encode())
   expected=[dict(path=(f.path.relative_to(root) if f.path.is_absolute() else f.path).as_posix(),line=f.line,code=f.code,message=f.message) for f in bounded.collect_violations(root)]
  cases.append(dict(test='bounded-text-prefix-'+label,check='bounded_composed_graph_paths',entry='all',parameters={},production=True,files=files,expected=expected))
 return cases
