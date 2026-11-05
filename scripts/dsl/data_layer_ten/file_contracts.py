# SPDX-License-Identifier: Apache-2.0
"""Record original contracts without changing raw file contents."""
import argparse,importlib,json,sys,tempfile
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from data_layer_ten.common import PIN

def record(source,output):
 sys.path[:0]=[str(source),str(source/'scripts')]
 from data_layer_nine.suite import normalize
 root=Path(__file__).resolve().parents[3]
 seeds=json.loads((root/'fixtures/host/data-layer-port/batch-10-shape-seeds.json').read_text())
 bounded=importlib.import_module('scripts.lint_bounded_composed_graph_paths');cases=[]
 clean=next(r for r in seeds['records'] if r['test']=='bounded_composed_graph_paths-clean')
 for positive in [False,True]:
  for ending,label in [('\n','lf'),('\r\n','crlf'),('\r','cr')]:
   files=[dict(path=f['path'],source=seeds['sources'][f['sourceSha256']]) for f in clean['files']]
   if positive:
    target=next(f for f in files if f['path']=='l7200_versioned_transactions/versioned_lazy_mixin.py')
    target['source']+='\n\ndef '+sorted(bounded.BANNED_TRANSACTION_ENDPOINTS)[0]+'():\n    pass\n'
   for f in files:f['source']=f['source'].replace('\n',ending)
   with tempfile.TemporaryDirectory() as temporary:
    path=Path(temporary)
    for f in files:
     location=path/f['path'];location.parent.mkdir(parents=True,exist_ok=True);location.write_bytes(f['source'].encode())
    expected=[dict(path=(r.path.relative_to(path) if r.path.is_absolute() else r.path).as_posix(),line=r.line,code=r.code,message=r.message) for r in bounded.collect_violations(path)]
   assert bool(expected)==positive
   cases.append(dict(test='bounded-'+('positive-' if positive else 'clean-')+label,check='bounded_composed_graph_paths',entry='all',parameters={},production=True,files=files,expected=expected))
 for check,path,entry in [('graph_lazy_sql_plan','l7200_versioned_transactions/versioned_lazy_mixin.py','_violations_for_sources'),('plan_optimizer_v2','impl/common/l2065_plan_core/node.py','scan_file')]:
  module=importlib.import_module('scripts.lint_'+check)
  for text in ['if True:\n','def incomplete():\n']:
   expected=module._violations_for_sources({path:text}) if check=='graph_lazy_sql_plan' else module.scan_file(Path(path),text)
   cases.append(dict(test=check+'-incomplete-'+text.split()[0],check=check,entry=entry,parameters={},production=False,files=[dict(path=path,source=text)],expected=normalize(expected,source)))
 module=importlib.import_module('scripts.lint_conditional_architecture')
 with tempfile.TemporaryDirectory() as temporary:
  path=Path(temporary);relative='l2070_batching/conditional_step.py'
  text='class ConditionalStep:\n    def compile(self, ctx):\n        pass\n        from impl.common.l2100_sql_grammar import SqlA\n'+'\n'*5+'        from impl.common.l2100_sql_grammar import SqlB\n'
  location=path/relative;location.parent.mkdir();location.write_bytes(text.encode())
  expected=normalize(module.collect_violations(path),path)
 cases.append(dict(test='conditional-textual-line-order',check='conditional_architecture',entry='collect_violations',parameters={},production=False,files=[dict(path=relative,source=text)],expected=expected))
 output.write_text(json.dumps(dict(sourcePin=PIN,cases=cases),indent=2)+'\n')

if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();record(a.source.resolve(),a.output)
