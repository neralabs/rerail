#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Record supplemental eighth-batch cases through the unchanged pinned oracle."""
import argparse,importlib,json,sys,tempfile
from dataclasses import asdict,replace
from pathlib import Path
from data_layer_batch_eight import MODULES,normalize
from data_layer_probe_record import verify_sources

PIN='2b1504adc4291daaf080b2e932962b134da1a69a'
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('source','repo','inputs','output'):parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();source=args.source.resolve();sys.path[:0]=[str(source),str(source/'scripts')]
    events=[]
    for case in json.loads(args.inputs.read_text())['cases']:
        check=case['check'];module=importlib.import_module(MODULES[check]);files=case['files']
        with tempfile.TemporaryDirectory(prefix='rerail-batch8-oracle-') as temporary:
            root=Path(temporary).resolve()
            for file in files:
                path=root/file['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_text(file['source'])
            originals={name:getattr(module,name) for name in ('ROOT','REPO_ROOT') if hasattr(module,name)}
            try:
                for name in originals:setattr(module,name,root)
                entry=case.get('entry','scan_source')
                if check=='cached_pure_lazy_allowlist':
                    module_name=case.get('module',files[0]['path'][:-3].replace('/','.'))
                    if entry=='policy_violations':
                        actual=module.scan_cached_pure_lazy_source(root/files[0]['path'],files[0]['source'],module_name=module_name)
                        seed=module.scan_cached_pure_lazy_source(root/files[0]['path'],case.get('policySource',files[0]['source']),module_name=module_name)
                        selected=[]
                        for item in seed if case.get('signed',True) else ():
                            selected.append(module.CachedPureLazyPolicyEntry(function=item.function,cache_args=item.cache_args,cache_types=tuple((name,'l2070_batching.optimizer_run_cache.OptimizerRunCache') for name in item.cache_args),allowed_cache_members=item.cache_members,body_hash=item.body_hash,signature_hash=item.signature_hash,owner_scope='optimizer_run',rationale='Reviewed supplemental test contract.'))
                    elif entry=='memoized_policy_violations':
                        actual=module.discover_memoized_pure_lazy_functions(root)
                        seed_tree=module.ast.parse(case.get('policySource',files[0]['source']))
                        seed=module._memoized_declarations_in_body(seed_tree.body,module_name=module_name,path=root/files[0]['path'])
                        selected=[module.MemoizedPureLazyPolicyEntry(function=item.function,max_entries=item.max_entries,body_hash=item.body_hash,signature_hash=item.signature_hash,cache_lifetime='process',key_policy='bound_explicit_immutable_v1',return_policy='immutable_no_alias_v1',exception_policy='successful_results_only_v1',rationale='Reviewed supplemental test contract.') for item in seed] if case.get('signed',True) else []
                    else:raise ValueError('Unknown cached policy probe entry '+entry)
                    changes=case.get('policyChanges',{})
                    selected=[replace(item,**{key:tuple(tuple(v) if isinstance(v,list) else v for v in value) if isinstance(value,list) else value for key,value in changes.items()}) for item in selected]
                    if case.get('duplicatePolicy'):selected+=selected
                    raw=getattr(module,entry)(actual,tuple(selected))
                    policy_entries=json.dumps([asdict(item) for item in selected],sort_keys=True)
                elif check in {'readthrough_authority_architecture','authentication_plan_purity'}:
                    raw=module.scan_sources(tuple(root/file['path'] for file in files));entry='scan_sources'
                elif check=='lazy_architecture':
                    raw=module.scan_files([root/file['path'] for file in files]);entry='scan_files'
                elif check=='plan_helpers_allowlist':
                    if entry=='scan_repo':raw=module.scan_repo(root)
                    else:raw=module.scan_file(root/files[0]['path']);entry='scan_file'
                elif check in {'legacy_callback_quarantine','lazy_sql_surface'} and entry=='scan_repo':raw=module.scan_repo(root)
                else:raw=getattr(module,entry)(root/files[0]['path'],files[0]['source'])
            finally:
                for name,value in originals.items():setattr(module,name,value)
            def logical(path):
                p=Path(path)
                return p.relative_to(root).as_posix() if p.is_absolute() and p.is_relative_to(root) else p.as_posix()
            findings=normalize(check,raw,files,logical)
            events.append({'test':'probe:batch8:'+case['id'],'check':check,'entry':entry,'call':1,'upstream_passed':True,
                'scope':{'include':['**/*.py'],'exclude':[]},'production_scope':False,'files':files,'findings':findings,
                'oracle_config':{'options':{'oracleAnalyzer':True,'entryPoint':entry,**({'policyEntries':policy_entries} if check=='cached_pure_lazy_allowlist' else {})}}})
    report={'format':'rerail.data-layer-recording/v1','commit':PIN,'checks':sorted({e['check'] for e in events}),
            'source_mismatches':[],'pytest_exit_code':0,'collected_tests':0,'uncaptured_tests':[],
            'loaded_source_files_checked':verify_sources(source,args.repo,PIN),'events':events}
    args.output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'observations':len(events),'positive':sum(bool(e['findings']) for e in events),'clean':sum(not e['findings'] for e in events)}))
if __name__=='__main__':main()
