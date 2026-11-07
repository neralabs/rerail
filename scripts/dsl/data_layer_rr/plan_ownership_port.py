"""Input, filesystem and passive DTO transport for native plan-scope ownership."""
import ast,inspect
from pathlib import Path
from ast_transport import syntax_columns


def snapshot(module, entry, args, kwargs):
    values=inspect.signature(getattr(module,entry)).bind(*args,**kwargs).arguments
    root=Path(values.get('root') or module.ROOT)
    if entry=='lint':
        paths=list(root.rglob('*.py'))
    else:
        paths=[Path(values['path'])]
    files=[]
    for path in paths:
        try:source=values['source'] if 'source' in values else path.read_text(encoding='utf-8')
        except (OSError,UnicodeError):continue
        logical=path.relative_to(root).as_posix() if path.is_relative_to(root) else path.as_posix().lstrip('/')
        files.append(dict(path=logical,source=source))
    return files,dict(include=['**/*.py'],exclude=[]),root


def ast_input(tree):
    options=syntax_columns(tree);options.pop('receiverTransport',None)
    annotations=[];pending=[tree];index=0
    while pending:
        node=pending.pop()
        if isinstance(node,ast.expr):annotations.append((index,ast.unparse(node)))
        pending.extend(reversed(list(ast.iter_child_nodes(node))));index+=1
    transport=('def _pso_transport('+', '.join(f'e{i}: {text}' for i,text in annotations)+'):\n    pass\n') if annotations else 'pass\n'
    return options,transport


def install(suite,module):
    from scripts.lint_lib.ast_collectors import AstEventCollector
    class Collector(AstEventCollector):
        def __init__(self,root,path,source):
            self.root,self.path,self.source=root,path,source
            self.tree=None
        def on_node(self,node,context):
            if self.tree is None:self.tree=node
        def finish_file(self):
            options,transport=ast_input(self.tree or ast.Module(body=[],type_ignores=[]))
            options.update(providedSource=self.source,originalPath=str(self.path),relativePath=self.path.relative_to(self.root).as_posix())
            rows=suite.native('plan_scope_ownership','collector',[dict(path='_collector.py',source=transport)],options)
            return [dict(code=r['code'],file_path=r['path'],line=r['line'],message=r['message'],severity='error') for r in ordered(rows)]
    def factory(root):
        return {module.PLAN_SCOPE_OWNERSHIP_FACT.extractor_id:lambda path,source:Collector(root,path,source)}
    suite.replace(module,'plan_scope_ownership_fact_extractors',factory)


def ordered(rows):
    return sorted(rows,key=lambda r:(r['path'],r['line'],r['code'],r['parameters']['event'],r['parameters']['rank'],r['parameters']['witness']))


def scan(suite,entry,module,values):
    if entry=='lint':
        root=Path(values.get('root') or module.ROOT)
        paths=list(root.rglob('*.py'))
        options=dict(availablePaths=[p.relative_to(root).as_posix() for p in paths],
            topDirectories=sorted(p.name for p in root.iterdir() if p.is_dir()),scanDirectories=list(module.SCAN_DIRECTORIES),
            skipDirectories=sorted(module.SKIP_DIR_NAMES))
        selected=suite.native('plan_scope_ownership','selection',[],options)
        if any(r['code']!='PSO_FILE' for r in selected):raise AssertionError('Invalid native ownership inventory')
        files=[dict(path=r['parameters']['path'],source=paths[r['parameters']['index']].read_text(encoding='utf-8'))
            for r in sorted(selected,key=lambda r:(r['parameters']['root_rank'],r['parameters']['index']))]
        rows=suite.native('plan_scope_ownership',entry,files)
    else:
        root=Path(values['root']);path=Path(values['path'])
        source=values['source'] if entry=='_source_facts' else path.read_text(encoding='utf-8')
        logical=path.relative_to(root).as_posix() if path.is_relative_to(root) else path.as_posix().lstrip('/')
        rows=suite.native('plan_scope_ownership',entry,[dict(path=logical,source=source)])
    if any(r['code'] not in {f'PSO{i:03}' for i in range(14)} for r in rows):raise AssertionError('Invalid ownership diagnostic')
    if entry=='_source_facts':
        return [dict(code=r['code'],file_path=r['path'],line=r['line'],message=r['message'],severity='error') for r in ordered(rows)]
    return [module.Violation(code=r['code'],file_path=r['path'],line_number=r['line'],message=r['message'],severity='error') for r in ordered(rows)]
