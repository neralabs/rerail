"""Transport type-safety CLI arguments and observations to native decisions."""
import argparse
import json
from pathlib import Path
import sys

CHECK='type_safety_patterns'


def one(rows,code,key):
    values=[r['parameters'][key] for r in rows if r['code']==code]
    if len(values)!=1:raise ValueError('One native '+code+' required')
    return values[0]


def install(backend,module,config,native):
    discovery_globals=module.resolve_root_and_dirs.__globals__
    def resolve(args,*,default_dirs=None):
        rows=native('_tsp_cli_root',[],dict(tspCliHasRoot=str(int(args.root is not None)),tspCliRoot=args.root or '',tspDiscoverySource=str(Path(discovery_globals['__file__']).resolve())))
        root=Path(one(rows,'TSP_CLI_ROOT','path')).resolve()
        rows=native('_tsp_cli_dirs',[],dict(tspCliHasDirs=str(int(args.dirs is not None)),tspCliDirs=list(args.dirs or []),tspCliHasDefaults=str(int(default_dirs is not None)),tspCliDefaults=list(default_dirs or [])),root=root)
        if any(r['code']!='TSP_CLI_DIR' for r in rows):raise ValueError('Unexpected type-safety CLI directory')
        return root,[r['parameters']['path'] for r in sorted(rows,key=lambda r:tuple(r['parameters'][k] for k in ['argument','rank','path']))]
    def cli():
        schema=json.loads(config['cli_schema']);parser=argparse.ArgumentParser(description=schema['description'])
        for item in schema['arguments']:
            kwargs=dict(item['kwargs'])
            if 'type' in kwargs:kwargs['type']={'str':str}[kwargs['type']]
            parser.add_argument(*item['names'],**kwargs)
        args=parser.parse_args();root,directories=module.resolve_root_and_dirs(args)
        paths=[]
        for name in directories:
            directory=root/name
            if directory.exists():paths.extend(module.find_python_files(directory))
        values=module._issues_for_files(root,tuple(paths))
        items=[dict(category=v.category.name,file_path=str(v.file_path),line_number=v.line_number,column=v.column,code_snippet=v.code_snippet,suggestion=v.suggestion) for v in values]
        rows=native('_tsp_cli_report',[dict(path='type-safety-cli.json',source=json.dumps(dict(items=items),ensure_ascii=False))],dict(tspReportRoot=str(root),tspSummary=str(int(args.summary)),tspCheck=str(int(args.check))))
        if any(r['code'] not in {'TSP_CLI_OUTPUT','TSP_CLI_EXIT'} for r in rows):raise ValueError('Unexpected type-safety CLI report')
        for p in sorted((r['parameters'] for r in rows if r['code']=='TSP_CLI_OUTPUT'),key=lambda p:tuple(p[k] for k in ['phase','category','part','path_key','sign','length','digits','ordinal'])):sys.stdout.write(p['text'])
        return one(rows,'TSP_CLI_EXIT','status')
    backend.replace(module,'resolve_root_and_dirs',resolve);backend.replace(module,'cli_main',cli)
