"""Marshal helper inputs and caller-owned objects; native relations choose every result."""
import ast
import json
from pathlib import Path
from ast_transport import syntax_columns


def query(backend,name,options=None,tree=None,render=None):
    arguments=dict(apHelper=name,**(options or {}));files=[]
    if tree is not None:
        arguments.update(syntax_columns(tree));arguments.pop('receiverTransport',None)
        pending=[tree];strings=[]
        while pending:
            node=pending.pop();strings.append(node.value if isinstance(node,ast.Constant) and isinstance(node.value,str) else '')
            pending.extend(reversed(list(ast.iter_child_nodes(node))))
        arguments['apHelperStrings']=strings
    if render is not None:
        try:text=ast.unparse(render)
        except Exception:arguments['apHelperRenderFailure']='1'
        else:
            if not text:arguments['apHelperRenderEmpty']='1'
            else:files=[dict(path='api/_helper_input.py',source='def _helper(expression: '+text+'):\n    pass\n')]
    return backend.native('async_patterns','_ap_helper',files,arguments)


def value(rows):
    if len(rows)!=1 or rows[0]['code']!='AP_HELPER':raise ValueError('One native async helper value required')
    return json.loads(rows[0]['parameters']['value_json'])


def install(backend,module,collector):
    def context(owner,line,radius=2):return value(query(backend,'context',dict(apHelperLines=owner.source_lines,apHelperLine=str(line),apHelperRadius=str(radius))))
    collector._get_context=context
    collector._annotation_contains_type_name=lambda owner,node,names:value(query(backend,'annotation',dict(apHelperTypes=list(names)),tree=node))
    def infer(owner,node):
        rows=query(backend,'infer',tree=node)
        if any(r['code']!='AP_NAME' for r in rows):raise ValueError('Unexpected native transaction parameter')
        return {r['parameters']['value'] for r in rows}
    collector._infer_transaction_param_names=infer
    collector._receiver_root_name=lambda owner,node:value(query(backend,'receiver',tree=node))
    collector._is_session_factory_expr=lambda owner,node:value(query(backend,'session',tree=node))
    collector._is_query_chain_from_txn=lambda owner,node:value(query(backend,'query',dict(apHelperTxn=list(owner.async_txn_name_stack[-1]) if owner.async_txn_name_stack else []),tree=node))
    collector._is_async_decorator=lambda owner,node,name:value(query(backend,'decorator',dict(apHelperDecorator=name),tree=node))
    collector._is_marked_fallback=lambda owner,line:value(query(backend,'marked',dict(apHelperLines=owner.source_lines,apHelperLine=str(line),apHelperRadius='5')))
    collector._expr_text=lambda owner,node:value(query(backend,'render',render=node))
    for kind in ['txn','session','query']:
        def bind(kind):
            def get(owner):
                stack=getattr(owner,'async_'+kind+'_name_stack');index=value(query(backend,'getter',dict(apHelperStackSize=str(len(stack)))))
                return set() if index==-1 else stack[index]
            return get
        setattr(collector,'_get_async_'+kind+'_names',bind(kind))
    def root(filepath):
        resolved=Path(filepath).resolve();parents=[resolved.parent,*resolved.parents]
        return Path(value(query(backend,'root',dict(apParentPaths=[str(p) for p in parents],apParentMarkers=[str(int((p/'pyproject.toml').exists())) for p in parents],apParentFallback=str(resolved.parent)))))
    backend.replace(module,'_analysis_root_for_file',root)
