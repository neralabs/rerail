"""Transfer type-safety APIs to typed .rr decisions and passive DTO storage."""
import os
from enum import Enum
from functools import wraps
import inspect
import json
from pathlib import Path
import re
from types import SimpleNamespace
from typed_fact_transport import encode_bundles
from types_cli_port import install as install_cli,one

CHECK='type_safety_patterns'


def allow_options(module):
    values=[(c.name,p) for c,paths in module.ALLOWLIST.items() for p in sorted(paths)]
    return dict(tspAllowOverride='1',tspAllowCategories=[c for c,_ in values],tspAllowPaths=[p for _,p in values])


def supplied_files(suite, check, entry, sources, options=None):
    files = [dict(path=f'input/{i}/source.py', source=source) for i, (_, _, source) in enumerate(sources)]
    options = dict(options or {}, apiInputDirs=['input'], apiBuffers=[f['path'] for f in files],
                   apiSourcePaths=[p for p, _, _ in sources], apiSourceNames=[p.name for _, p, _ in sources])
    rows = suite.native(check, entry, files, options)
    originals = {file['path']: original for file, (_, original, _) in zip(files, sources, strict=True)}
    return rows, originals


def scan(suite, entry, module, args):
    check = 'type_safety_patterns'
    if entry == 'normalize_path':
        rows = suite.native(check, entry, [], dict(filePath=Path(args['file_path']).as_posix(), basePath=Path(args['base_dir']).as_posix()))
        if len(rows) != 1 or rows[0]['code'] != 'TSP_NORMALIZED':
            raise AssertionError('Native relative-path API must return exactly one value')
        return rows[0]['parameters']['result']
    if entry == 'is_allowlisted':
        rows = suite.native(check, entry, [], dict(**allow_options(module),category=args['category'].name,
            filePath=Path(args['file_path']).as_posix(), basePath=Path(args['base_dir']).as_posix()))
        if len(rows) > 1 or any(r['code'] != 'TSP_ALLOWED' for r in rows):
            raise AssertionError('Invalid native allowlist predicate')
        return bool(rows)
    if entry == 'find_python_files':
        root = Path(args['directory'])
        # Enumerate without the source scanner's pruning. SQL owns the complete filter.
        paths = [Path(parent) / name for parent, _, names in os.walk(root) for name in names]
        rows, originals = supplied_files(suite, check, entry,
            [(path.relative_to(root).as_posix(), path, '') for path in paths])
        if any(r['code'] != 'TSP_DISCOVERED' for r in rows):
            raise AssertionError('Invalid native discovery output')
        selected = {r['path'] for r in rows}
        return iter(path for name, path in originals.items() if name in selected)
    if entry == '_dedupe_issues':
        issues = args['issues']
        rows = suite.native(check, entry, [], dict(issuePaths=[str(v.file_path) for v in issues],
            issueLines=[v.line_number for v in issues], issueCategories=[v.category.value for v in issues]))
        if any(r['code'] != 'TSP_DEDUPE' for r in rows):
            raise AssertionError('Invalid native issue deduplication')
        return [issues[r['parameters']['ordinal']] for r in sorted(rows, key=lambda r: r['parameters']['ordinal'])]
    if entry == '_issues_for_files':
        paths = list(args['files'])
    else:
        paths = [Path(args['file_path'])]
    root = Path(args['base_dir'])
    if entry in {'_issues_for_files','check_function_signature_content'}:
        options=dict(tspPaths=[str(p) for p in paths],tspScopes=[p.relative_to(root).as_posix() if p.is_relative_to(root) else p.as_posix() for p in paths],tspNames=[p.name for p in paths],tspExists=[str(int(p.is_file())) for p in paths],tspPrepareEntry=entry)
        if entry=='check_function_signature_content':options['tspCallSource']=args['content']
        rows=suite.native(check,'_tsp_prepare',[],options)
        if any(r['code']!='TSP_READ' for r in rows):raise ValueError('Unexpected type-safety preparation output')
        paths=[paths[r['parameters']['ordinal']] for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]
    sources = []
    for path in paths:
        try:
            content = (args['content'] if entry == 'check_file_content_for_patterns' else path.read_text())
        except (OSError, UnicodeDecodeError):
            if entry in {'_issues_for_files','check_function_signature_content'}:raise
            continue
        name = (path.relative_to(root).as_posix() if path.is_relative_to(root)
                else '__outside_root__/' + path.as_posix().lstrip('/'))
        sources.append((name, path, content))
    options = dict(callSource=args['content']) if entry == 'check_function_signature_content' else {}
    options.update(allow_options(module))
    rows, originals = supplied_files(suite, check, entry, sources, options)
    if any(r['code'] != 'TSP001' for r in rows):
        raise AssertionError('Unexpected native type-safety diagnostic')
    order = {path: i for i, path in enumerate(originals)}
    def key(row):
        ordinal = row['parameters']['ordinal']
        return (order[row['path']], 0, row['line'], ordinal) if ordinal < 5 else (order[row['path']], 1, ordinal, 0)
    return [module.TypeSafetyIssue(category=module.IssueCategory[r['parameters']['category']],
        file_path=originals[r['path']], line_number=r['line'], column=r['parameters']['column'],
        code_snippet=r['parameters']['snippet'], suggestion=r['parameters']['suggestion']) for r in sorted(rows, key=key)]


def install(backend,module,detectors=False):
    def native(entry,files,options=None,*,root=None):
        return backend.native(CHECK,entry,files,{'apiInputDirs':['__tsp_no_primary__'],**(options or {})},root=root)
    rows=native('_tsp_configuration',[])
    if not rows or any(r['code']!='TSP_CONFIG' for r in rows):raise ValueError('Missing type-safety configuration')
    config={r['parameters']['key']:r['parameters']['value'] for r in rows}
    categories=Enum('IssueCategory',json.loads(config['categories']),module=module.__name__)
    backend.replace(module,'IssueCategory',categories)
    backend.replace(module,'ALLOWLIST',{categories[c]:set(paths) for c,paths in json.loads(config['allowlist']).items()})
    for name,(pattern,flags) in json.loads(config['patterns']).items():backend.replace(module,name,re.compile(pattern,flags))
    for name,key in [('PATTERN_SOURCE_TRIGGERS','pattern_triggers'),('SIGNATURE_SOURCE_TRIGGERS','signature_triggers')]:backend.replace(module,name,tuple(json.loads(config[key])))
    data=json.loads(config['extractor'])
    for name in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']:data[name]=tuple(data[name])
    backend.replace(module,'TYPE_SAFETY_PATTERNS_EXTRACTOR',type(module.TYPE_SAFETY_PATTERNS_EXTRACTOR)(**data))
    if hasattr(backend,'configuration_bindings'):backend.configuration_bindings[CHECK]=dict(nativeCall=len(backend.calls)-1,names=['IssueCategory','ALLOWLIST',*json.loads(config['patterns']),'PATTERN_SOURCE_TRIGGERS','SIGNATURE_SOURCE_TRIGGERS','TYPE_SAFETY_PATTERNS_EXTRACTOR'])
    def collected(path,source,root):
        logical=path.relative_to(root).as_posix() if path.is_relative_to(root) else path.as_posix()
        return native('_tsp_collect',[dict(path='input/collector.py',source=source)],dict(apiInputDirs=['input'],apiBuffers=['input/collector.py'],apiSourcePaths=[logical],apiSourceNames=[path.name],tspCollectName=path.name,**allow_options(module)))
    def ordered(rows):
        return sorted((r for r in rows if r['code']=='TSP001'),key=lambda r:(0,r['line'],r['parameters']['ordinal']) if r['parameters']['ordinal']<5 else (1,r['parameters']['ordinal'],0))
    def issues(rows,path):
        return [module.TypeSafetyIssue(categories[p['category']],path,r['line'],p['column'],p['snippet'],p['suggestion']) for r in ordered(rows) for p in [r['parameters']]]
    def facts(rows,path):
        return [dict(category=p['category'],file_path=path.as_posix(),line_number=r['line'],column=p['column'],code_snippet=p['snippet'],suggestion=p['suggestion']) for r in ordered(rows) for p in [r['parameters']]]
    class Collector(module.AstEventCollector):
        def __init__(self,file_path,base_dir):self.file_path=file_path;self.base_dir=base_dir;self.source='';self.lines=[];self.issues=[]
        def start_file(self,_path,source,_context):self.source=source;self.lines=source.splitlines()
        def finish_file(self):
            rows=collected(self.file_path,self.source,self.base_dir);self.issues=issues(rows,self.file_path);return facts(rows,self.file_path)
    backend.replace(module,'TypeSafetyPatternsCollector',Collector)
    backend.replace(module,'type_safety_patterns_fact_extractors',lambda root:{module.TYPE_SAFETY_PATTERNS_EXTRACTOR.extractor_id:lambda path,source:Collector(path,root)})
    def extract(path,source,root):
        rows=collected(path,source,root);present=one(rows,'TSP_COLLECT_STATUS','present');errors=[r['parameters']['value'] for r in rows if r['code']=='TSP_COLLECT_ERROR'];family=module.TYPE_SAFETY_PATTERNS_EXTRACTOR.extractor_id
        return SimpleNamespace(facts={family:facts(rows,path)} if present else {},errors={family:errors[0]} if errors else {})
    module._rerail_types_extract=extract
    def project(issue):
        data=dict(category=issue.category.name,file_path=issue.file_path.as_posix(),line_number=issue.line_number,column=issue.column,code_snippet=issue.code_snippet,suggestion=issue.suggestion)
        rows=native('_issue_to_fact',[dict(path='type-safety-value.json',source=json.dumps(data,ensure_ascii=False))])
        if len(rows)!=1 or rows[0]['code']!='TSP_PROJECTED':raise ValueError('One native type-safety projection required')
        p=rows[0]['parameters'];return dict(category=p['category'],file_path=p['path'],line_number=json.loads(p['line_json']),column=json.loads(p['col_json']),code_snippet=json.loads(p['snippet_json']),suggestion=json.loads(p['suggestion_json']))
    backend.replace(module,'_issue_to_fact',project)
    def load(raw):
        bundles=encode_bundles([dict(path='',source_path='',value=[raw])]);rows=native('_issue_from_fact',[dict(path='reporter-facts.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False,allow_nan=False))])
        for r in rows:
            if r['code']=='TSP_LOAD_ERROR':raise AttributeError(r['parameters']['message'])
        present=one(rows,'TSP_LOAD_STATUS','present')
        if not present:return None
        p=next(r['parameters'] for r in rows if r['code']=='TSP_LOADED');return module.TypeSafetyIssue(categories[p['category']],Path(p['path']),json.loads(p['line_json']),json.loads(p['col_json']),p['snippet'],p['suggestion'])
    backend.replace(module,'_issue_from_fact',load)
    def plugin(root):
        dirs=native('_tsp_plugin_dirs',[],root=root);scope=json.loads(config['file_scope']);scope['directories']=tuple(root/r['parameters']['path'] for r in sorted(dirs,key=lambda r:(r['parameters']['rank'],r['parameters']['path'])));scope['explicit_files']=tuple(Path(p) for p in scope['explicit_files'])
        def report(view):
            family=module.TYPE_SAFETY_PATTERNS_EXTRACTOR.extractor_id;bundles=encode_bundles([dict(path=str(b.path),source_path=str(b.path),value=b.facts.get(family,[])) for b in view.files.values()]);rows=native('_report_facts',[dict(path='reporter-facts.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False,allow_nan=False))])
            if any(r['code'] not in {'SF_ROOT','SF_REPORT'} for r in rows):raise ValueError('Unexpected type-safety reporter response')
            ordered=sorted((r['parameters'] for r in rows if r['code']=='SF_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal']))
            return [module.StructuredViolation(CHECK,p['source_code'],Path(p['file']),json.loads(p['line_json']),p['source_message'],p['severity']) for p in ordered]
        return module.LinterPlugin(**json.loads(config['plugin']),file_scope=module.FileScopeSpec(**scope),required_facts=(module.TYPE_SAFETY_PATTERNS_EXTRACTOR,),reporter=report)
    backend.replace(module,'type_safety_patterns_plugin',plugin)
    install_cli(backend,module,config,native)
    if detectors:
        for entry in ['check_file_for_patterns','check_file_content_for_patterns','check_function_signatures','check_function_signature_content','normalize_path','is_allowlisted','find_python_files','_issues_for_files','_dedupe_issues']:
            original=getattr(module,entry);signature=inspect.signature(original)
            def bind(entry,original,signature):
                @wraps(original)
                def run(*args,**kwargs):
                    bound=signature.bind(*args,**kwargs);bound.apply_defaults();return scan(backend,entry,module,bound.arguments)
                return run
            backend.replace(module,entry,bind(entry,original,signature))
