"""Transport callback collector arguments and complete raw facts without detector decisions."""
import ast,json
from pathlib import Path
from transaction_contract import syntax


def ast_input(tree):
    options=syntax(tree);pending=[(tree,-1,False)];nodes=[];exit_order={};identities={};identity=[]
    while pending:
        node,index,leaving=pending.pop()
        if leaving:exit_order[index]=len(exit_order);continue
        index=len(nodes);nodes.append(node);identity.append(str(identities.setdefault(id(node),index)))
        pending.append((node,index,True));pending.extend((c,-1,False) for c in reversed(list(ast.iter_child_nodes(node))))
    options['astIdentity']=identity;options['astExitOrder']=[str(exit_order[i]) for i in range(len(nodes))]
    options['astStringJson']=[json.dumps(n.value,ensure_ascii=True) if isinstance(n,ast.Constant) and isinstance(n.value,str) else '' for n in nodes]
    annotations=[(i,ast.unparse(n)) for i,n in enumerate(nodes) if options['astFields'][i]=='annotation']
    transport=('def _annotation_transport('+', '.join(f'e{i}: {text}' for i,text in annotations)+'):\n    pass\n') if annotations else 'pass\n'
    return options,transport


def relative_path(module,path):
    """The caller's filesystem spelling relative to the linter's own repository root."""
    try:return path.resolve().relative_to(module.REPO_ROOT).as_posix()
    except ValueError:return path.as_posix()


def ordered(rows):
    return sorted(rows,key=lambda r:(r['path'],r['parameters']['exit_order'],-r['parameters']['entry_order'],r['parameters']['reason']))


def make_collector(suite,module,path,source):
    from scripts.lint_lib.ast_collectors import AstEventCollector
    class Collector(AstEventCollector):
        def __init__(self):self.tree=None
        def on_node(self,node,context):
            if self.tree is None:
                root=node
                while context.parent(root) is not None:root=context.parent(root)
                self.tree=root
        def finish_file(self):
            options,transport=ast_input(self.tree or ast.Module(body=[],type_ignores=[]))
            options.update(originalPath=str(path),relativePath=relative_path(module,path),providedSource=source)
            rows=suite.native('transaction_callback_wrappers','collector',[dict(path='_annotations.py',source=transport)],options)
            return [dict(line=r['line'],qualname=r['parameters']['qualname'],message=r['message']) for r in ordered(rows)]
    return Collector()


def collector_result(module,path,source,tree,suite=None,check_inline=None):
    """Deliver a supplied tree through the unchanged event protocol."""
    from scripts.lint_lib.ast_collectors import AstTraversalContext
    if suite is not None:
        options,transport=ast_input(tree)
        options.update(originalPath=str(path),relativePath=relative_path(module,path),providedSource=source)
        if check_inline is not None:options['inlineOverride']=str(check_inline).lower()
        rows=suite.native('transaction_callback_wrappers','collector',[dict(path='_annotations.py',source=transport)],options)
        return [dict(line=r['line'],qualname=r['parameters']['qualname'],message=r['message']) for r in ordered(rows)]
    collector=(module.TransactionCallbackWrapperCollector(path,check_inline_reentrant_callbacks=check_inline) if check_inline is not None else
        module.transaction_callback_wrapper_fact_extractors(Path.cwd())[module.TRANSACTION_CALLBACK_WRAPPER_EXTRACTOR.extractor_id](path,source))
    parents={tree:None};children={};pending=[tree]
    while pending:
        n=pending.pop();children[n]=tuple(ast.iter_child_nodes(n))
        for c in children[n]:parents[c]=n
        pending.extend(reversed(children[n]))
    context=AstTraversalContext(path,source,parents,children);collector.start_file(path,source,context);pending=[(tree,False)]
    while pending:
        n,leaving=pending.pop();kind=type(n).__name__
        if leaving:
            collector.on_exit_node(n,context);handler=getattr(collector,'on_exit_'+kind,None)
            if handler:handler(n,context)
        else:
            collector.on_node(n,context);handler=getattr(collector,'on_'+kind,None)
            if handler:handler(n,context)
            pending.append((n,True));pending.extend((c,False) for c in reversed(children[n]))
    return collector.finish_file()


def scan(suite,entry,module,values):
    paths=[Path(p) for p in values['files']] if entry=='_violations_for_files' else [Path(values['file_path'])]
    root=Path(values.get('root',module.REPO_ROOT));provided=None
    if entry=='scan_source':provided=values['source']
    if entry=='scan_file':
        try:provided=paths[0].read_text(encoding='utf-8')
        except (OSError,UnicodeDecodeError):provided=''
    files=[];original={};relatives=[];scopes=[]
    for i,path in enumerate(paths):
        actual=path if path.is_absolute() else root/path
        try:text=actual.read_text(encoding='utf-8')
        except (OSError,UnicodeDecodeError):continue
        logical=f'input-{i}/'+path.name;original[logical]=actual
        files.append(dict(path=logical,source=text))
        relatives.append(relative_path(module,actual))
        scopes.append(actual.relative_to(root).as_posix() if actual.is_relative_to(root) else actual.as_posix())
    options=dict(repositoryRoot=str(module.REPO_ROOT),documentPaths=[f['path'] for f in files],relativePaths=relatives,scopePaths=scopes)
    if provided is not None:options['providedSource']=provided
    rows=suite.native('transaction_callback_wrappers',entry,files,options)
    return [module.Violation(file_path=original[r['path']],line=r['line'],qualname=r['parameters']['qualname'],message=r['message']) for r in ordered(rows)]


def install(suite,module):
    def factory(root):
        return {module.TRANSACTION_CALLBACK_WRAPPER_EXTRACTOR.extractor_id:lambda path,source:make_collector(suite,module,path,source)}
    suite.replace(module,'transaction_callback_wrapper_fact_extractors',factory)
