"""Transport State Tripwires source arguments and native finding DTOs."""
import ast
import json
from pathlib import Path

CHECK = 'serverplan_state_tripwires'


def helper_transport(tree):
    """Serialize AST occurrences and identity; leave all selection to native rules."""
    from ast_transport import columns
    data=columns(tree);nodes=[];refs={};pending=[tree]
    while pending:
        node=pending.pop();nodes.append(node)
        refs.setdefault(id(node),len(refs))
        pending.extend(reversed(list(ast.iter_child_nodes(node))))
    data['helperAstRefs']=[str(refs[id(node)]) for node in nodes]
    data['astArgNames']=[getattr(node,'arg','') for node in nodes]
    return data,nodes,refs


def collect_functions(backend,body,*,class_stack=()):
    options,nodes,_=helper_transport(ast.Module(body=body,type_ignores=[]))
    options['helperClassStack']=json.dumps(class_stack,ensure_ascii=False,separators=(',',':'))
    rows=backend.native(CHECK,'_collect_function_nodes',[],options)
    if any(r['code']!='SPT_HELPER_COLLECT' for r in rows):
        raise ValueError('Invalid native function collection row')
    return [(nodes[r['parameters']['occurrence']],tuple(json.loads(r['parameters']['class_stack'])))
            for r in sorted(rows,key=lambda r:r['parameters']['occurrence'])]


def select_functions(backend,module,path,function_nodes,function_body_nodes_by_fn,*,
                     native_python_work_functions=None,called_names_by_fn=None):
    inputs=ast.Module(body=[fn for fn,_ in function_nodes],type_ignores=[])
    caches=[(fn,values) for fn,values in function_body_nodes_by_fn.items() if values is not None]
    cache_roots=[ast.Module(body=values,type_ignores=[]) for _,values in caches]
    # Extra references can include functions not present in the input forest.
    extras=list(native_python_work_functions or ()) + list((called_names_by_fn or {}).keys())
    tree=ast.Module(body=[inputs,ast.Module(body=cache_roots,type_ignores=[]),
                          ast.Module(body=extras+[fn for fn,_ in caches],type_ignores=[])],type_ignores=[])
    options,nodes,refs=helper_transport(tree)
    root_indices={id(node):i for i,node in enumerate(nodes) if node is inputs or any(node is c for c in cache_roots)}
    inputs_index=root_indices[id(inputs)]
    options.update(helperFunctionNodes=[str(i) for i,p in enumerate(options['astParents']) if int(p)==inputs_index and options['astFields'][i]=='body'],
        helperClassStacks=[json.dumps(stack,ensure_ascii=False,separators=(',',':')) for _,stack in function_nodes],
        helperCacheOwners=[str(refs[id(fn)]) for fn,_ in caches],
        helperCacheRoots=[str(root_indices[id(c)]) for c in cache_roots],
        helperNativeProvided=str(native_python_work_functions is not None).lower(),
        helperNativeRefs=[str(refs[id(fn)]) for fn in native_python_work_functions or ()],
        helperCalledProvided=str(called_names_by_fn is not None).lower(),
        helperCallerRefs=[],helperCalledNames=[],helperPath=path.as_posix(),
        moduleRoot=module.ROOT.as_posix(),filterPaths=[path.as_posix()])
    for fn,names in (called_names_by_fn or {}).items():
        for name in names:
            options['helperCallerRefs'].append(str(refs[id(fn)]));options['helperCalledNames'].append(name)
    rows=backend.native(CHECK,'_select_reachable_function_nodes',[],options)
    selected=[];walks={};objects={refs[id(node)]:node for node in nodes}
    for row in rows:
        p=row['parameters']
        if row['code']=='SPT_HELPER_SELECT':selected.append(p)
        elif row['code']=='SPT_HELPER_CACHE':walks.setdefault(p['owner'],[]).append((p['depth'],p['occurrence']))
        else:raise ValueError('Invalid native reachability row')
    for owner,occurrences in walks.items():
        function_body_nodes_by_fn[objects[owner]]=[nodes[n] for _,n in sorted(occurrences)]
    return [(function_nodes[p['ordinal']][0],tuple(json.loads(p['class_stack'])))
            for p in sorted(selected,key=lambda p:p['ordinal'])]


def analysis_contexts(backend,module,path,source):
    files,_,_=snapshot(module,dict(path=path,source=source),'scan_source')
    rows=backend.native(CHECK,'_analysis_contexts',files)
    facts=[];runtime=[]
    for row in rows:
        if row['code']!='SPT_ANALYSIS_CONTEXT':
            raise ValueError('Native State Tripwires analysis returned a non-context row')
        p=row['parameters']
        if p['family']=='facts':facts.append([p['line'],p['name'],p['class_scope']])
        elif p['family']=='runtime':runtime.append([p['line'],p['name']])
        else:raise ValueError('Unknown native State Tripwires analysis family')
    return dict(facts=sorted(facts),runtime=sorted(runtime))


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
    backend.replace(module,'_collect_function_nodes',lambda body,*,class_stack=():
                    collect_functions(backend,body,class_stack=class_stack))
    backend.replace(module,'_select_reachable_function_nodes',lambda path,function_nodes,function_body_nodes_by_fn,**kw:
                    select_functions(backend,module,path,function_nodes,function_body_nodes_by_fn,**kw))
    backend.replace(module,'ServerPlanStateTripwireCollector',NativeCollector)
