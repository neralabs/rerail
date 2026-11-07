"""Exercise complete transaction visitor states and independently supplied syntax."""
from pathlib import Path
from transaction_port import graph,violations


def probe(module,values,suite=None):
    mode=values['mode'];tree=values['tree'];path=Path(values.get('file_path','input.py'))
    def at(path):
        node=tree
        for key in path:node=node[key] if isinstance(key,int) else getattr(node,key)
        return node
    for edit in values.get('edits',[]):
        parent=at(edit['target'][:-1]);key=edit['target'][-1];value=at(edit['source'])
        if isinstance(key,int):parent[key]=value
        else:setattr(parent,key,value)
    if mode=='detector':
        if suite is None:
            detector=module.NonLocalStateDetector(set(values.get('outer',[])))
            detector.local_vars.update(values.get('initial',[]));detector.visit(tree)
            return dict(violations=detector.violations,local_vars=sorted(detector.local_vars),
                outer_scope_vars=sorted(detector.outer_scope_vars),in_nested_function=detector.in_nested_function)
        rows=graph(suite,'detector',tree,dict(outerVars=values.get('outer',[]),initialLocals=values.get('initial',[])))
        from transaction_port import ordered
        return dict(violations=[(r['line'],r['parameters']['column'],r['message']) for r in ordered(rows) if r['code']!='TX_LOCAL'],
            local_vars=sorted({r['parameters']['name'] for r in rows if r['code']=='TX_LOCAL'}),
            outer_scope_vars=sorted(set(values.get('outer',[]))),in_nested_function=False)
    if mode=='source':
        if suite is None:return module.lint_source(path,values.get('source',''),tree)
        from transaction_port import scan
        return scan(suite,module,'lint_source',dict(file_path=path,source=values.get('source',''),tree=tree))
    if suite is None:
        if mode=='inline':
            visitor=module.TransactionClosureLinter(path);visitor.visit(tree)
            return dict(violations=visitor.violations,scope_stack=[sorted(s) for s in visitor.scope_stack])
        if mode=='named':
            visitor=module.ClosureDefinitionFinder(path);visitor.visit(tree)
            return dict(violations=visitor.violations,current_scope_functions=visitor.current_scope_functions)
        collector=module.TransactionClosureCollector(path)
        module._run_collector_on_tree(collector,tree,path,values.get('source',''))
        return collector.finish_file()
    rows=graph(suite,'visitor',tree,dict(originalPath=str(path),checkInline=str(mode!='named').lower(),checkNamed=str(mode!='inline').lower()))
    found=violations(module,rows,{str(path):path})
    if mode=='inline':return dict(violations=found,scope_stack=[sorted({r['parameters']['name'] for r in rows if r['code']=='TX_SCOPE'})])
    if mode=='named':return dict(violations=found,current_scope_functions={})
    return [dict(line=v.line,col=v.col,message=v.message,severity=v.severity) for v in found]
