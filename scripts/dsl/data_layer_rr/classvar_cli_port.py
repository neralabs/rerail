"""Transport ClassVar CLI arguments and callbacks; select and render in .rr."""
import argparse
import json
import os
from pathlib import Path
import sys

CHECK='classvar_check'


def install(backend,module,config):
    discovery_source=Path(module.resolve_root_and_dirs.__globals__['__file__']).absolute()
    schema=json.loads(config['cli_schema'])
    def resolve(args,*,default_dirs=None):
        rows=backend.native(CHECK,'_classvar_cli_root',[],dict(cvHasRoot=str(int(args.root is not None)),cvRoot=args.root or '',cvDiscoverySource=str(discovery_source),cvHasDirs=str(int(args.dirs is not None)),cvDirs=list(args.dirs or []),cvHasDefaults=str(int(default_dirs is not None)),cvDefaults=list(default_dirs or [])))
        if any(r['code'] not in {'CV_ROOT','CV_DIR_ARG'} for r in rows):raise ValueError('Invalid native ClassVar root response')
        roots=[r['parameters']['path'] for r in rows if r['code']=='CV_ROOT']
        if len(roots)!=1:raise ValueError('Missing native ClassVar root')
        root=Path(roots[0]).resolve()
        directories=[r['parameters']['path'] for r in sorted((r for r in rows if r['code']=='CV_DIR_ARG'),key=lambda r:r['parameters']['ordinal'])]
        exports=(['cv.cli.dirs=1'] if directories else [])+[f'data-layer.scan.{i}={p}' for i,p in enumerate(directories)]
        rows=backend.native(CHECK,'_classvar_cli_dirs',[],dict(env=exports),root=root)
        if any(r['code']!='CV_DIRECTORY' for r in rows):raise ValueError('Invalid native ClassVar directories')
        return root,[r['parameters']['path'] for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]
    def cli():
        parser=argparse.ArgumentParser(description=schema['description'])
        for item in schema['arguments']:
            kwargs=dict(item['kwargs'])
            if 'type' in kwargs:kwargs['type']={'str':str}[kwargs['type']]
            parser.add_argument(item['name'],**kwargs)
        args=parser.parse_args();root,directories=module.resolve_root_and_dirs(args,default_dirs=['public'])
        rows=backend.native(CHECK,'_classvar_cli_paths',[],dict(cvRequestedPaths=list(args.paths or []),cvDefaultPaths=[str(root/d) for d in directories]))
        if any(r['code']!='CV_CLI_PATH' for r in rows):raise ValueError('Invalid native ClassVar CLI paths')
        paths=[];kinds=[]
        for r in sorted(rows,key=lambda r:r['parameters']['ordinal']):
            path=Path(r['parameters']['path'])
            if path.is_file():paths.append(str(path));kinds.append('direct')
            elif path.is_dir():
                for directory,_names,filenames in os.walk(path):
                    for name in filenames:paths.append(str(Path(directory)/name));kinds.append('walk')
            else:paths.append(str(path));kinds.append('missing')
        rows=backend.native(CHECK,'_classvar_cli_prepare',[],dict(cvCliPaths=paths,cvCliKinds=kinds))
        if any(r['code'] not in {'CV_CLI_FILE','CV_CLI_WARNING'} for r in rows):raise ValueError('Invalid native ClassVar CLI selection')
        for p in sorted((r['parameters'] for r in rows if r['code']=='CV_CLI_WARNING'),key=lambda p:p['ordinal']):sys.stderr.write(p['text'])
        files=tuple(Path(p['path']) for p in sorted((r['parameters'] for r in rows if r['code']=='CV_CLI_FILE'),key=lambda p:p['ordinal']))
        values=module._issues_for_files(root,files)
        options={key:[str(v[i]) for v in values] for i,key in enumerate(['cvCliFiles','cvCliLines','cvCliColumns','cvCliMessages','cvCliCodes'])}
        rows=backend.native(CHECK,'_classvar_cli_report',[],dict(options,cvCliCheck=str(int(args.check))))
        if any(r['code'] not in {'CV_CLI_OUTPUT','CV_CLI_EXIT'} for r in rows):raise ValueError('Invalid native ClassVar CLI result')
        for p in sorted((r['parameters'] for r in rows if r['code']=='CV_CLI_OUTPUT'),key=lambda p:(p['phase'],p['ordinal'])):sys.stdout.write(p['text'])
        statuses=[r['parameters']['status'] for r in rows if r['code']=='CV_CLI_EXIT']
        if len(statuses)!=1:raise ValueError('Missing native ClassVar CLI status')
        return statuses[0]
    backend.replace(module,'resolve_root_and_dirs',resolve)
    backend.replace(module,'cli_main',cli)
