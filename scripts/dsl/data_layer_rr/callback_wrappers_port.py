"""Transport callback collector arguments and complete raw facts without detector decisions."""
import ast
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


def install(suite,module):
    def factory(root):
        return {module.TRANSACTION_CALLBACK_WRAPPER_EXTRACTOR.extractor_id:lambda path,source:make_collector(suite,module,path,source)}
    suite.replace(module,'transaction_callback_wrapper_fact_extractors',factory)
