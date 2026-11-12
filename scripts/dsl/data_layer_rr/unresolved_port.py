"""Transport FieldInfo file inputs, external AST DTOs, and result dictionaries."""
import ast
import inspect
import json
from pathlib import Path

CHECK = 'unresolved_forward_refs'


def analyzed_state(owner, returned):
    """Expose the complete observable instance state without invoking source methods."""
    return dict(returned=returned, file_path=owner.file_path, findings=owner.findings, source_lines=owner.source_lines,
                current_function=owner.current_function, current_function_lineno=owner.current_function_lineno,
                resolved_hint_vars=sorted(owner.resolved_hint_vars))


def supplied_snapshot(owner, tree):
    from ast_transport import columns
    root = Path(owner.file_path).parent
    try:
        source = owner.file_path.read_text()
    except (OSError, UnicodeError):
        files = []
    else:
        files = [dict(path=Path(owner.file_path).name, source=source)]
    return files, dict(include=['**/*.py'], exclude=[]), root, columns(tree)


def identity_graph(container, target):
    """Serialize object identities and child edges; native reachability answers containment."""
    objects, pending, seen, parents, children = {}, [container, target], set(), [], []
    def key(node):
        return str(objects.setdefault(id(node), len(objects)))
    container_id, target_id = key(container), key(target)
    while pending:
        node = pending.pop()
        if id(node) in seen:
            continue
        seen.add(id(node))
        for child in ast.iter_child_nodes(node):
            parents.append(key(node))
            children.append(key(child))
            pending.append(child)
    return dict(containerId=container_id, targetId=target_id, graphParents=parents, graphChildren=children)


def scan(suite, entry, module, arguments):
    if entry == 'AnnotationUsageFinder.analyze':
        from ast_transport import columns
        owner = arguments['self']
        options = columns(arguments['tree'])
        read_error = None
        try:
            source = owner.file_path.read_text()
        except UnicodeDecodeError:
            source = ''
        except OSError as error:
            read_error, source = error, ''
        options.update(providedSource=source, sourceLines=source.splitlines(),
                       readError=str(read_error is not None).lower(), ufOwnerPath=str(owner.file_path))
        rows = suite.native(CHECK, entry, [], options)
        if any(row['code'] == 'UF_READ_ERROR' for row in rows):
            if len(rows) != 1 or read_error is None:
                raise AssertionError('Invalid supplied-AST read failure')
            raise read_error
        if any(row['code'] != 'unresolved_forward_ref' for row in rows):
            raise AssertionError('Invalid native supplied-AST finding')
        owner.findings.extend(dict(file=str(owner.file_path), line=row['line'], function=row['parameters']['function'],
            problem=row['parameters']['problem'], source=row['parameters']['source'])
            for row in sorted(rows, key=lambda row: row['parameters']['ordinal']))
        return None
    if entry.startswith('AnnotationUsageFinder.'):
        options, files = {}, []
        if entry.endswith('._node_contains'):
            options = identity_graph(arguments['container'], arguments['target'])
        elif entry.endswith(('._is_using_resolved_hints', '._is_assignment_from_resolved')):
            pass  # Constant source predicates have no relevant input fields.
        else:
            supplied = arguments.get('parent', arguments.get('node'))
            # Tests supply Python AST values. Printing is test-input transport, never a rule callback.
            files = [dict(path='impl/helper.py', source='_rr_input = ' + ast.unparse(supplied) + '\n')]
        rows = suite.native(CHECK, entry, files, options)
        if len(rows) > 1 or any(row['code'] != 'UF_HELPER_TRUE' for row in rows):
            raise AssertionError('Invalid native unresolved-reference helper result')
        return bool(rows)
    path = Path(arguments['file_path'])
    read_error = None
    try:
        source = path.read_text(encoding='utf-8')
    except UnicodeDecodeError:
        source = ''
    except OSError as error:
        read_error, source = error, ''
    # The workspace loader applies policy scope to the synthetic buffer path.
    # The caller's original path and suffix still govern the public API result.
    rows = suite.native(CHECK, entry, [dict(path='impl/input.py', source=source)],
                        dict(suffix=path.suffix, isFile=str(path.is_file()).lower(),
                             relativePath=str(not path.is_absolute()).lower(),
                             readError=str(read_error is not None).lower()))
    if any(row['code'] == 'UF_READ_ERROR' for row in rows):
        if len(rows) != 1 or read_error is None:
            raise AssertionError('Invalid native unresolved-reference read failure')
        raise read_error
    if any(row['code'] != 'unresolved_forward_ref' for row in rows):
        raise AssertionError('Unexpected native unresolved-reference finding')
    return [dict(file=str(path), line=row['line'], function=row['parameters']['function'],
                 problem=row['parameters']['problem'], source=row['parameters']['source'])
            for row in sorted(rows, key=lambda row: row['parameters']['ordinal'])]


def source_content(suite, path, source, tree, root=None):
    from ast_transport import columns
    options=columns(tree) if tree is not None else {}
    options.update(providedSource=source,sourceLines=source.splitlines(),ufOwnerPath=str(path),ufHasTree=str(tree is not None).lower())
    if root is not None:options['ufFactRoot']=str(root)
    rows=suite.native(CHECK,'analyze_source_content',[],options)
    errors=[r for r in rows if r['code']=='UF_SOURCE_ERROR']
    if errors:
        if len(rows)!=1:raise ValueError('One native unresolved source error required')
        raise ValueError(errors[0]['parameters']['detail'])
    if any(r['code']!='unresolved_forward_ref' for r in rows):raise ValueError('Unexpected native unresolved source response')
    return [dict(file=p['file'],line=r['line'],function=p['function'],problem=p['problem'],source=p['source'])
            for r in sorted(rows,key=lambda r:r['parameters']['ordinal']) for p in [r['parameters']]]


def install(suite, module, detectors=False):
    rows = suite.native(CHECK, 'configuration', [])
    tables = {}
    for row in rows:
        if row['code'] != 'UF_CONFIG':
            raise AssertionError('Invalid native unresolved-reference configuration')
        tables.setdefault(row['parameters']['key'], set()).add(row['parameters']['value'])
    names=['TYPE_INSPECTION_FUNCTIONS', 'RESOLVED_PATTERNS']
    for key in names:
        suite.replace(module, key, tables[key])
    config={key:json.loads(next(iter(tables[key]))) for key in ['extractor','plugin','scope']}
    spec=dict(config['extractor'])
    for key in ['source_triggers','project_input_facts']:spec[key]=tuple(spec[key])
    spec['source_trigger_groups']=tuple(tuple(group) for group in spec['source_trigger_groups'])
    suite.replace(module,'UNRESOLVED_FORWARD_REFS_EXTRACTOR',type(module.UNRESOLVED_FORWARD_REFS_EXTRACTOR)(**spec))
    if not hasattr(suite, 'configuration_bindings'):
        suite.configuration_bindings = {}
    suite.configuration_bindings[CHECK] = dict(nativeCall=len(suite.calls)-1,
        names=[*names,'UNRESOLVED_FORWARD_REFS_EXTRACTOR'])
    suite.replace(module,'analyze_source_content',lambda file_path,source,tree=None:source_content(suite,file_path,source,tree))
    def registry(root):
        return {module.UNRESOLVED_FORWARD_REFS_EXTRACTOR.extractor_id:
                lambda path,source,tree=None:source_content(suite,path,source,tree,root)}
    suite.replace(module,'unresolved_forward_refs_fact_extractors',registry)
    def plugin(root):
        from typed_fact_transport import encode_bundles
        scope=dict(config['scope'])
        scope['directories']=tuple(root/p for p in scope['directories'])
        scope['explicit_files']=tuple(Path(p) for p in scope['explicit_files'])
        def report(view):
            family=module.UNRESOLVED_FORWARD_REFS_EXTRACTOR.extractor_id
            bundles=encode_bundles([dict(path=str(b.path),source_path=str(b.path),value=b.facts.get(family,[])) for b in view.files.values()])
            rows=suite.native(CHECK,'_report_facts',[dict(path='reporter-facts.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False,allow_nan=False))])
            if any(r['code'] not in {'SF_ROOT','SF_REPORT'} for r in rows):raise ValueError('Unexpected native unresolved report response')
            ordered=sorted((r['parameters'] for r in rows if r['code']=='SF_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal']))
            return [module.StructuredViolation(CHECK,p['source_code'],Path(p['file']),json.loads(p['line_json']),p['source_message'],p['severity']) for p in ordered]
        return module.LinterPlugin(**config['plugin'],file_scope=module.FileScopeSpec(**scope),required_facts=(module.UNRESOLVED_FORWARD_REFS_EXTRACTOR,),reporter=report)
    suite.replace(module,'unresolved_forward_refs_plugin',plugin)
    if detectors:
        import functools
        from suite import ENTRIES
        for entry in ENTRIES[CHECK]:
            owner,name=(module.AnnotationUsageFinder,entry.split('.',1)[1]) if '.' in entry else (module,entry)
            original=getattr(owner,name);signature=inspect.signature(original)
            def bind(entry,original,signature):
                @functools.wraps(original)
                def run(*args,**kwargs):
                    bound=signature.bind(*args,**kwargs);bound.apply_defaults();return scan(suite,entry,module,bound.arguments)
                return run
            suite.replace(owner,name,bind(entry,original,signature))

