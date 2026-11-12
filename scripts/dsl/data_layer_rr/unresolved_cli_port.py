"""Transport CLI values and file batches; native relations select and render findings."""
import argparse
import json
from pathlib import Path
import sys

CHECK='unresolved_forward_refs'


def one(rows,code):
    values=[r['parameters'] for r in rows if r['code']==code]
    if len(values)!=1:raise ValueError('One native '+code+' required')
    return values[0]


def batch(backend,module,paths):
    from unresolved_port import scan
    names={};files=[]
    for i,path in enumerate(paths):
        try:source=path.read_text(encoding='utf-8')
        except UnicodeDecodeError:source=''
        except OSError:
            scan(backend,'analyze_file',module,dict(file_path=path))
            raise AssertionError('Native unresolved I/O error unexpectedly succeeded')
        key=f'impl/input-{i}.py';names[key]=path;files.append(dict(path=key,source=source))
    rows=backend.native(CHECK,'cli_scan',files,dict(logicalFiles=list(names),fileSuffixes=[p.suffix for p in paths],relativeFiles=[key for key,p in names.items() if not p.is_absolute()]))
    if any(r['code']!='unresolved_forward_ref' for r in rows):raise ValueError('Unexpected unresolved CLI scan response')
    grouped={key:[] for key in names}
    for row in rows:grouped[row['path']].append(row)
    values=[]
    for key,path in names.items():
        found=[dict(file=str(path),line=r['line'],function=r['parameters']['function'],problem=r['parameters']['problem'],source=r['parameters']['source']) for r in sorted(grouped[key],key=lambda r:r['parameters']['ordinal'])]
        if hasattr(backend,'compare_original'):backend.compare_original(CHECK,'analyze_file',module,(path,),{},found)
        values.extend(found)
    return values


def install(backend,module,config):
    discovery_globals=module.resolve_root_and_dirs.__globals__
    def resolve(args,*,default_dirs=None):
        rows=backend.native(CHECK,'_uf_cli_root',[],dict(ufCliHasRoot=str(int(args.root is not None)),ufCliRoot=args.root or '',ufDiscoverySource=str(Path(discovery_globals['__file__']).resolve()),ufCliHasDirs=str(int(args.dirs is not None)),ufCliDirs=list(args.dirs or []),ufCliHasDefaults=str(int(default_dirs is not None)),ufCliDefaults=list(default_dirs or [])))
        root=Path(one(rows,'UF_CLI_ROOT')['path']).resolve()
        arguments=[r['parameters']['path'] for r in sorted((r for r in rows if r['code']=='UF_CLI_ARG'),key=lambda r:r['parameters']['ordinal'])]
        rows=backend.native(CHECK,'_uf_cli_dirs',[],dict(env=['unresolved.paths=true',*[f'data-layer.scan.{i}={p}' for i,p in enumerate(arguments)]]),root=root)
        if any(r['code']!='UF_CLI_DIR' for r in rows):raise ValueError('Unexpected unresolved CLI directory')
        return root,[r['parameters']['path'] for r in sorted(rows,key=lambda r:tuple(r['parameters'][k] for k in ['argument','rank','path']))]
    def cli():
        schema=config['cli_schema'];parser=argparse.ArgumentParser(description=schema['description'],epilog=schema['epilog'],formatter_class=argparse.RawDescriptionHelpFormatter)
        for item in schema['arguments']:
            kwargs=dict(item['kwargs'])
            if 'type' in kwargs:kwargs['type']={'str':str}[kwargs['type']]
            parser.add_argument(*item['names'],**kwargs)
        args=parser.parse_args();root,dirs=module.resolve_root_and_dirs(args,default_dirs=['api','impl'])
        rows=backend.native(CHECK,'_uf_cli_paths',[],dict(ufCliPaths=list(args.paths or []),ufCliDefaultPaths=[str(root/d) for d in dirs]))
        if any(r['code']!='UF_CLI_PATH' for r in rows):raise ValueError('Unexpected unresolved CLI path')
        candidates=[];modes=[];relative=[]
        for row in sorted(rows,key=lambda r:r['parameters']['ordinal']):
            path=Path(row['parameters']['path'])
            if not path.exists():continue
            if path.is_file():candidates.append(path);modes.append('direct');relative.append(path.name)
            else:
                for child in sorted(path.rglob('*')):
                    candidates.append(child);modes.append('recursive');relative.append(child.relative_to(path).as_posix())
        rows=backend.native(CHECK,'cli_selection',[],dict(cliNames=[p.name for p in candidates],cliModes=modes,cliRelativePaths=relative))
        if any(r['code']!='UF_CLI_FILE' for r in rows):raise ValueError('Unexpected unresolved CLI file')
        paths=[candidates[r['parameters']['ordinal']] for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]
        if getattr(module.analyze_file,'_rerail_unresolved_file',False):values=batch(backend,module,paths)
        else:
            values=[]
            for path in paths:values.extend(module.analyze_file(path))
        rows=backend.native(CHECK,'_uf_cli_report',[dict(path='unresolved-cli.json',source=json.dumps(dict(items=values),ensure_ascii=False))],dict(ufCliReportRoot=str(root),ufCliCheck=str(int(args.check)),ufCliVerbose=str(int(args.verbose))))
        if any(r['code'] not in {'UF_CLI_OUTPUT','UF_CLI_EXIT'} for r in rows):raise ValueError('Unexpected unresolved CLI report')
        for p in sorted((r['parameters'] for r in rows if r['code']=='UF_CLI_OUTPUT'),key=lambda p:tuple(p[k] for k in ['phase','path_key','sign','length','digits','ordinal','sub'])):sys.stdout.write(p['text'])
        control=one(rows,'UF_CLI_EXIT')
        if control['raised']:raise SystemExit(control['status'])
        return None
    backend.replace(module,'resolve_root_and_dirs',resolve);backend.replace(module,'cli_main',cli)
