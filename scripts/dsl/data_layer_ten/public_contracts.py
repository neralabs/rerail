# SPDX-License-Identifier: Apache-2.0
"""Observe configured upstream reporters and file discovery without replay adapters."""
from pathlib import Path
import argparse,hashlib,importlib,json,os,sys,tempfile
DSL=Path(__file__).resolve().parents[1];sys.path.insert(0,str(DSL))
from data_layer_ten.common import CHECKS,PIN
from data_layer_nine.evidence import expand,compact

def record(source,repo,reports,output):
 sys.path[:0]=[str(source),str(source/'scripts')]
 from scripts.lint_lib.fact_engine import FactEngine,_run_plugin_reporter
 from scripts.lint_lib.fact_store import JsonFactStore
 from data_layer_probe_record import verify_sources
 records=[r for report in reports for r in expand(json.loads(report.read_text()))['records']];results=[]
 overrides={'generated_index_sources':'impl/common/l7245_versioned_backends/generic_sparse.py','graphql_plan_ownership':'l9750_graphql/probe.py','hnsw_architecture':'impl/common/l2100_sql_grammar/domain/sql_table_hnsw/state.py'}
 def observe(check,identity,files):
  module=importlib.import_module('scripts.lint_'+check)
  with tempfile.TemporaryDirectory() as temporary:
   root=Path(temporary);changed=[]
   for file in files:
    path=root/file['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_text(file['source'])
   # Relocate repository constants. Preserve the original detector and reporter code.
   oldroot=getattr(module,'ROOT',getattr(module,'REPO_ROOT',source))
   for name,value in list(vars(module).items()):
    if isinstance(value,Path) and value.is_absolute() and value.is_relative_to(oldroot):
     changed.append((name,value));setattr(module,name,root/value.relative_to(oldroot))
   previous=Path.cwd();os.chdir(root)
   try:
    if check in {'bounded_composed_graph_paths','matching_architecture'}:
     findings=module.collect_violations(root) if check=='bounded_composed_graph_paths' else module.lint(root)
     selected=sorted(p.relative_to(root).as_posix() for p in module._production_paths(root) if module._source_requires_matching_scan(p.relative_to(root),p.read_text())) if check=='matching_architecture' else None
    else:
     plugin=getattr(module,check+'_plugin')(root)
     engine=FactEngine(root=root,fact_store=JsonFactStore(root/'tmp/store'),extractors=getattr(module,check+'_fact_extractors')(root))
     selected=sorted(p.relative_to(root).as_posix() for p in engine.plan((plugin,)).files)
     findings=_run_plugin_reporter(plugin,engine.materialize((plugin,),max_workers=1)).violations
    expected=[]
    for finding in findings:
     path=Path(getattr(finding,'path',getattr(finding,'file_path',getattr(finding,'file',Path('')))))
     if path.is_absolute() and path.is_relative_to(root):path=path.relative_to(root)
     expected.append(dict(path=path.as_posix(),line=finding.line,code=getattr(finding,'code',getattr(finding,'rule','')),message=finding.message))
    expected.sort(key=lambda row:(row['path'],row['line'],row['code'],row['message']))
    results.append(dict(test=check+':'+identity,check=check,entry='all',files=files,parameters={},expected=expected,selected=selected))
   finally:
    os.chdir(previous)
    for name,value in changed:setattr(module,name,value)
 for check in CHECKS:
  choices=[r for r in records if r['check']==check and isinstance(r['expected'],list)]
  if check=='matching_architecture':choices=[r for r in choices if r['entry']=='scan_sources' and any(f['path'].startswith(('l1470_','l5250_','impl/common/')) for f in r['files'])]
  positive=next(r for r in choices if r['expected'])
  clean=next((r for r in choices if not r['expected']),None)
  def files(record):return [{**file,'path':overrides.get(check,file['path'])} for file in record['files']]
  observe(check,'positive',files(positive))
  if clean:observe(check,'clean',files(clean))
  positive_files=files(positive)
  if len(positive_files)==1:
   one=positive_files[0]
   for folder in ['tests','tmp','__pycache__','venv','unrelated']:
    path=Path(one['path']);location=(path.parent/folder/path.name).as_posix() if folder!='unrelated' else 'unrelated/'+path.name
    observe(check,'scope-'+folder,[dict(path=location,source=one['source'])])
 for text in ['def f(scope: PlanScope):\n    return (\n','x = (\n']:
  observe('graphql_plan_ownership','parser-'+hashlib.sha256(text.encode()).hexdigest()[:8],[dict(path='l9750_graphql/probe.py',source=text)])
 observe('matching_architecture','native-selection',[
  dict(path='l1700_other/plain.py',source='class Helper:\n    def match(self):\n        return self.conn.query()\n'),
  dict(path='l1700_other/continued.py',source='class Helper:\n    def \\\n        match(self):\n        return self.conn.query()\n'),
  dict(path='l1700_other/marked.py',source='# MatchSpec\nclass Helper:\n    def \\\n        match(self):\n        return self.conn.query()\n'),
  dict(path='l1700_other/clean.py',source='pass\n'),
  dict(path='l1470_matching_contracts/empty.py',source='pass\n'),
  dict(path='l1470_matching_contracts/tests/ignored.py',source='MatchSpec()\n'),
 ])
 report=dict(format='rerail.data-layer-batch-ten-defaults/v1',sourcePin=PIN,sourceFilesChecked=verify_sources(source,repo,PIN),records=results)
 output.write_text(json.dumps(compact(report),indent=2)+'\n')
def main():
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,required=True);p.add_argument('--repo',type=Path,required=True);p.add_argument('--reports',type=Path,nargs='+',required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();record(a.source.resolve(),a.repo.resolve(),a.reports,a.output)
if __name__=='__main__':main()
