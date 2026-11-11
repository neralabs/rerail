"""Transport State Tripwires source arguments and native finding DTOs."""
from pathlib import Path

CHECK = 'serverplan_state_tripwires'


def sources(arguments, entry):
    if entry == 'scan_project_sources':
        return list(arguments['file_sources'])
    if 'source' not in arguments or arguments['source'] is None:
        raise ValueError('State Tripwires supplied AST transport needs its original source text')
    return [(arguments['path'], arguments['source'])]


def logical(path, root):
    path = Path(path)
    if path.is_absolute():
        path = path.resolve()
        return (path.relative_to(root) if path.is_relative_to(root) else path.relative_to('/')).as_posix()
    return path.as_posix()


def snapshot(module, arguments, entry):
    pairs = sources(arguments, entry)
    module_root = Path(module.ROOT).resolve()
    # The lossless source recorder retains the module root for a project scan.
    root = module_root if entry == 'scan_project_sources' else None
    if root is None and Path(pairs[0][0]).is_absolute() and Path(pairs[0][0]).resolve().is_relative_to(module_root):
        root = module_root
    files = [dict(path=logical(path, module_root), source=text) for path,text in pairs]
    return files, dict(include=['**/*.py'], exclude=[]), root


def scan(backend, module, arguments, entry):
    pairs = sources(arguments, entry)
    files, _, _ = snapshot(module, arguments, entry)
    supplied = arguments.get('project_return_kinds')
    options = {}
    if supplied is not None:
        items = [(name, kind) for name, kinds in supplied.items() for kind in sorted(kinds)]
        options.update(returnKindNames=[name for name,_ in items],
                       returnKindValues=[kind for _,kind in items])
    rows = backend.native(CHECK, entry, files, options)
    order = {file['path']:i for i,file in enumerate(files)}
    paths = {file['path']:Path(path) for file,(path,_) in zip(files,pairs)}
    def ordering(row):
        p=row['parameters']
        if p['mode']=='raw':
            return order[row['path']],p['visit'],p['phase'],p['ordinal']
        if p['mode']!='sorted':
            raise ValueError('Invalid native State Tripwires result order')
        return order[row['path']],row['line'],row['code'],p['qualname'],row['message']
    rows = sorted(rows,key=ordering)
    return [module.Violation(row['code'],paths[row['path']],row['line'],
                             row['parameters']['qualname'],row['message']) for row in rows]


def install(backend, module):
    def native(entry,options=None):
        return backend.native(CHECK,entry,[],options or {})

    def trigger(source):
        rows=native('_source_may_have_state_tripwire_hit',dict(inputText=source))
        if len(rows)>1 or any(r['code']!='SPT_TRIGGER' for r in rows):
            raise ValueError('Invalid native State Tripwires trigger')
        return bool(rows)

    def load_generated():
        result={}
        for row in native('_load_generated_return_kinds'):
            if row['code']!='SPT_RETURN_KIND':
                raise ValueError('Invalid native generated return category')
            p=row['parameters'];result.setdefault(p['name'],set()).add(p['kind'])
        return {name:frozenset(kinds) for name,kinds in sorted(result.items())}

    def filter_signed(violations):
        rows=native('_filter_signed_python_work_allowlist',dict(moduleRoot=module.ROOT.as_posix(),
            filterPaths=[v.file.as_posix() for v in violations],filterCodes=[v.code for v in violations],
            filterQualnames=[v.qualname for v in violations]))
        if any(r['code']!='SPT_FILTER' for r in rows):
            raise ValueError('Invalid native signed-work filter result')
        return [violations[r['parameters']['ordinal']] for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]

    class NativeCollector(module.ServerPlanStateTripwireCollector):
        def __init__(self,path,source):
            self.path=path;self.source=source;self.violations=[]

        def on_Module(self,node,_context):
            if self.generated_return_kinds is None:
                type(self).generated_return_kinds=load_generated()
            self.violations=module.scan_source(self.path,self.source,project_return_kinds=self.generated_return_kinds,
                project_semantic_runtime_builder_names=frozenset(),project_semantic_runtime_builder_helper_names=frozenset(),tree=node)

        def finish_file(self):
            return [dict(code=v.code,file=v.file.as_posix(),line=v.line,qualname=v.qualname,message=v.message)
                    for v in self.violations]

    backend.replace(module,'_source_may_have_state_tripwire_hit',trigger)
    backend.replace(module,'_load_generated_return_kinds',load_generated)
    backend.replace(module,'_filter_signed_python_work_allowlist',filter_signed)
    backend.replace(module,'ServerPlanStateTripwireCollector',NativeCollector)
