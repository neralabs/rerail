"""Transport ForwardRef CLI arguments, filesystem observations and native rows."""
import argparse
import json
from pathlib import Path
import sys

CHECK='forwardref'


def one(rows,code,key):
    values=[r['parameters'][key] for r in rows if r['code']==code]
    if len(values)!=1:raise ValueError('One native '+code+' required')
    return values[0]


def install(backend,module,config):
    discovery_globals=module.resolve_root_and_dirs.__globals__
    def resolve(args,*,default_dirs=None):
        rows=backend.native(CHECK,'_fr_cli_root',[],dict(frCliHasRoot=str(int(args.root is not None)),frCliRoot=args.root or '',frDiscoverySource=str(Path(discovery_globals['__file__']).resolve()),frCliHasDirs=str(int(args.dirs is not None)),frCliDirs=list(args.dirs or []),frCliHasDefaults=str(int(default_dirs is not None)),frCliDefaults=list(default_dirs or [])))
        root=Path(one(rows,'FR_CLI_ROOT','path')).resolve()
        arguments=[r['parameters']['path'] for r in sorted((r for r in rows if r['code']=='FR_CLI_ARG'),key=lambda r:r['parameters']['ordinal'])]
        rows=backend.native(CHECK,'_fr_cli_dirs',[],dict(frCliRawDirs=arguments),root=root)
        if any(r['code']!='FR_CLI_DIR' for r in rows):raise ValueError('Unexpected ForwardRef CLI directory')
        return root,[r['parameters']['path'] for r in sorted(rows,key=lambda r:tuple(r['parameters'][k] for k in ['argument','rank','path']))]
    def cli():
        schema=json.loads(config['cli_schema']);parser=argparse.ArgumentParser(description=schema['description'])
        for item in schema['arguments']:
            kwargs=dict(item['kwargs'])
            if 'type' in kwargs:kwargs['type']={'str':str}[kwargs['type']]
            parser.add_argument(item['name'],**kwargs)
        args=parser.parse_args();root,directories=module.resolve_root_and_dirs(args,default_dirs=['public'])
        rows=backend.native(CHECK,'_fr_cli_paths',[],dict(frCliPaths=list(args.paths or []),frCliDefaultPaths=[str(root/d) for d in directories]))
        candidates=[];kinds=[]
        for p in sorted((r['parameters'] for r in rows),key=lambda p:p['ordinal']):
            path=Path(p['path'])
            if path.is_file():candidates.append(path);kinds.append('direct')
            elif path.is_dir():
                for child in path.rglob('*.py'):candidates.append(child);kinds.append('walk')
            else:candidates.append(path);kinds.append('missing')
        rows=backend.native(CHECK,'_fr_cli_select',[],dict(frCliCandidates=[str(p) for p in candidates],frCliKinds=kinds,frCliExclude=json.loads(config['cli_exclude'])))
        if any(r['code'] not in {'FR_CLI_FILE','FR_CLI_WARNING'} for r in rows):raise ValueError('Unexpected ForwardRef CLI selection')
        for p in sorted((r['parameters'] for r in rows if r['code']=='FR_CLI_WARNING'),key=lambda p:p['ordinal']):sys.stderr.write(p['text'])
        paths=tuple(Path(r['parameters']['path']) for r in sorted((r for r in rows if r['code']=='FR_CLI_FILE'),key=lambda r:r['parameters']['ordinal']))
        values=module._issues_for_files(root,paths)
        items=[dict(file=str(v.file),line=v.line,col=v.col,message=v.message,code=v.code) for v in values]
        rows=backend.native(CHECK,'_fr_cli_report',[dict(path='forwardref-cli.json',source=json.dumps(dict(items=items),ensure_ascii=False))])
        if any(r['code'] not in {'FR_CLI_OUTPUT','FR_CLI_EXIT'} for r in rows):raise ValueError('Unexpected ForwardRef CLI report')
        for p in sorted((r['parameters'] for r in rows if r['code']=='FR_CLI_OUTPUT'),key=lambda p:tuple(p[k] for k in ['phase','path_key','sign','length','digits','ordinal'])):sys.stdout.write(p['text'])
        return one(rows,'FR_CLI_EXIT','status')
    backend.replace(module,'resolve_root_and_dirs',resolve);backend.replace(module,'cli_main',cli)
