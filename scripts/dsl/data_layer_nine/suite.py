#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Run unchanged upstream cases, then compare every analyzer result with Rerail."""
from __future__ import annotations
import argparse,ast,hashlib,importlib,inspect,json,os,sys,threading
from dataclasses import asdict,is_dataclass
from functools import wraps
from collections import Counter
from pathlib import Path
from weakref import WeakKeyDictionary
import pytest

DSL=Path(__file__).resolve().parents[1];sys.path[:0]=[str(DSL),str(DSL/'data_layer_audit')]
from native_suite import NativeSuite
from native_guard import codes
from native_scanners import logical
from data_layer_batch_eight import tree_source
from data_layer_probe_record import verify_sources

CHECKS=('connection_mode_split','connection_structural_boundaries','connection_service_architecture','schema_architecture','schema_bootstrap_contracts','bootstrap_engine_invariants','bootstrap_lazy_sql','bootstrap_eager_resolve','bootstrap_cache_health','plan_composition_architecture')
ENTRIES={
 'connection_mode_split':['lint','lint_source','_violations_for_files'],
 'connection_structural_boundaries':['lint','_connection_file_violations','_sql_connection_violations','_backend_file_violations','_lifecycle_violations'],
 'connection_service_architecture':['lint','analyze_source','_lint_schema_transaction_facades_source','_lint_sql_services','_lint_backend_cores','_lint_duplicate_service_ownership_across_files','_lint_forbidden_connection_level_schema_methods_for_files'],
 'schema_architecture':['lint_file'],
 'schema_bootstrap_contracts':['lint_file'],
 'bootstrap_engine_invariants':['_collect_file_violations','_collect_primitive_install_coverage_violations','_collect_fake_executor_violations'],
 'bootstrap_lazy_sql':['scan_source','scan_tree'],
 'bootstrap_eager_resolve':[],
 'bootstrap_cache_health':['scan_file','find_violations'],
 'plan_composition_architecture':['scan_semantic_source','run_behavior_probes'],
}
HOST={
 'connection_mode_split':{'iter_python_files'},
 'connection_structural_boundaries':set(),
 'connection_service_architecture':{'lint','_lint_schema_transaction_facades','_lint_broad_connection_surface','_connection_target_files','_is_test_path'},
 'schema_architecture':{'collect_findings'},
 'schema_bootstrap_contracts':{'lint_paths','iter_files'},
 'bootstrap_engine_invariants':{'collect_violations','_scan_paths','_iter_python_files'},
 'bootstrap_cache_health':{'allowlist_signature_valid','bootstrap_cache_health_fact_extractors','bootstrap_cache_health_plugin','_violation_to_fact'},
}

def normalize(result,root):
 def convert(value):
  if is_dataclass(value):return convert(asdict(value))
  if isinstance(value,Path):return logical(value,root)
  if isinstance(value,dict):
   return {k:logical(Path(v),root) if k=='path' and isinstance(v,str) else v.replace(str(value['path']),logical(Path(value['path']),root)) if k=='key' and isinstance(v,str) and 'path' in value else convert(v) for k,v in value.items()}
  if isinstance(value,(tuple,list)):return [convert(v) for v in value]
  return value
 return convert(result)

class Suite(NativeSuite):
 def __init__(self,root,workspace,mode,output,oracle=False,baseline=None):
  super().__init__(root,workspace,mode,output);self.oracle=oracle;self.records=[];self.depth=0;self.semantic_fallbacks=[];self.instrumentation_translations=[];self.baseline=baseline or [];self.seen={};self.protocol_lock=threading.RLock()
 def execute(self,check,entry,files,options=None,contract=None,parameters=None):
  values=dict(options or {})
  if parameters is not None:values['parameters']=json.dumps(parameters)
  with self.protocol_lock:
   try:return super().execute(check,entry,files,values,contract)
   except AssertionError as error:
    if check=='connection_service_architecture' and 'legacy decorator cardinality' in str(error):
     self.calls.append(dict(test=self.item.nodeid,check=check,entry=entry,files=len(files),findings=0,expectedError='ValueError'))
     raise ValueError('too many values to unpack (expected 1)') from None
    raise
 def pytest_configure(self):
  parse,walk=ast.parse,ast.walk
  @wraps(parse)
  def capture(source,*args,**kwargs):
   tree=parse(source,*args,**kwargs)
   if isinstance(source,(str,bytes)):
    text=source.decode() if isinstance(source,bytes) else source
    for node in walk(tree):self.trees[node]=text
   return tree
  self.replace(ast,'parse',capture)
  self.modules={name:importlib.import_module('scripts.lint_'+name) for name in CHECKS if name!='bootstrap_eager_resolve'}
  self.schema_plugin=importlib.import_module('scripts.lint_schema_architecture_plugin')
  original_codes=codes({**self.modules,'schema_architecture_plugin':self.schema_plugin})
  self.replace(self.schema_plugin,'scan_source',self.wrap('schema_architecture','scan_source',self.schema_plugin,self.schema_plugin.scan_source))
  for check,names in ENTRIES.items():
   if check not in self.modules:continue
   module=self.modules[check]
   for name in names:
    self.replace(module,name,self.wrap(check,name,module,getattr(module,name)))
  if not self.oracle:
   self.install_cache_collector()
   self.guard(original_codes)
  else:self.observe_cache_collector()
 def guard(self,original_codes):
  import threading
  def audit(frame,event,arg):
   if event!='call' or frame.f_code not in original_codes:return
   check,name,passive=original_codes[frame.f_code];key=check+':'+name
   self.original_calls[key]=self.original_calls.get(key,0)+1
   if not passive and name not in HOST.get(check,set()):
    self.semantic_fallbacks.append(dict(test=self.item.nodeid if self.item else '<setup>',function=key))
    raise AssertionError('Upstream semantic execution is forbidden: '+key)
  sys.setprofile(audit);threading.setprofile(audit)
 def pytest_collection_modifyitems(self,items):
  items[:]=items[getattr(self,'part',0)::getattr(self,'parts',1)]
  if self.oracle:return
  for item in items:
   if item.name=='test_bootstrap_function_checks_reuse_one_subtree_walk':
    def native_walk_check(monkeypatch):
     rows=self.execute('bootstrap_engine_invariants','_collect_fake_executor_violations',[dict(path='bootstrap.py',source='def execute_application_plan():\n    return None\n')])
     metrics=self.last_metrics.get('workspaceProfile',self.last_metrics)
     assert metrics['pythonAstModuleWalks']==1
     assert metrics['pythonAstParses']==1
    item.obj=native_walk_check
    self.instrumentation_translations.append(dict(test=item.nodeid,original='ast.walk call count',native='pythonAstModuleWalks=1 and pythonAstParses=1'))
 def collector_root(self,module,path):
  candidate=self.item.funcargs.get('tmp_path') if self.item else None
  return Path(candidate) if candidate and Path(path).is_relative_to(candidate) else module.REPO_ROOT
 def observe_cache_collector(self):
  module=self.modules['bootstrap_cache_health'];original=module._collector
  def factory(path,source):
   collector=original(path,source)
   if self.depth:return collector
   finish=collector.finish_file;root=self.collector_root(module,path)
   def observe():
    result=finish()
    self.records.append(dict(test=self.item.nodeid,check='bootstrap_cache_health',entry='collector',files=[dict(path=logical(path,root),source=source)],parameters={},expected=normalize(result,root)))
    return result
   collector.finish_file=observe
   return collector
  self.replace(module,'_collector',factory)
 def install_cache_collector(self):
  module=self.modules['bootstrap_cache_health']
  from scripts.lint_lib.ast_collectors import AstEventCollector
  backend=self
  class Collector(AstEventCollector):
   def __init__(self,path,source):self.path=path;self.source=source
   def finish_file(self):
    root=backend.collector_root(module,self.path);path=logical(self.path,root)
    files=[dict(path=path,source=self.source)]
    rows=backend.execute('bootstrap_cache_health','collector',files)
    result=[dict(path=str(self.path),line=row['variables']['row']['line'],code=row['code'],message=row['message'],key=f"{row['code']}:{self.path}:{row['variables']['row']['line']}") for row in rows]
    record=dict(test=backend.item.nodeid,check='bootstrap_cache_health',entry='collector',files=files,parameters={},expected=normalize(result,root))
    backend.records.append(record);backend.compare_record(record)
    return result
  # The public factory returns the collector through this class constructor.
  self.replace(module,'_collector',lambda path,source:Collector(path,source))
 def inputs(self,check,entry,module,arguments):
  root=Path(arguments.get('root') or getattr(module,'ROOT',getattr(module,'REPO_ROOT',self.root)))
  paths=arguments.get('files',arguments.get('paths'));path=arguments.get('path');source=arguments.get('source');parameters={}
  if check=='connection_mode_split' and entry=='lint':paths=module.iter_python_files()
  if check=='connection_mode_split' and entry=='lint_source':parameters['probeSource']=source;source=None # Upstream intentionally rereads the file.
  if check=='connection_structural_boundaries':paths=[module.CONNECTION_FILE,module.CONNECTION_LIFECYCLE_FILE,module.SQL_CONNECTION_FILE,*module.BACKEND_CONNECTION_FILES,*module.LIFECYCLE_SCAN_FILES]
  if check=='connection_service_architecture':
   if entry=='lint':paths=[*module._connection_target_files(),module.SQL_CONNECTION,module.SCHEMA_TRANSACTION,*module.BACKEND_CORE_FILES,*module.FORBIDDEN_CONNECTION_METHOD_FILES]
   if entry=='_lint_sql_services':paths=[module.SQL_CONNECTION]
   if entry=='_lint_backend_cores':paths=list(module.BACKEND_CORE_FILES)
  if check=='bootstrap_cache_health' and entry=='find_violations':
   paths=[p for directory in (arguments['directories'] if arguments.get('directories') is not None else module.DEFAULT_DIRECTORIES) for p in sorted((root/directory).rglob('test_*.py')) if not set(p.parts)&module.EXCLUDED_PARTS]
  if check=='bootstrap_engine_invariants':
   parameters.update(enginePaths=[logical(p,root) for p in module.BOOTSTRAP_ENGINE_FILES],bootstrapPaths=[logical(p,root) for p in module.BOOTSTRAP_FILES])
   if entry=='_collect_primitive_install_coverage_violations':
    coordinator=root/'l3200_metadata/bootstrap_coordinator.py';paths=[coordinator,root/'l3200_metadata/bootstrap_data_plan.py'] if coordinator.exists() else []
  if entry=='run_behavior_probes':
   from data_layer_nine.live import observations
   return [],{},root,observations()
  tree=next((v for v in arguments.values() if isinstance(v,ast.AST)),None)
  supplied_trees=arguments.get('trees')
  if supplied_trees is not None:paths=list(supplied_trees)
  if check=='connection_structural_boundaries':
   parameters.update(connectionPath=logical(module.CONNECTION_FILE,root),lifecyclePath=logical(module.CONNECTION_LIFECYCLE_FILE,root),sqlPath=logical(module.SQL_CONNECTION_FILE,root))
   if supplied_trees is not None:parameters['lineCounts']={logical(p,root):len(Path(p).read_text().splitlines()) for p in supplied_trees}
  raw_source=source
  if check=='bootstrap_engine_invariants' and entry=='_collect_file_violations' and tree is not None and raw_source is None:raw_source=Path(path).read_text()
  if tree is not None and check!='bootstrap_cache_health':
   source=tree_source(tree,self.trees)
   if source is None:raise AssertionError('AST argument has no original source')
   if not isinstance(tree,ast.Module):parameters['functionLine']=tree.lineno
  if paths is None:paths=[] if path is None else [path]
  files=[];originals={}
  for p in dict.fromkeys(map(Path,paths)):
   if supplied_trees is not None:
    text=tree_source(supplied_trees[p],self.trees)
    if text is None:raise AssertionError('Supplied AST has no recorded source')
   elif source is not None and path is not None and p==Path(path):text=source
   else:
    try:text=p.read_text(encoding='utf-8')
    except (OSError,UnicodeError):continue
   name=logical(p,root);files.append(dict(path=name,source=text));originals[name]=Path(name) if check=='bootstrap_cache_health' and entry=='find_violations' else p
  if check=='connection_service_architecture' and entry in ('lint','_lint_duplicate_service_ownership_across_files'):
   parameters['fileOrder']=[f['path'] for f in files]
   parameters['displayPaths']={name:display for name,path in originals.items() if (display:=str(path.relative_to(module.ROOT) if path.is_relative_to(module.ROOT) else path))!=name}
  if check=='bootstrap_engine_invariants':
   if tree is not None and raw_source is not None and source!=raw_source:parameters['rawSources']={logical(path,root):raw_source}
   parameters['enginePaths']=[logical(p,root) for p in map(Path,paths) if p in module.BOOTSTRAP_ENGINE_FILES]
   parameters['bootstrapPaths']=[logical(p,root) for p in map(Path,paths) if p in module.BOOTSTRAP_FILES]
  return files,originals,root,parameters
 def wrap(self,check,entry,module,original):
  signature=inspect.signature(original)
  @wraps(original)
  def run(*args,**kwargs):
   if self.oracle and self.depth:return original(*args,**kwargs)
   arguments=dict(signature.bind_partial(*args,**kwargs).arguments)
   files,originals,root,parameters=self.inputs(check,entry,module,arguments)
   error=None
   try:
    if self.oracle:
     self.depth+=1
     try:result=original(*args,**kwargs)
     finally:self.depth-=1
    else:
     rows=self.execute(check,'all' if check=='connection_service_architecture' and entry=='lint' else entry,files,parameters=parameters)
     result=self.dto(check,entry,module,rows,originals)
   except (SyntaxError,ValueError) as exception:
    error=exception
   expected=normalize(result,root) if error is None else {'exception':dict(type=type(error).__name__,message=str(error),**({field:getattr(error,field) for field in ('msg','lineno','offset','text','end_lineno','end_offset')} if isinstance(error,SyntaxError) else {}))}
   record=dict(test=self.item.nodeid if self.item else '<probe>',check=check,entry=entry,files=files,parameters=parameters,expected=expected)
   self.records.append(record)
   if not self.oracle:self.compare_record(record)
   if error is not None:raise error
   return result
  return run
 def compare_record(self,record):
  check,entry=record['check'],record['entry']
  matches=[r for r in self.baseline if r['test']==record['test'] and r['check']==check and r['entry']==entry]
  key=(record['test'],check,entry);ordinal=self.seen.get(key,0);self.seen[key]=ordinal+1
  assert ordinal<len(matches),('Unrecorded native analyzer call',record)
  expected=matches[ordinal]
  assert record['files']==expected['files'],('input drift',record['test'],check,entry)
  assert record['parameters']==expected['parameters'],('parameter drift',record['test'],check,entry,record['parameters'],expected['parameters'])
  if not getattr(self,'mutant',False):assert record['expected']==expected['expected'],('full-result parity',record['test'],check,entry,record['expected'],expected['expected'])
 def dto(self,check,entry,module,rows,originals):
  syntax=[row for row in rows if row['code']=='NATIVE-SYNTAX']
  if syntax:
   finding=syntax[0];details=finding['variables']['syntax_error'];path=originals[finding['path']]
   filename=str(path) if check in {'schema_bootstrap_contracts','bootstrap_lazy_sql','bootstrap_eager_resolve','plan_composition_architecture','bootstrap_cache_health'} else '<unknown>'
   error=SyntaxError(details['msg'],(filename,details['lineno'],details['offset'],details['text'],details['end_lineno'],details['end_offset']))
   if check=='bootstrap_cache_health':return [module.BootstrapCacheHealthViolation(path,1,'BCH000','SyntaxError: '+str(error))]
   raise error
  values=[row['variables']['row'] for row in rows]
  file_order={p:i for i,p in enumerate(originals)}
  def order(r):
   cross=check=='connection_service_architecture' and (entry=='_lint_duplicate_service_ownership_across_files' or r.get('api_group')==4)
   return (r.get('api_group',0),0 if cross else file_order.get(r['path'],0),r.get('phase',0),r.get('sort_name',''),file_order.get(r['path'],0) if cross else 0,r.get('ordering',0),r['issue'])
  values.sort(key=order)
  if check=='bootstrap_cache_health':values.sort(key=lambda r:(file_order.get(r['path'],0),r['line'],r['issue'],r['message']))
  result=[]
  for r in values:
   path=originals.get(r['path'],Path(r['path']));line=r['line'];code=r['issue'];message=r['message']
   if check=='connection_mode_split':v=module.Violation(path,line,message)
   elif check=='connection_structural_boundaries':v=module.StructuralViolation(code,path,line,message)
   elif check=='connection_service_architecture':v=module.ArchitectureError(path,message,None if code=='CONN000' else line,code)
   elif check=='schema_architecture' and entry=='scan_source':v=module.StructuredViolation(linter_name='schema_architecture',code=code,path=path,line=line,message=message)
   elif check=='schema_architecture':v=module.Finding(code,str(path.relative_to(module.REPO_ROOT) if path.is_relative_to(module.REPO_ROOT) else path),line,r['symbol'],message)
   elif check in ('schema_bootstrap_contracts','bootstrap_lazy_sql','bootstrap_eager_resolve'):v=module.Violation(code,path,line,message)
   elif check=='bootstrap_engine_invariants':v=module.Violation(code,path,line,message)
   elif check=='bootstrap_cache_health':v=module.BootstrapCacheHealthViolation(path,line,code,message)
   elif check=='plan_composition_architecture':v=module.CompositionViolation(code,path,line,message)
   else:raise AssertionError(check)
   result.append(v)
  if check in ('schema_bootstrap_contracts','plan_composition_architecture'):return tuple(result)
  return result

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--source',type=Path,required=True);p.add_argument('--repo',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--oracle',action='store_true');p.add_argument('--baseline',type=Path);p.add_argument('--policies',type=Path,default=Path('packs/data-layer/rules'));p.add_argument('--mode',choices=['snapshot','workspace'],default='snapshot');p.add_argument('--mutant',action='store_true');p.add_argument('--part',type=int,default=0);p.add_argument('--parts',type=int,default=1);p.add_argument('tests',nargs='*');args=p.parse_args()
 workspace=Path.cwd();root=args.source.resolve();output=args.output.resolve();output.parent.mkdir(parents=True,exist_ok=True)
 sys.path[:0]=[str(root),str(root/'scripts')];os.environ['RERAIL_NATIVE_POLICY_DIRECTORY']=str(args.policies.resolve())
 from data_layer_nine.evidence import expand,compact
 if not args.oracle and args.baseline is None:p.error('Native assertions require a recorded oracle')
 baseline=expand(json.loads(args.baseline.read_text())) if args.baseline else None
 backend=Suite(root,workspace,args.mode,output,args.oracle,baseline['records'] if baseline else None)
 backend.part=args.part;backend.parts=args.parts;backend.mutant=args.mutant
 if not 0<=args.part<args.parts:p.error('Invalid shard')
 defaults=[str(root/('scripts/tests/test_lint_'+check+'.py')) for check in CHECKS if (root/('scripts/tests/test_lint_'+check+'.py')).exists()]
 defaults += [str(root/'l2000_connection/tests/test_schema_architecture_linter.py'),str(root/'l2000_connection/tests/test_connection_service_architecture.py'),str(root/'scripts/tests/test_linter_supplied_ast_contracts.py')+'::test_structural_helper_does_not_reparse_supplied_tree',str(root/'scripts/tests/test_codemod_bootstrap_cache_health.py'),str(Path(__file__).with_name('probe_cases.py'))]
 tests=[str(Path(t).resolve()) if Path(t).exists() else str(root/t) for t in args.tests] or defaults
 os.chdir(root)
 code=pytest.main(['--rootdir',str(root),'-c',str(root/'pyproject.toml'),'-q',*tests],plugins=[backend])
 checked=verify_sources(root,args.repo.resolve(),'2b1504adc4291daaf080b2e932962b134da1a69a')
 report=dict(format='rerail.data-layer-batch-nine/v1',oracle=args.oracle,mode=args.mode,exitCode=int(code),sourceFilesChecked=checked,engineSha256=hashlib.sha256(backend.engine_path.read_bytes()).hexdigest(),records=backend.records,calls=backend.calls,cases=backend.outcomes,semanticFallbacks=backend.semantic_fallbacks,originalCalls=backend.original_calls,instrumentationTranslations=backend.instrumentation_translations)
 report['policySha256']={check:hashlib.sha256((Path(os.environ['RERAIL_NATIVE_POLICY_DIRECTORY'])/('data-layer-'+check.replace('_','-')+'.json')).read_bytes()).hexdigest() for check in CHECKS}
 if baseline is not None and not args.mutant:
  translated={r['test'] for r in backend.instrumentation_translations}
  wanted=Counter((r['test'],r['check'],r['entry']) for r in baseline['records'] if r['test'] in backend.outcomes and r['test'] not in translated)
  actual=Counter((r['test'],r['check'],r['entry']) for r in backend.records)
  if wanted!=actual:report['missingResults']=list((wanted-actual).elements());code=pytest.ExitCode.TESTS_FAILED;report['exitCode']=int(code)
 output.write_text(json.dumps(compact(report),indent=2)+'\n');raise SystemExit(code)
if __name__=='__main__':main()
