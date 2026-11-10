"""Transport blanket-bypass CLI arguments; native rules format and enforce results."""
from pathlib import Path
import os
import json


def metadata(files,owners,roles):
    return dict(bypassDocs=[f['path'] for f in files],bypassPaths=[owners[f['path']] for f in files],
                bypassRoles=[roles[f['path']] for f in files])


def prepared(suite,module,entry,root,paths,files=None,owners=None,roles=None):
    files=[] if files is None else files;owners={} if owners is None else owners;roles={} if roles is None else roles
    logical=[p.relative_to(root).as_posix() for p in paths]
    options=dict(sqlConnectionFile=module.SQL_CONNECTION_FILE,bypassCandidates=logical,
                 bypassIsFile=[str(p.is_file()).lower() for p in paths],**metadata(files,owners,roles))
    rows=suite.native('no_blanket_grammar_bypass',entry,files,options)
    if any(r['code']!='NGB_READ' for r in rows):raise AssertionError('Invalid bypass read plan')
    return [r['parameters']['ordinal'] for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]


def run_documents(suite,module,entry,files,owners,roles,exists=None):
    options=dict(sqlConnectionFile=module.SQL_CONNECTION_FILE,**metadata(files,owners,roles))
    if exists is not None:options['bypassTargetExists']=str(exists).lower()
    rows=suite.native('no_blanket_grammar_bypass',entry,files,options)
    if any(r['code'] not in {'NGB001','NGB_FACT'} for r in rows):raise AssertionError('Invalid bypass finding')
    if entry in {'scan_file_content','_bypass_facts'}:
        facts=[r for r in rows if r['code']=='NGB_FACT']
        facts.sort(key=lambda r:tuple(r['parameters'][k] for k in ['file_order','phase','order0','order1']))
        return [json.loads(r['parameters']['value_json']) for r in facts]
    findings=[r for r in rows if r['code']=='NGB001']
    order={owners[f['path']]:i for i,f in enumerate(files)}
    def owner(row):return owners.get(row['path'],row['path'])
    findings.sort(key=lambda r:(order.get(owner(r),-1),r['parameters'].get('collector_order0',0),
        r['parameters'].get('collector_order1',r['line']),r['parameters'].get('collector_order2',0),r['at']))
    return [f"  {owner(r)}:{r['line']}: {r['message']}" if r['line'] else f"  {owner(r)}: {r['message']}" for r in findings]


def fact_files(suite,module,root,paths,entry='_bypass_facts',exists=None):
    root=Path(root);paths=[p if p.is_absolute() else root/p for p in map(Path,paths)]
    selected=prepared(suite,module,'_bypass_prepare_facts',root,paths)
    files=[];owners={};roles={}
    for i in selected:
        key=f'disk-{i}.py';files.append(dict(path=key,source=paths[i].read_text(encoding='utf-8')))
        owners[key]=paths[i].relative_to(root).as_posix();roles[key]='disk'
    return run_documents(suite,module,entry,files,owners,roles,exists)


def scan(suite,module,entry,arguments):
    root=Path(arguments.get('project_root',arguments.get('root')))
    if entry=='check_no_public_grammar_methods':
        path=root/module.SQL_CONNECTION_FILE
        return fact_files(suite,module,root,[path],entry,path.exists())
    files=[];owners={};roles={}
    if entry=='scan_file_content':
        path=Path(arguments['path']);logical=path.relative_to(root).as_posix()
        files=[dict(path='supplied.py',source=arguments['source'])];owners['supplied.py']=logical;roles['supplied.py']='source'
        selected=prepared(suite,module,'_bypass_prepare_content',root,[path],files,owners,roles)
        for i in selected:
            key=f'disk-{i}.py';files.append(dict(path=key,source=path.read_text(encoding='utf-8')))
            owners[key]=logical;roles[key]='disk'
    else:
        paths=[]
        for directory,names,filenames in os.walk(root):
            names[:]=[n for n in names if n not in {'__pycache__','.venv','.git','tmp'}]
            paths.extend(Path(directory)/name for name in filenames)
        selected=prepared(suite,module,'_bypass_prepare_refs',root,paths)
        for i in selected:
            try:source=paths[i].read_text(encoding='utf-8')
            except UnicodeDecodeError:continue
            key=f'source-{i}.py';files.append(dict(path=key,source=source));owners[key]=paths[i].relative_to(root).as_posix();roles[key]='source'
    return run_documents(suite,module,entry,files,owners,roles)


def install(suite,module,detectors=False):
    import argparse
    import sys
    check='no_blanket_grammar_bypass'
    settings=suite.native(check,'_configuration',[])
    if not settings or any(r['code'] not in {'NGB_CONFIG','NGB_EXTRACTOR'} for r in settings):raise ValueError('Native bypass configuration is missing')
    extractors=[r for r in settings if r['code']=='NGB_EXTRACTOR']
    if hasattr(module,'NO_BLANKET_GRAMMAR_BYPASS_EXTRACTOR'):
        if len(extractors)!=1:raise ValueError('Native bypass extractor metadata is missing')
        data=json.loads(extractors[0]['parameters']['value'])
        for key in ['source_triggers','project_input_facts','node_kinds']:data[key]=tuple(data[key])
        data['source_trigger_groups']=tuple(tuple(g) for g in data['source_trigger_groups'])
        suite.replace(module,'NO_BLANKET_GRAMMAR_BYPASS_EXTRACTOR',type(module.NO_BLANKET_GRAMMAR_BYPASS_EXTRACTOR)(**data))
    def values(key):return [r['parameters']['value'] for r in settings if r['code']=='NGB_CONFIG' and r['parameters']['key']==key]
    for name,key in [('ALLOWED_FILES','allowed'),('FORBIDDEN_PUBLIC_METHODS','method')]:
        if set(values(key))!=getattr(module,name):raise ValueError('Native bypass configuration differs: '+name)
    if values('sql')!=[module.SQL_CONNECTION_FILE]:raise ValueError('Native SQL connection path differs')
    if hasattr(suite,'configuration_bindings'):
        suite.configuration_bindings.setdefault(check,dict(nativeCall=len(suite.calls)-1,names=['ALLOWED_FILES','FORBIDDEN_PUBLIC_METHODS','SQL_CONNECTION_FILE']))
    suite.replace(module,'_facts_for_files',lambda root,files:fact_files(suite,module,root,files))
    if detectors:
        suite.replace(module,'check_no_bypass_references',lambda project_root:scan(suite,module,'check_no_bypass_references',dict(project_root=project_root)))
        suite.replace(module,'check_no_public_grammar_methods',lambda project_root:scan(suite,module,'check_no_public_grammar_methods',dict(project_root=project_root)))
        suite.replace(module,'scan_file_content',lambda root,path,source:scan(suite,module,'scan_file_content',dict(root=root,path=path,source=source)))
    def cli_main():
        parser=argparse.ArgumentParser(description=values('description')[0])
        parser.add_argument('--check',action='store_true',help=values('check_help')[0]);args=parser.parse_args()
        root=Path(module.__file__).parent.parent
        bypass=module.check_no_bypass_references(root)
        methods=module.check_no_public_grammar_methods(root)
        rows=suite.native(check,'_cli_report',[],dict(bypassRows=bypass,methodRows=methods,checkMode=str(int(args.check))))
        if any(r['code'] not in {'NGB_OUTPUT','NGB_EXIT'} for r in rows):raise ValueError('Unexpected native bypass CLI output')
        for r in sorted((r['parameters'] for r in rows if r['code']=='NGB_OUTPUT'),key=lambda r:(r['phase'],r['ordinal'])):
            sys.stdout.write(r['stdout']);sys.stderr.write(r['stderr'])
        exits=[r['parameters']['value'] for r in rows if r['code']=='NGB_EXIT']
        if len(exits)!=1:raise ValueError('One native bypass exit value is required')
        return exits[0]
    suite.replace(module,'cli_main',cli_main)
