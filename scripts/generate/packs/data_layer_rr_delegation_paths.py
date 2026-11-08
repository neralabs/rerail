"""Compile the pinned collector's literal path/name exemptions into Datalog conditions."""
import ast,json


def acceptance_paths(source):
    module=ast.parse((source/'scripts/lint_async_sync_delegation.py').read_text())
    cls=next(n for n in module.body if isinstance(n,ast.ClassDef) and n.name=='AsyncDelegationCollector')
    function=next(n for n in cls.body if isinstance(n,ast.FunctionDef) and n.name=='_is_acceptable_pattern')
    rules=[]
    def literal(node,env):
        if isinstance(node,ast.Name) and node.id in env:return literal(env[node.id],env)
        return ast.literal_eval(node)
    def term(node,env):
        if isinstance(node,ast.Name):
            if node.id in {'path','name'}:return node.id
            if node.id in env:return term(env[node.id],env)
        if isinstance(node,ast.Attribute) and isinstance(node.value,ast.Name) and node.value.id=='self' and node.attr=='file_path':return 'path'
        if isinstance(node,ast.Constant):return json.dumps(node.value)
        raise ValueError('Unsupported exemption term: '+ast.dump(node))
    def condition(node,env):
        if isinstance(node,ast.Name) and node.id in env:return condition(env[node.id],env)
        if isinstance(node,ast.Constant) and isinstance(node.value,bool):return '1 = '+str(int(node.value))
        if isinstance(node,ast.BoolOp):return '('+(' , ' if isinstance(node.op,ast.And) else ' ; ').join(condition(n,env) for n in node.values)+')'
        if isinstance(node,ast.Call) and isinstance(node.func,ast.Attribute) and node.func.attr in {'startswith','endswith'} and len(node.args)==1:
            value=term(node.func.value,env);patterns=literal(node.args[0],env);patterns=patterns if isinstance(patterns,tuple) else (patterns,)
            name='prefix' if node.func.attr=='startswith' else 'suffix'
            return '('+' ; '.join(name+'('+json.dumps(p)+', '+value+')' for p in patterns)+')' if patterns else '1 = 0'
        if isinstance(node,ast.Compare) and len(node.ops)==len(node.comparators)==1:
            left,right=node.left,node.comparators[0];op=node.ops[0]
            if isinstance(op,ast.Eq):return term(left,env)+' = '+term(right,env)
            if isinstance(op,ast.In):
                try:values=literal(right,env)
                except (ValueError,TypeError):values=None
                if isinstance(values,(tuple,list,set,frozenset)):return '('+' ; '.join(term(left,env)+' = '+json.dumps(v) for v in sorted(values))+')' if values else '1 = 0'
                return 'contains('+term(left,env)+', '+term(right,env)+')'
        raise ValueError('Unsupported exemption condition: '+ast.dump(node))
    def walk(body,env,conditions):
        env=dict(env)
        for statement in body:
            if isinstance(statement,ast.Expr) and isinstance(statement.value,ast.Constant) and isinstance(statement.value.value,str):continue
            if isinstance(statement,ast.Assign) and len(statement.targets)==1 and isinstance(statement.targets[0],ast.Name):env[statement.targets[0].id]=statement.value;continue
            if isinstance(statement,ast.If):
                if statement.orelse:raise ValueError('An exemption branch gained an else: review its control flow')
                walk(statement.body,env,[*conditions,condition(statement.test,env)]);continue
            if isinstance(statement,ast.For) and isinstance(statement.target,ast.Tuple):
                values=literal(statement.iter,env)
                for value in sorted(values):
                    local=dict(env)
                    for target,item in zip(statement.target.elts,value,strict=True):
                        if not isinstance(target,ast.Name):raise ValueError('Unsupported exemption target')
                        local[target.id]=ast.Constant(item)
                    walk(statement.body,local,conditions)
                continue
            if isinstance(statement,ast.Return) and isinstance(statement.value,ast.Constant) and isinstance(statement.value.value,bool):
                if statement.value.value:rules.append('asdp_ab_skip(path, name) :- asdp_candidate(path, name), '+', '.join(conditions)+'.')
                continue
            raise ValueError('Unsupported exemption statement: '+ast.dump(statement))
    walk(function.body,{},[])
    return '\n.decl asdp_ab_skip(path: symbol, name: symbol)\n'+'\n'.join(rules)+'\n'
