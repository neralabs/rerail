"""Transport source, AST arguments, and DTOs for native async/sync delegation rules."""
import ast
from pathlib import Path
from ast_transport import visit_columns,list_lengths


def body_identity_probe(module, tree):
    """Observe list ownership and original node identities without formatting syntax."""
    body = tree.body
    result = module._strip_method_docstring(body)
    return dict(sameList=result is body, indices=[next(i for i,n in enumerate(body) if n is node) for node in result])


def install(suite,module):
    def native(entry,files=(),options=None):return suite.native('async_sync_delegation',entry,list(files),options)
    for row in native('configuration'):
        p=row['parameters'];suite.replace(module,p['name'],p['value'])
    suite.replace(module,'ALLOWLISTED_CATEGORY_C_TARGETS',[])
    def argument(entry,node=None,extra=None):
        data={};nodes=[]
        if node is not None:
            data=visit_columns(node);data.update(list_lengths(node));pending=[node]
            while pending:
                value=pending.pop();nodes.append(value);pending.extend(reversed(list(ast.iter_child_nodes(value))))
            data['astConstantText']=[str(n.value) if isinstance(n,ast.Constant) else '' for n in nodes]
        data.update(extra or {})
        return native(entry,options=data),nodes
    def boolean(entry,node=None,extra=None):return any(r['code']=='ASD_TRUE' for r in argument(entry,node,extra)[0])
    def value(entry,node=None,extra=None):return next((r['parameters']['value'] for r in argument(entry,node,extra)[0] if r['code']=='ASD_VALUE'),None)
    suite.replace(module,'_has_mixed_lazy_execution_name',lambda name:boolean('_has_mixed_lazy_execution_name',extra=dict(helperName=name)))
    suite.replace(module,'_normalize_helper_family',lambda helper_name:value('_normalize_helper_family',extra=dict(helperName=helper_name)))
    suite.replace(module,'_extract_callable_name',lambda node:value('_extract_callable_name',node))
    def strip(body):
        rows,nodes=argument('_strip_method_docstring',ast.Module(body=body,type_ignores=[]))
        return [nodes[r['parameters']['n']] for r in sorted(rows,key=lambda r:r['parameters'].get('ordinal',0)) if r['code']=='ASD_NODE']
    suite.replace(module,'_strip_method_docstring',strip)
    suite.replace(module,'_is_abstract_like',lambda node:boolean('_is_abstract_like',node))
    suite.replace(module,'_has_decorator_named',lambda node,names:boolean('_has_decorator_named',node,dict(decoratorNames=list(names))))
    suite.replace(module,'_decorator_names',lambda node:{r['parameters']['value'] for r in argument('_decorator_names',node)[0] if r['code']=='ASD_VALUE'})
    suite.replace(module,'_is_lazy_decorated',lambda node:boolean('_is_lazy_decorated',node))
    def requirements(node):
        row=next((r['parameters'] for r in argument('_legacy_wrapper_requirements',node)[0] if r['code']=='ASD_REQUIREMENT'),None)
        return tuple(row[k] for k in ['base','lazy','counterpart','decorator']) if row is not None else None
    suite.replace(module,'_legacy_wrapper_requirements',requirements)
    suite.replace(module,'_looks_like_callback_entrypoint',lambda node:boolean('_looks_like_callback_entrypoint',node))
    suite.replace(module,'_looks_like_transaction_bound_eager_entrypoint',lambda node:boolean('_looks_like_transaction_bound_eager_entrypoint',node))
    def stats(stats,root_dir=None):
        rows=native('ViolationStats.to_dict',options=dict(categoryA=stats.category_a_count,categoryB=stats.category_b_count,categoryC=stats.category_c_count,total=stats.total_count,files=list(stats.files_with_violations),offenderPaths=list(stats.worst_offenders),offenderCounts=list(stats.worst_offenders.values()),hasRoot=root_dir is not None,root=str(root_dir) if root_dir is not None else ''))
        p=next(r['parameters'] for r in rows if r['code']=='ASD_STATS')
        offenders=[r['parameters'] for r in rows if r['code']=='ASD_OFFENDER']
        return dict(category_a=dict(count=p['a'],description=p['a_description']),category_b=dict(count=p['b'],description=p['b_description']),category_c=dict(count=p['c'],description=p['c_description']),summary=dict(total_violations=p['total'],files_affected=p['files']),worst_offenders={r['path']:r['count'] for r in sorted(offenders,key=lambda r:r['ordinal'])})
    suite.replace(module.ViolationStats,'to_dict',stats)
    def analyze_source(file_path,source,tree=None):
        rows=native('analyze_source',[dict(path='input.py',source=source)],dict(originalPath=str(file_path),originalName=Path(file_path).name,allowlistedTargets=list(module.ALLOWLISTED_CATEGORY_C_TARGETS)))
        errors=[r['parameters']['error'] for r in rows if r['code']=='ASD_PARSE']
        if errors:raise SyntaxError(errors[0])
        return [module.Violation(file=r['path'],line=r['line'],function_name=p['function_name'],category=p['category'],description=p['description'],sync_counterpart=p['sync'] if p['has_sync'] else None,details=p['details'] if p['has_details'] else None) for r in sorted(rows,key=lambda r:(r['parameters'].get('phase',0),r['parameters'].get('ordinal',0))) if r['code']=='ASD_ANALYSIS' for p in [r['parameters']]]
    suite.replace(module,'analyze_source',analyze_source)
    def analyze_file(file_path):
        if not file_path.exists():return []
        try:return analyze_source(file_path,file_path.read_text())
        except SyntaxError:return []
    suite.replace(module,'analyze_file',analyze_file)
