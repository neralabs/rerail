#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Record supplemental eighth-batch cases through the unchanged pinned oracle."""
import argparse,importlib,json,sys,tempfile
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
                if check in {'readthrough_authority_architecture','authentication_plan_purity'}:
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
                'oracle_config':{'options':{'oracleAnalyzer':True,'entryPoint':entry}}})
    report={'format':'rerail.data-layer-recording/v1','commit':PIN,'checks':sorted({e['check'] for e in events}),
            'source_mismatches':[],'pytest_exit_code':0,'collected_tests':0,'uncaptured_tests':[],
            'loaded_source_files_checked':verify_sources(source,args.repo,PIN),'events':events}
    args.output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'observations':len(events),'positive':sum(bool(e['findings']) for e in events),'clean':sum(not e['findings'] for e in events)}))
if __name__=='__main__':main()
