"""Filesystem, supplied-syntax and passive DTO transport for transaction closures."""
import os
from pathlib import Path
from transaction_contract import syntax


def ordered(rows):
    return sorted(rows,key=lambda r:(r['path'],r['parameters'].get('call',-1),r['parameters'].get('ordinal',-1),r['parameters'].get('rank',0)))


def violations(module,rows,paths):
    return [module.LintViolation(file=paths.get(r['path'],Path(r['path'])),line=r['line'],
        col=r['parameters']['column'],message=r['message'],severity='error') for r in ordered(rows)
        if r['code'] not in {'TX_LOCAL','TX_SCOPE'}]


def graph(suite,entry,tree,options=None):
    values=syntax(tree);values.update(options or {});values.setdefault('originalPath','input.py')
    return suite.native('transaction_closures',entry,[],values)


def scan(suite,module,entry,values):
    if entry=='lint_source':
        tree=values.get('tree');path=Path(values['file_path'])
        if tree is None:
            rows=suite.native('transaction_closures','source_missing',[],dict(providedSource=values['source']))
            if rows:raise ValueError(rows[0]['message'])
            return []
        rows=graph(suite,'source',tree,dict(originalPath=str(path),providedSource=values['source']))
        return violations(module,rows,{str(path):path})
    if entry=='lint_file':
        path=Path(values['file_path'])
        try:source=path.read_text(encoding='utf-8');error=''
        except Exception as e:source='';error=str(e)
        rows=suite.native('transaction_closures','lint_file',[dict(path=path.name,source=source)],dict(readError=error))
        return violations(module,rows,{path.name:path if error or path.is_absolute() else path.resolve()})
    candidates=[];kinds=[];traversal=[]
    for item in values['paths']:
        path=Path(item)
        if path.is_file():candidates.append(path);kinds.append('direct');traversal.append(path.name)
        elif path.is_dir():
            for current,dirs,files in os.walk(path):
                dirs.sort();files.sort()
                for name in files:
                    file=Path(current)/name;candidates.append(file);kinds.append('nested');traversal.append(file.relative_to(path.parent).as_posix())
    options=dict(inventoryPath=[str(p) for p in candidates],inventoryKind=kinds,inventoryTraversal=traversal,
        excludeTests=str(values.get('exclude_tests',False)).lower())
    selected=suite.native('transaction_closures','selection',[],options)
    paths=sorted({candidates[r['parameters']['index']].absolute() for r in selected})
    root=Path(os.path.commonpath([p.parent for p in paths])) if paths else Path.cwd()
    files=[];original={};errors=[]
    for path in paths:
        logical=path.relative_to(root).as_posix();original[logical]=path
        try:source=path.read_text(encoding='utf-8')
        except Exception as e:source='';errors.append((logical,type(e).__name__+': '+str(e)))
        files.append(dict(path=logical,source=source))
    rows=suite.native('transaction_closures','lint_paths',files,dict(errorPaths=[p for p,e in errors],errorMessages=[e for p,e in errors]))
    return violations(module,rows,original)


def install(suite,module):
    """Replace semantic APIs; list/set writes below are caller-visible storage operations."""
    constants=suite.native('transaction_closures','constants',[])
    if any(r['code']!='TX_CONSTANT' for r in constants):raise AssertionError('Unexpected native transaction constant output')
    bindings=[]
    for table,name in [('transaction','TRANSACTION_METHODS'),('mutating','MUTATING_METHODS')]:
        target=getattr(module,name);values={r['parameters']['value'] for r in constants if r['parameters']['table']==table}
        bindings.append((target,set(target),values))
    def apply_constants():
        # Collection retains the complete original parametrization. Each assertion sees native values,
        # including aliases imported by its unchanged module during collection.
        for target,_,values in bindings:target.clear();target.update(values)
    def restore_constants():
        for target,original,_ in bindings:target.clear();target.update(original)
    suite.apply_transaction_constants=apply_constants;suite.restore_transaction_constants=restore_constants
    def detector_init(instance,outer_scope_vars):
        instance.outer_scope_vars=outer_scope_vars;instance.local_vars=set();instance.violations=[];instance.in_nested_function=False
    def detector_visit(instance,node):
        rows=graph(suite,'detector',node,dict(outerVars=sorted(instance.outer_scope_vars),initialLocals=sorted(instance.local_vars)))
        instance.local_vars.update(r['parameters']['name'] for r in rows if r['code']=='TX_LOCAL')
        instance.violations.extend((r['line'],r['parameters']['column'],r['message']) for r in ordered(rows) if r['code']!='TX_LOCAL')
    def visitor_init(instance,file_path):
        instance.file_path=file_path;instance.violations=[]
        if isinstance(instance,module.TransactionClosureLinter):instance.scope_stack=[set()]
        else:instance.current_scope_functions={}
    def visitor_visit(instance,tree):
        inline=isinstance(instance,module.TransactionClosureLinter)
        rows=graph(suite,'visitor',tree,dict(originalPath=str(instance.file_path),checkInline=str(inline).lower(),checkNamed=str(not inline).lower()))
        instance.violations=violations(module,rows,{str(instance.file_path):instance.file_path})
        if inline:instance.scope_stack=[{r['parameters']['name'] for r in rows if r['code']=='TX_SCOPE'}]
    suite.replace(module.NonLocalStateDetector,'__init__',detector_init)
    suite.replace(module.NonLocalStateDetector,'visit',detector_visit)
    for cls in [module.TransactionClosureLinter,module.ClosureDefinitionFinder]:
        suite.replace(cls,'__init__',visitor_init);suite.replace(cls,'visit',visitor_visit)
    cls=module.TransactionClosureLinter
    suite.replace(cls,'push_scope',lambda instance:instance.scope_stack.append(set()))
    def pop(instance):
        if len(instance.scope_stack)>1:instance.scope_stack.pop()
    suite.replace(cls,'pop_scope',pop)
    suite.replace(cls,'add_to_scope',lambda instance,name:instance.scope_stack[-1].add(name))
    def current(instance):
        rows=suite.native('transaction_closures','scope',[],dict(scopeNames=[n for s in instance.scope_stack for n in s]))
        return {r['parameters']['name'] for r in rows}
    suite.replace(cls,'current_scope_vars',current)
    suite.replace(module.LintViolation,'__str__',lambda v:f'{v.file}:{v.line}:{v.col}: {v.severity}: {v.message}')
    for entry in ['lint_source','lint_file','lint_paths']:
        import inspect
        original=getattr(module,entry);signature=inspect.signature(original)
        def wrap(entry,signature):
            def run(*args,**kwargs):return scan(suite,module,entry,dict(signature.bind(*args,**kwargs).arguments))
            return run
        suite.replace(module,entry,wrap(entry,signature))
