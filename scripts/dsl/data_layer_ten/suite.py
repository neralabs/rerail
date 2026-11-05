# SPDX-License-Identifier: Apache-2.0
"""Compare complete detector results while running unchanged upstream assertions."""
from __future__ import annotations
import argparse,ast,hashlib,importlib,inspect,json,os,sys,re
from pathlib import Path
from functools import wraps
DSL=Path(__file__).resolve().parents[1];sys.path[:0]=[str(DSL),str(DSL/'data_layer_audit')]
from data_layer_nine.suite import Suite as NineSuite,normalize
from data_layer_nine.evidence import compact,expand
from data_layer_batch_eight import tree_source
from native_guard import codes
from data_layer_probe_record import verify_sources
from data_layer_ten.common import CHECKS,PIN
import pytest
ENTRIES={'generated_index_sources':['check_source','check_file'],'hnsw_architecture':['find_hnsw_architecture_findings'],'graphql_plan_ownership':['scan_source'],'schema_introspection_data_paths':['_collect_violations'],'embedding_vector_orchestration':['scan_source','scan_file','_violations_for_files'],'graph_lazy_sql_plan':['run_checks','_scan_versioned_lazy_mixin','_scan_versioned_transactions','_scan_graph_lazy_sql','_scan_intent_graph','_violations_for_sources','_violations_for_files'],'bounded_composed_graph_paths':['collect_violations'],'conditional_architecture':['collect_violations'],'matching_architecture':['scan_sources','lint','_scan_tree'],'plan_optimizer_v2':['scan_file','find_violations']}
GRAPH={'versioned_lazy_mixin':'l7200_versioned_transactions/versioned_lazy_mixin.py','versioned_transactions':'l7200_versioned_transactions/versioned_transactions.py','graph_lazy_sql':'l7200_versioned_transactions/graph_lazy_sql.py','intent_graph':'impl/common/l2070_server_plan/steps/intent_graph.py'}
class Suite(NineSuite):
 def pytest_configure(self):
  parse,walk=ast.parse,ast.walk
  @wraps(parse)
  def capture(source,*args,**kwargs):
   tree=parse(source,*args,**kwargs)
   if isinstance(source,(str,bytes)):
    for node in walk(tree):self.trees[node]=source.decode() if isinstance(source,bytes) else source
   return tree
  self.replace(ast,'parse',capture)
  self.modules={check:importlib.import_module('scripts.lint_'+check) for check in self.checks}
  original_codes=codes(self.modules)
  self.detector_codes={}
  for check,module in self.modules.items():
   for entry in ENTRIES[check]:
    if not hasattr(module,entry):continue
    original=getattr(module,entry);self.detector_codes[original.__code__]=check+':'+entry
    self.replace(module,entry,self.wrap(check,entry,module,original))
  if not self.oracle:
   def guard(frame,event,arg):
    if event!='call':return
    detail=original_codes.get(frame.f_code)
    if detail is not None:
     check,name,passive=detail;key=check+':'+name
     self.original_calls[key]=self.original_calls.get(key,0)+1
     semantic=not passive and ('Collector.' in name or name in {'_extract_hnsw_architecture_facts','_source_facts','_scan_inventory','_scan_inventory_facts','_summary_from_tree','_run_graph_collector_on_tree'})
    else:semantic=False
    if frame.f_code in self.detector_codes or semantic:
     name=self.detector_codes.get(frame.f_code,check+':'+name);self.semantic_fallbacks.append(name);raise AssertionError('Original detector executed: '+name)
   sys.setprofile(guard)
 def inputs(self,check,entry,module,arguments):
  root=Path(arguments.get('root') or getattr(module,'ROOT',getattr(module,'REPO_ROOT',self.root)))
  temporary=self.item.funcargs.get('tmp_path') if self.item else None
  if temporary is not None:root=Path(temporary)
  parameters={};files=[];originals={}
  def add(path,source=None):
   path=Path(path);name=path.relative_to(root).as_posix() if path.is_absolute() and path.is_relative_to(root) else path.as_posix()
   if source is None:
    try:source=path.read_text()
    except (OSError,UnicodeError):return
   files.append(dict(path=name,source=source));originals[name]=path
  if check=='graph_lazy_sql_plan':
   if entry=='run_checks':
    for name,path in GRAPH.items():add(path,arguments.get(name+'_source') if arguments.get(name+'_source') is not None else (root/path).read_text())
   elif entry.startswith('_scan_'):add(GRAPH[entry.removeprefix('_scan_')],arguments['source'])
   elif entry=='_violations_for_sources':
    for path,source in arguments['sources'].items():add(path,source)
   else:
    for path in arguments['files']:add(path)
  elif check=='matching_architecture':
   parameters['applicationPaths']=[str(p) for p in sorted(arguments.get('application_paths',()))]
   if entry=='_scan_tree':
    text=tree_source(arguments['tree'],self.trees)
    if text is None:raise AssertionError('Missing original tree source')
    add(arguments['path'],text)
    if arguments.get('application_path_override'):parameters['applicationPaths']=[files[0]['path']]
   elif entry=='scan_sources':
    for path,source in arguments['sources'].items():add(path,source)
   else:
    paths=module._production_paths(root)
    for application in arguments.get('application_roots',()):
     resolved=root/application
     if not resolved.is_dir():raise FileNotFoundError('Matching application root does not exist: '+str(application))
     for p in resolved.rglob('*.py'):
      if 'tests' not in p.parts:paths.add(p);parameters['applicationPaths'].append(p.relative_to(root).as_posix())
    for path in sorted(paths):
     text=path.read_text();relative=path.relative_to(root)
     if relative.as_posix() in parameters['applicationPaths'] or module._source_requires_matching_scan(relative,text):add(path,text)
    add(root/'data_layer_testing/matching_operator.py')
    originals={name:Path(name) for name in originals}
  elif check=='plan_optimizer_v2' and entry=='scan_file':add(arguments['path'],arguments['source'])
  elif check in {'bounded_composed_graph_paths','conditional_architecture','plan_optimizer_v2'}:
   for path in sorted(root.rglob('*')):
    if path.is_file() and path.suffix in {'.py','.rst','.md'} and not any(part in {'.git','.venv','venv','__pycache__'} for part in path.relative_to(root).parts):add(path)
  elif check=='hnsw_architecture':
   for path in arguments['paths']:add(path)
  else:
   path=arguments.get('path',arguments.get('file_path'));source=arguments.get('source');tree=arguments.get('tree',arguments.get('module'))
   if source is None and tree is not None:
    source=tree_source(tree,self.trees)
    if source is None:raise AssertionError('Missing original AST source')
   if check=='generated_index_sources' and entry=='check_source':source=None
   if path is not None:add(path,source)
   else:
    for path in arguments.get('files',()):add(path)
   if check=='embedding_vector_orchestration' and entry=='scan_source':
    for file in files:
     rel=file['path'];digest=hashlib.sha256(rel.encode()+b'\0'+file['source'].encode()).hexdigest()[:16]
     originals[rel]=module.REPO_ROOT/'tmp/embedding_vector_orchestration_scan'/digest/rel
  return files,originals,root,parameters
 def compare_record(self,record):
  if record['check']=='generated_index_sources':
   clean=lambda values:[re.sub(r'/[^ ]*/pytest-[0-9]+/[^/]+/','',value) for value in values]
   record={**record,'expected':clean(record['expected'])}
   for original in self.baseline:
    if original['check']=='generated_index_sources':original['expected']=clean(original['expected'])
  if record['check']=='graph_lazy_sql_plan':
   # FactEngine completes independent files concurrently; cross-file list order varies.
   record={**record,'expected':sorted(record['expected'],key=lambda r:json.dumps(r,sort_keys=True))}
   for original in self.baseline:
    if original['check']=='graph_lazy_sql_plan':original['expected']=sorted(original['expected'],key=lambda r:json.dumps(r,sort_keys=True))
  return super().compare_record(record)
 def dto(self,check,entry,module,rows,originals):
  values=[r['variables']['row'] for r in rows]
  values.sort(key=lambda r:(r['phase'],r.get('sort_name',''),r['ordering'],r['issue']))
  if check=='hnsw_architecture':values.sort(key=lambda r:(r['path'],r['line'],r['issue'],r['message']))
  if check in {'bounded_composed_graph_paths','matching_architecture','plan_optimizer_v2'}:values.sort(key=lambda r:(r['path'],r['line'],r['issue'],r['message']))
  if check=='conditional_architecture':values.sort(key=lambda r:(r['issue'],r['path'],r['line'],r['message']))
  result=[]
  for r in values:
   path=originals.get(r['path'],Path(r['path']));line=r['line'];code=r['issue'];message=r['message']
   if check=='generated_index_sources':value=f'{path}:{line}: {message}'
   elif check=='hnsw_architecture':value=module.HnswArchitectureFinding(path,line,code,message)
   elif check=='graphql_plan_ownership':value=module.StructuredViolation(module.LINTER_NAME,code,path,line,message)
   elif check=='schema_introspection_data_paths':value=module.Violation(path,line,r['call_name'],message)
   elif check=='embedding_vector_orchestration':value=module.Violation(path,line,r['qualname'],code,message)
   elif check=='graph_lazy_sql_plan':value=module.Violation(code,r['path'],line,message)
   elif check=='conditional_architecture':value=module.Violation(code,path,line,message)
   elif check=='matching_architecture':value=module.MatchingArchitectureFinding(code,path,line,r['symbol'],message)
   elif check=='plan_optimizer_v2':value=module.Finding(path,line,code,message)
   else:value=module.Violation(code,path,line,message)
   result.append(value)
  return tuple(result) if check=='hnsw_architecture' or check=='matching_architecture' and entry!='_scan_tree' else result
 def pytest_collection_modifyitems(self,items):
  items[:]=items[getattr(self,'part',0)::getattr(self,'parts',1)]
def main():
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,required=True);p.add_argument('--repo',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--policies',type=Path,required=True);p.add_argument('--oracle',action='store_true');p.add_argument('--baseline',type=Path);p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');p.add_argument('--checks',default=','.join(CHECKS));p.add_argument('--mutant',action='store_true');p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);p.add_argument('tests',nargs='*');args=p.parse_args()
 workspace=Path.cwd();root=args.source.resolve();output=args.output.resolve();output.parent.mkdir(parents=True,exist_ok=True);os.environ['RERAIL_NATIVE_POLICY_DIRECTORY']=str(args.policies.resolve());sys.path[:0]=[str(root),str(root/'scripts')]
 baseline=expand(json.loads(args.baseline.read_text())) if args.baseline else None
 if not args.oracle and baseline is None:p.error('A recorded original baseline is required')
 checks=tuple(args.checks.split(','));from data_layer_nine.discovery import upstream_tests,independent_collections
 tests=args.tests or [*upstream_tests(root,checks),str(Path(__file__).with_name('optimizer_cases.py')),str(Path(__file__).with_name('audit_cases.py'))];os.chdir(root);backends=[];exitcodes=[]
 for index,collection in enumerate(independent_collections(tests)):
  backend=Suite(root,workspace,args.mode,output.with_name(output.stem+f'-{index}.json'),args.oracle,baseline['records'] if baseline else None);backend.checks=checks;backend.part=args.part;backend.parts=args.parts;backend.mutant=args.mutant
  exitcodes.append(int(pytest.main(['--rootdir',str(root),'-c',str(root/'pyproject.toml'),'-q',*collection],plugins=[backend])));backends.append(backend)
 records=[r for b in backends for r in b.records];cases={k:v for b in backends for k,v in b.outcomes.items()};calls=[r for b in backends for r in b.calls];fallbacks=[r for b in backends for r in b.semantic_fallbacks]
 code=max(exitcodes);wanted=[r for r in baseline['records'] if r['test'] in cases] if baseline else records
 from collections import Counter
 missing=Counter((r['test'],r['check'],r['entry']) for r in wanted)-Counter((r['test'],r['check'],r['entry']) for r in records)
 if missing or fallbacks:code=1
 report=dict(format='rerail.data-layer-batch-ten/v1',oracle=args.oracle,mode=args.mode,exitCode=code,sourceFilesChecked=verify_sources(root,args.repo.resolve(),PIN),records=records,calls=calls,cases=cases,semanticFallbacks=fallbacks,originalCalls={k:sum(b.original_calls.get(k,0) for b in backends) for k in set().union(*(b.original_calls.keys() for b in backends))},retainedCaseIds=sorted(set(cases)-{r['test'] for r in records}),missingResults=list(missing.elements()),engineSha256=hashlib.sha256(backends[0].engine_path.read_bytes()).hexdigest())
 output.write_text(json.dumps(compact(report),indent=2)+'\n');raise SystemExit(code)
if __name__=='__main__':main()
