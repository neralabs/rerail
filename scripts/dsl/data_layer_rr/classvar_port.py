"""ClassVar metadata, value and collector transport; decisions execute in .rr."""
import json
from pathlib import Path
from ast_transport import scalar_columns
from fact_transport import kind,members
from python_value_transport import encode

CHECK='classvar_check'


def install(backend,module,detectors=False):
    rows=backend.native(CHECK,'_classvar_configuration',[])
    if not rows or any(r['code']!='CV_CONFIG' for r in rows):raise ValueError('Missing ClassVar configuration')
    config={r['parameters']['key']:r['parameters']['value'] for r in rows}
    backend.replace(module,'__doc__',config['docstring'])
    data=json.loads(config['extractor'])
    for key in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']:data[key]=tuple(data[key])
    backend.replace(module,'CLASSVAR_CHECK_EXTRACTOR',type(module.CLASSVAR_CHECK_EXTRACTOR)(**data))
    if hasattr(backend,'configuration_bindings'):
        backend.configuration_bindings[CHECK]=dict(nativeCall=len(backend.calls)-1,names=['__doc__','CLASSVAR_CHECK_EXTRACTOR'])

    class Collector(module.AstEventCollector):
        def __init__(self,filepath):self.filepath=filepath;self.tree=None
        def on_node(self,node,context):
            if self.tree is None:
                while context.parent(node) is not None:node=context.parent(node)
                self.tree=node
        def finish_file(self):
            options=dict(cvFactPath=self.filepath.as_posix())
            if self.tree is not None:options.update(scalar_columns(self.tree))
            rows=backend.native(CHECK,'_classvar_ast_collect',[],options)
            if any(r['code']!='CV_AST_FACT' for r in rows):raise ValueError('Unexpected ClassVar AST fact')
            return [dict(file=p['file'],line=p['line'],col=p['column'],message=r['message'],code='CLASSVAR001')
                    for r in sorted(rows,key=lambda r:(r['parameters']['visit'],r['parameters']['ordinal'])) for p in [r['parameters']]]
    backend.replace(module,'ClassVarCheckCollector',Collector)
    backend.replace(module,'classvar_check_fact_extractors',lambda root:{module.CLASSVAR_CHECK_EXTRACTOR.extractor_id:lambda path,source:Collector(path)})

    def project(issue):
        raw=dict(file=issue.file.as_posix(),line=issue.line,col=issue.col,message=issue.message,code=issue.code)
        rows=backend.native(CHECK,'_classvar_project',[dict(path='classvar-values.json',source=json.dumps(dict(values=[raw]),ensure_ascii=False))])
        if len(rows)!=1 or rows[0]['code']!='CV_PROJECT':raise ValueError('Missing ClassVar projection')
        return json.loads(rows[0]['parameters']['value_json'])
    backend.replace(module,'_issue_to_fact',project)

    def plugin(root):
        root=Path(root)
        rows=backend.native(CHECK,'_classvar_plugin_metadata',[],root=root)
        if any(r['code']!='CV_PLUGIN_DIR' for r in rows):raise ValueError('Unexpected ClassVar scope row')
        scope=json.loads(config['file_scope'])
        scope['directories']=tuple(root/r['parameters']['path'] for r in sorted(rows,key=lambda r:(r['parameters']['rank'],r['parameters']['path'])))
        scope['explicit_files']=tuple(Path(p) for p in scope['explicit_files'])
        def report(view):
            bundles=[]
            for bundle in view.files.values():
                families={name:dict(kind=kind(value),rows=[dict(kind=kind(row),fields=[dict(key=str(k),encoded=encode(v)) for k,v in members(row)]) for _,row in members(value)]) for name,value in bundle.facts.items()}
                bundles.append(dict(path=str(bundle.path),facts=families))
            rows=backend.native(CHECK,'_report_facts',[dict(path='reporter-facts.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False,allow_nan=False))])
            if any(r['code'] not in {'SF_ROOT','SF_REPORT'} for r in rows):raise ValueError('Unexpected ClassVar supplied-fact response')
            ordered=sorted((r['parameters'] for r in rows if r['code']=='SF_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal']))
            return [module.StructuredViolation(CHECK,p['source_code'],Path(p['file']),json.loads(p['line_json']),p['source_message'],p['severity']) for p in ordered]
        return module.LinterPlugin(**json.loads(config['plugin']),file_scope=module.FileScopeSpec(**scope),required_facts=(module.CLASSVAR_CHECK_EXTRACTOR,),reporter=report)
    backend.replace(module,'classvar_check_plugin',plugin)
    from classvar_cli_port import install as install_cli
    install_cli(backend,module,config)
    backend.replace(module,'_issues_for_files',lambda root,files:batch(backend,module,root,files))
    if detectors:
        for entry in ['lint_file','lint_file_content','lint_directory']:
            def bind(entry):
                if entry=='lint_file':return lambda filepath:scan(backend,entry,module,dict(filepath=filepath))
                if entry=='lint_directory':return lambda dirpath,exclude_patterns=None:scan(backend,entry,module,dict(dirpath=dirpath,exclude_patterns=exclude_patterns))
                return lambda filepath,source,*,root=None:scan(backend,entry,module,dict(filepath=filepath,source=source,root=root))
            backend.replace(module,entry,bind(entry))


def batch(backend,module,root,files,gate_sources=None):
    root=Path(root);paths=[Path(p) if Path(p).is_absolute() else root/Path(p) for p in files]
    scopes=[str(p.relative_to(root)) if p.is_relative_to(root) else str(p) for p in paths]
    request=dict(cvCandidatePaths=[str(p) for p in paths],cvScopePaths=scopes,cvCandidateExists=[str(int(p.is_file())) for p in paths],
                 cvNeedsSourceGate='0' if gate_sources is None else '1',cvCallSources=list(gate_sources or []))
    prepared=backend.native(CHECK,'_classvar_prepare',[],request)
    if any(r['code']!='CV_READ' for r in prepared):raise ValueError('Unexpected ClassVar read plan')
    files=[];owners={};logical=[]
    for p in sorted((r['parameters'] for r in prepared),key=lambda p:p['ordinal']):
        i=p['ordinal'];key=f'classvar-input-{i}.py';owners[key]=(i,paths[i]);logical.append(scopes[i])
        files.append(dict(path=key,source=paths[i].read_text(encoding='utf-8')))
    rows=backend.native(CHECK,'_issues_for_files',files,dict(cvDocuments=[f['path'] for f in files],cvLogicalPaths=logical))
    if any(r['code']!='CLASSVAR001' for r in rows):raise ValueError('Unexpected ClassVar batch finding')
    return [module.LintIssue(owners[r['path']][1],r['line'],r['parameters']['column'],r['message'],r['code'])
            for r in sorted(rows,key=lambda r:(owners[r['path']][0],r['parameters']['collector_visit'],r['parameters']['ordinal']))]


def scan(backend,entry,module,arguments):
    if entry=='_issues_for_files':return batch(backend,module,arguments['root'],arguments['files'])
    if entry=='lint_directory':
        import os
        root=Path(arguments['dirpath']);candidates=[];relative=[]
        for directory,_names,filenames in os.walk(root):
            for filename in filenames:
                path=Path(directory)/filename;candidates.append(str(path));relative.append(str(path.relative_to(root)))
        rows=backend.native(CHECK,'_classvar_walk_files',[],dict(cvWalkPaths=candidates,cvWalkRelative=relative,cvExclude=list(arguments.get('exclude_patterns') or [])))
        if any(r['code']!='CV_WALK_FILE' for r in rows):raise ValueError('Unexpected ClassVar directory inventory')
        paths=[Path(r['parameters']['path']) for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]
        return batch(backend,module,Path.cwd(),paths,[p.read_text() for p in paths])
    path=Path(arguments['filepath']);root=arguments.get('root') or Path.cwd()
    supplied=path.read_text() if entry=='lint_file' else arguments['source']
    return batch(backend,module,root,[path],[supplied])
