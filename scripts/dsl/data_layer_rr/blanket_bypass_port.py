"""Transport blanket-bypass CLI arguments; native rules format and enforce results."""
from pathlib import Path
import os


def scan(suite,module,entry,arguments):
    root=Path(arguments.get('project_root',arguments.get('root')));files=[]
    if entry=='scan_file_content':
        path=Path(arguments['path']);files=[dict(path=path.relative_to(root).as_posix(),source=arguments['source'])]
    elif entry=='check_no_public_grammar_methods':
        target=root/module.SQL_CONNECTION_FILE
        if target.exists():files=[dict(path=module.SQL_CONNECTION_FILE,source=target.read_text(encoding='utf-8'))]
    else:
        for directory,names,filenames in os.walk(root):
            names[:]=[n for n in names if n not in {'__pycache__','.venv','.git','tmp'}]
            for name in filenames:
                if not name.endswith('.py'):continue
                path=Path(directory)/name
                try:source=path.read_text(encoding='utf-8')
                except UnicodeDecodeError:continue
                files.append(dict(path=path.relative_to(root).as_posix(),source=source))
    rows=suite.native('no_blanket_grammar_bypass',entry,files,dict(sqlConnectionFile=module.SQL_CONNECTION_FILE))
    order={f['path']:i for i,f in enumerate(files)}
    rows.sort(key=lambda r:(order.get(r['path'],-1),r['line'],r['parameters'].get('ordinal',-1),r['at']))
    if entry=='scan_file_content':return [dict(file_path=r['path'],line=r['line'],message=r['message']) for r in rows]
    return [f"  {r['path']}:{r['line']}: {r['message']}" if r['line'] else f"  {r['path']}: {r['message']}" for r in rows]


def install(suite,module,detectors=False):
    import argparse
    import sys
    check='no_blanket_grammar_bypass'
    settings=suite.native(check,'_configuration',[])
    if not settings or any(r['code']!='NGB_CONFIG' for r in settings):raise ValueError('Native bypass configuration is missing')
    def values(key):return [r['parameters']['value'] for r in settings if r['parameters']['key']==key]
    for name,key in [('ALLOWED_FILES','allowed'),('FORBIDDEN_PUBLIC_METHODS','method')]:
        if set(values(key))!=getattr(module,name):raise ValueError('Native bypass configuration differs: '+name)
    if values('sql')!=[module.SQL_CONNECTION_FILE]:raise ValueError('Native SQL connection path differs')
    if hasattr(suite,'configuration_bindings'):
        suite.configuration_bindings.setdefault(check,dict(nativeCall=len(suite.calls)-1,names=['ALLOWED_FILES','FORBIDDEN_PUBLIC_METHODS','SQL_CONNECTION_FILE']))
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
