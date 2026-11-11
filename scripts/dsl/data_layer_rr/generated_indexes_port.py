"""Transport generated-index API arguments and complete rendered findings."""
from pathlib import Path
import inspect

CHECK='generated_index_sources'


def install(suite,module,detectors=False):
    if not detectors:return
    for name in ['check_file','check_source','check_repo']:
        signature=inspect.signature(getattr(module,name))
        def bind(entry,signature):
            def run(*args,**kwargs):
                values=signature.bind(*args,**kwargs);values.apply_defaults()
                return scan(suite,entry,module,values.arguments)
            return run
        suite.replace(module,name,bind(name,signature))


def scan(suite, entry, module, args):
    paths=([Path(args['root'])/p for p in module.GENERATOR_FILES]
           if entry=='check_repo' else [Path(args['path'])])
    files=[];originals={}
    for index,path in enumerate(paths):
        try:source=path.read_text(encoding='utf-8')
        except FileNotFoundError:continue
        logical=f'input/{index}/{path.name}'
        files.append(dict(path=logical,source=source));originals[logical]=path
    rows=suite.native(CHECK,entry,files,dict(apiInputDirs=['input'],apiBuffers=list(originals),
        apiSourcePaths=[str(p) for p in originals.values()]))
    if any(r['code']!='missing_source' for r in rows):raise AssertionError('Unexpected generated-index diagnostic')
    order={f['path']:i for i,f in enumerate(files)}
    return [f"{originals[r['path']]}:{r['line']}: {r['message']}"
        for r in sorted(rows,key=lambda r:(order[r['path']],r['parameters']['ordinal'],r['parameters']['rank']))]


def normalize(values, root):
    rows=[]
    for value in values:
        head,separator,message=value.partition(': ')
        path,colon,line=head.rpartition(':')
        if not separator or not colon:raise ValueError('Malformed generated-index finding')
        path=Path(path)
        if path.is_absolute():path=path.relative_to(root)
        rows.append(dict(path=path.as_posix(),line=int(line),column=None,code='missing_source',message=message,severity=None))
    return rows
