# SPDX-License-Identifier: Apache-2.0
"""Record original contracts without changing raw file contents."""
import argparse,ast,importlib,json,sys,tempfile
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
 from scripts.lint_lib.fact_engine import FactEngine,_run_plugin_reporter
 from scripts.lint_lib.fact_store import JsonFactStore
 optimizer=importlib.import_module('scripts.lint_plan_optimizer_v2')
 for ending,label in [('\n','lf'),('\r\n','crlf'),('\r','cr')]:
  text=ending*601;relative='impl/common/l2065_plan_core/node.py'
  cases.append(dict(test='optimizer-source-lines-'+label,check='plan_optimizer_v2',entry='scan_file',parameters={},production=False,files=[dict(path=relative,source=text)],expected=normalize(optimizer.scan_file(Path(relative),text),source)))
  with tempfile.TemporaryDirectory() as temporary:
   path=Path(temporary);location=path/relative;location.parent.mkdir(parents=True);location.write_bytes(text.encode())
   plugin=optimizer.plan_optimizer_v2_plugin(path)
   engine=FactEngine(root=path,fact_store=JsonFactStore(path/'.facts'),extractors=optimizer.plan_optimizer_v2_fact_extractors(path))
   findings=_run_plugin_reporter(plugin,engine.materialize((plugin,),max_workers=1)).violations
   expected=[dict(path=Path(f.path).relative_to(path).as_posix() if Path(f.path).is_absolute() else str(f.path),line=f.line,code=f.code,message=f.message) for f in findings]
  assert len(expected)==1 and expected[0]['code']=='PV2-044'
  cases.append(dict(test='optimizer-file-lines-'+label,check='plan_optimizer_v2',entry='all',parameters={},production=True,files=[dict(path=relative,source=text)],expected=expected))
 matching=importlib.import_module('scripts.lint_matching_architecture');relative='data_layer_testing/matching_operator.py'
 for hook in [False,True]:
  text='from data_layer.matching import MatchSpec\nimport importlib\nimportlib.import_module("plugin")\n'+('register_table_hook()\n' if hook else '')
  with tempfile.TemporaryDirectory() as temporary:
   path=Path(temporary);location=path/relative;location.parent.mkdir();location.write_bytes(text.encode())
   findings=matching.lint(path)
   expected=[dict(path=str(f.path),line=f.line,code=f.code,message=f.message) for f in findings]
  cases.append(dict(test='matching-auxiliary-hook-'+str(hook),check='matching_architecture',entry='all',parameters={},production=True,files=[dict(path=relative,source=text)],expected=expected))
  cases.append(dict(test='matching-explicit-auxiliary-'+str(hook),check='matching_architecture',entry='scan_sources',parameters={},production=False,files=[dict(path=relative,source=text)],expected=normalize(matching.scan_sources({Path(relative):text}),source)))
  cases.append(dict(test='matching-tree-auxiliary-'+str(hook),check='matching_architecture',entry='_scan_tree',parameters={},production=False,files=[dict(path=relative,source=text)],expected=normalize(matching._scan_tree(Path(relative),ast.parse(text)),source)))
 relative='impl/common/l5250_matching/probe.py';text='value=1\n'
 cases.append(dict(test='matching-tree-has-no-project-obligations',check='matching_architecture',entry='_scan_tree',parameters={},production=False,files=[dict(path=relative,source=text)],expected=normalize(matching._scan_tree(Path(relative),ast.parse(text)),source)))
 hnsw=importlib.import_module('scripts.lint_hnsw_architecture')
 for index,word in enumerate(['json\u0301','json\u200c','json\u203f','json_\u0301','jsoné','αjson','json\u1c89']):
  relative='impl/common/l2100_sql_grammar/domain/sql_table_hnsw/state.py';text='state = '+repr(word)+'\n'
  with tempfile.TemporaryDirectory() as temporary:
   path=Path(temporary);location=path/relative;location.parent.mkdir(parents=True);location.write_bytes(text.encode())
   expected=normalize(hnsw.find_hnsw_architecture_findings((location,)),path)
  cases.append(dict(test='hnsw-python-word-'+str(index),check='hnsw_architecture',entry='find_hnsw_architecture_findings',parameters={},production=False,files=[dict(path=relative,source=text)],expected=expected))
 for index,hint in enumerate(['Fix\u0301 producer','Fix\u200c producer','Fixé producer','Fix producer','Fix\u1c89 producer']):
  relative='impl/common/l2065_plan_core/node.py';text='def bad():\n    raise ValueError('+repr(hint)+')\n'
  cases.append(dict(test='optimizer-python-word-'+str(index),check='plan_optimizer_v2',entry='scan_file',parameters={},production=False,files=[dict(path=relative,source=text)],expected=normalize(optimizer.scan_file(Path(relative),text),source)))
 files=[dict(path=f['path'],source=seeds['sources'][f['sourceSha256']]) for f in clean['files']]
 target=next(f for f in files if f['path']==bounded.SQL_GRAPH_QUERY_BUILDER.as_posix());target['source']+='\n_SqlSparseUnionRecursiveBindingGrammar\u0301\n'
 with tempfile.TemporaryDirectory() as temporary:
  path=Path(temporary)
  for f in files:
   location=path/f['path'];location.parent.mkdir(parents=True,exist_ok=True);location.write_bytes(f['source'].encode())
  expected=[dict(path=(r.path.relative_to(path) if r.path.is_absolute() else r.path).as_posix(),line=r.line,code=r.code,message=r.message) for r in bounded.collect_violations(path)]
 assert len(expected)==1 and expected[0]['code']=='BCGP014'
 cases.append(dict(test='bounded-python-word-and-line',check='bounded_composed_graph_paths',entry='all',parameters={},production=True,files=files,expected=expected))
 for index,comment in enumerate(['# \u0301def match(self):\n','# def\x1cmatch(self):\n']):
  relative='l1700_other/helper.py';text=comment+'class Helper:\n    def \\\n        match(self):\n        return self.conn.query()\n'
  with tempfile.TemporaryDirectory() as temporary:
   path=Path(temporary);location=path/relative;location.parent.mkdir();location.write_bytes(text.encode())
   findings=matching.lint(path)
   expected=[dict(path=str(f.path),line=f.line,code=f.code,message=f.message) for f in findings]
  assert len(expected)==1 and expected[0]['code']=='MA003'
  cases.append(dict(test='matching-selector-python-whitespace-'+str(index),check='matching_architecture',entry='all',parameters={},production=True,files=[dict(path=relative,source=text)],expected=expected))
 from data_layer_ten.contracts.strings import record as semantic_contracts
 cases.extend(semantic_contracts(source,seeds))
 from data_layer_ten.contracts.structures import record as structural_contracts
 cases.extend(structural_contracts(source))
 from data_layer_ten.contracts.collectors import record as collector_contracts
 cases.extend(collector_contracts(source))
 output.write_text(json.dumps(dict(sourcePin=PIN,cases=cases),indent=2)+'\n')

if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();record(a.source.resolve(),a.output)
