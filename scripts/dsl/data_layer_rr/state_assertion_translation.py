"""Replace Python observer plumbing with actual native analysis-scope observations."""
import ast


NAMES={
    'test_function_fact_analysis_excludes_unreachable_functions':('facts','captured','collect','_collect_function_facts'),
    'test_runtime_authority_analysis_skips_unreachable_functions':('runtime','analyzed','record_analysis','_function_returns_runtime_authority'),
}


def translate(tree):
    fn=next(n for n in tree.body if isinstance(n,ast.FunctionDef))
    if fn.name not in NAMES:return []
    family,destination,spy,hook=NAMES[fn.name]
    assertions=[ast.dump(n,include_attributes=False) for n in ast.walk(fn) if isinstance(n,ast.Assert)]
    nested=next(n for n in fn.body if isinstance(n,ast.FunctionDef) and n.name==spy)
    lifted=[n for n in nested.body if isinstance(n,ast.Assert)]
    body=[];sites=[];observed=False
    for node in fn.body:
        if node is nested:
            sites.append(node.lineno);continue
        if isinstance(node,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='original' for t in node.targets):
            if not isinstance(node.value,ast.Attribute) or node.value.attr!=hook:
                raise ValueError('Unexpected original State observer assignment')
            sites.append(node.lineno);continue
        if (isinstance(node,ast.Expr) and isinstance(node.value,ast.Call) and isinstance(node.value.func,ast.Attribute)
                and node.value.func.attr=='setattr' and len(node.value.args)>=2
                and isinstance(node.value.args[1],ast.Constant) and node.value.args[1].value==hook):
            sites.append(node.lineno);continue
        body.append(node)
        call=node.value if isinstance(node,(ast.Assign,ast.Expr)) else None
        if isinstance(call,ast.Call) and isinstance(call.func,ast.Name) and call.func.id=='scan_source':
            if len(call.args)!=2 or call.keywords:raise ValueError('Unexpected original State scanner arguments')
            query=ast.Call(func=ast.Name(id='_rr_state_analysis_scope',ctx=ast.Load()),args=[*call.args,ast.Constant(value=family)],keywords=[])
            if family=='facts':
                body.append(ast.copy_location(ast.Assign(targets=[ast.Name(id='function_nodes',ctx=ast.Store())],value=query),node))
                body.extend(lifted)
                rows=ast.Name(id='function_nodes',ctx=ast.Load())
            else:rows=query
            # Native rows are [line, name, class scope]. No Python analysis or
            # original observer callback executes in this assertion transfer.
            names=ast.GeneratorExp(elt=ast.Subscript(value=ast.Name(id='_rr_row',ctx=ast.Load()),slice=ast.Constant(value=1),ctx=ast.Load()),
                generators=[ast.comprehension(target=ast.Name(id='_rr_row',ctx=ast.Store()),iter=rows,ifs=[],is_async=0)])
            body.append(ast.copy_location(ast.Expr(value=ast.Call(func=ast.Attribute(value=ast.Name(id=destination,ctx=ast.Load()),attr='extend',ctx=ast.Load()),args=[names],keywords=[])),node))
            observed=True;sites.append(node.lineno)
    if not observed:raise ValueError('State observer transfer did not find its scanner call')
    fn.body=body
    after=[ast.dump(n,include_attributes=False) for n in ast.walk(fn) if isinstance(n,ast.Assert)]
    if sorted(after)!=sorted(assertions):raise ValueError('State observer transfer changed an assertion predicate')
    return [dict(line=line,kind='native-analysis-scope-observation') for line in sites]
