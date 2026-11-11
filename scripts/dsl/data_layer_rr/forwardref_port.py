"""Transport ForwardRef inputs and native DTOs; execute decisions in .rr."""
from functools import wraps
import inspect
import json
from pathlib import Path
from types import SimpleNamespace
from ast_transport import columns
from typed_fact_transport import encode_bundles
from forwardref_cli_port import install as install_cli

CHECK='forwardref'


def collect(backend,path,source,root):
    rows=backend.native(CHECK,'_fr_collect',[dict(path='impl/collector.py',source=source)],dict(frInputDirs=['impl'],frCollectPath=Path(path).as_posix(),frCollectName=Path(path).name,frFactRoot=Path(root).as_posix()))
    if any(r['code'] not in {'FR_COLLECT_FACT','FR_COLLECT_STATE','FR_COLLECT_STATUS','FR_COLLECT_ERROR','FWDREF001','FWDREF002','FWDREF003'} for r in rows):raise ValueError('Unexpected ForwardRef collector response')
    return rows


def extract(backend,module,path,source,root):
    rows=collect(backend,path,source,root);present=[r['parameters']['present'] for r in rows if r['code']=='FR_COLLECT_STATUS']
    if len(present)!=1:raise ValueError('One ForwardRef parse status required')
    facts=[dict(file=p['file'],line=p['line_number'],col=p['column'],message=p['source_message'],code=p['source_code']) for r in sorted((r for r in rows if r['code']=='FR_COLLECT_FACT'),key=lambda r:r['parameters']['ordinal']) for p in [r['parameters']]]
    family=module.FORWARDREF_EXTRACTOR.extractor_id;errors=[r['parameters']['value'] for r in rows if r['code']=='FR_COLLECT_ERROR']
    return SimpleNamespace(facts={family:facts} if present[0] else {},errors={family:errors[0]} if errors else {})


def batch(backend,module,root,paths,*,entry='_issues_for_files',source=None,exclude=None):
    root=Path(root);paths=[p if p.is_absolute() else root/p for p in map(Path,paths)]
    scopes=[p.relative_to(root).as_posix() if p.is_relative_to(root) else p.as_posix() for p in paths]
    options=dict(frPaths=[str(p) for p in paths],frScopes=scopes,frExists=[str(int(p.is_file())) for p in paths],frEntry=entry,frExclude=list(exclude or []))
    if source is not None:options['frCallSource']=source
    rows=backend.native(CHECK,'_fr_prepare',[],options)
    if any(r['code']!='FR_READ' for r in rows):raise ValueError('Unexpected ForwardRef preparation response')
    prepared=sorted((r['parameters'] for r in rows),key=lambda r:r['ordinal']);files=[];owners={};logical=[]
    for p in prepared:
        i=p['ordinal'];key=f'impl/forwardref-{i}.py';files.append(dict(path=key,source=paths[i].read_text()));owners[key]=(i,paths[i]);logical.append(scopes[i])
    rows=backend.native(CHECK,'_fr_batch',files,dict(frInputDirs=['impl'],logicalPaths=[f['path'] for f in files],scopePaths=logical))
    if any(r['code'] not in {'FWDREF001','FWDREF002','FWDREF003'} for r in rows):raise ValueError('Unexpected ForwardRef diagnostic')
    return [module.LintIssue(owners[r['path']][1],r['line'],r['parameters']['column'],r['message'],r['code']) for r in sorted(rows,key=lambda r:(owners[r['path']][0],r['parameters']['ordinal']))]


def scan(backend,entry,module,arguments):
    path=Path(arguments.get('filepath',arguments.get('dirpath')))
    paths=list(path.rglob('*.py')) if entry=='lint_directory' else [path]
    return batch(backend,module,arguments.get('root') or Path.cwd(),paths,entry=entry,source=arguments.get('source'),exclude=arguments.get('exclude_patterns'))


def install(backend,module,detectors=False):
    rows=backend.native(CHECK,'_fr_configuration',[])
    if not rows or any(r['code']!='FR_CONFIG' for r in rows):raise ValueError('Missing ForwardRef configuration')
    config={r['parameters']['key']:r['parameters']['value'] for r in rows}
    data=json.loads(config['extractor'])
    for name in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']:data[name]=tuple(data[name])
    backend.replace(module,'FORWARDREF_EXTRACTOR',type(module.FORWARDREF_EXTRACTOR)(**data))
    if hasattr(backend,'configuration_bindings'):backend.configuration_bindings[CHECK]=dict(nativeCall=len(backend.calls)-1,names=['FORWARDREF_EXTRACTOR'])
    class Collector(module.AstEventCollector):
        def __init__(self,filepath,owner_root=None):
            self.filepath=Path(filepath);self.root=self.filepath.parent.parent if owner_root is None else owner_root;self.source='';self.issues=[]
            self._in_annotation_context=False;self._has_forwardref_check=False;self._annotation_var=None;self._state_stack=[]
        def start_file(self,path,source,context):self.filepath=path;self.source=source
        def _contains_forwardref(self,node):
            rows=backend.native(CHECK,'ForwardRefCollector._contains_forwardref',[],columns(node))
            if len(rows)!=1 or rows[0]['code']!='FR_CONTAINS':raise ValueError('One native ForwardRef helper result required')
            return bool(rows[0]['parameters']['value'])
        def finish_file(self):
            rows=collect(backend,self.filepath,self.source,self.root)
            states=[r['parameters'] for r in rows if r['code']=='FR_COLLECT_STATE']
            if len(states)!=1:raise ValueError('One native ForwardRef final state required')
            p=states[0];self._in_annotation_context=bool(p['context']);self._has_forwardref_check=bool(p['guarded']);self._annotation_var=p['annotation_var'] if p['context'] else None
            facts=[];self.issues=[]
            for row in sorted((r for r in rows if r['code']=='FR_COLLECT_FACT'),key=lambda r:r['parameters']['ordinal']):
                p=row['parameters'];self.issues.append(module.LintIssue(self.filepath,p['line_number'],p['column'],p['source_message'],p['source_code']))
                facts.append(dict(file=p['file'],line=p['line_number'],col=p['column'],message=p['source_message'],code=p['source_code']))
            return facts
    backend.replace(module,'ForwardRefCollector',Collector)
    backend.replace(module,'forwardref_fact_extractors',lambda root:{module.FORWARDREF_EXTRACTOR.extractor_id:lambda path,source:Collector(path,root)})
    def project(issue,root):
        payload=dict(file=issue.file.as_posix(),line=issue.line,col=issue.col,message=issue.message,code=issue.code)
        rows=backend.native(CHECK,'_issue_to_fact',[dict(path='forwardref-value.json',source=json.dumps(payload,ensure_ascii=False))],dict(frFactRoot=Path(root).as_posix()))
        if len(rows)!=1 or rows[0]['code']!='FR_PROJECT':raise ValueError('One native ForwardRef projection required')
        p=rows[0]['parameters'];return dict(file=p['file'],line=json.loads(p['line_json']),col=json.loads(p['col_json']),message=json.loads(p['message_json']),code=json.loads(p['code_json']))
    backend.replace(module,'_issue_to_fact',project)
    def plugin(root):
        scope=json.loads(config['file_scope']);scope['directories']=tuple(root/p for p in json.loads(config['plugin_dirs']));scope['explicit_files']=tuple(Path(p) for p in scope['explicit_files'])
        def report(view):
            family=module.FORWARDREF_EXTRACTOR.extractor_id
            bundles=encode_bundles([dict(path=str(b.path),source_path=str(b.path),value=b.facts.get(family,[])) for b in view.files.values()])
            rows=backend.native(CHECK,'_report_facts',[dict(path='reporter-facts.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False,allow_nan=False))])
            if any(r['code'] not in {'SF_ROOT','SF_REPORT'} for r in rows):raise ValueError('Unexpected ForwardRef reporter response')
            ordered=sorted((r['parameters'] for r in rows if r['code']=='SF_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal']))
            return [module.StructuredViolation(CHECK,p['source_code'],Path(p['file']),json.loads(p['line_json']),p['source_message'],p['severity']) for p in ordered]
        return module.LinterPlugin(**json.loads(config['plugin']),file_scope=module.FileScopeSpec(**scope),required_facts=(module.FORWARDREF_EXTRACTOR,),reporter=report)
    backend.replace(module,'forwardref_plugin',plugin)
    module._rerail_forwardref_extract=lambda path,source,root:extract(backend,module,path,source,root)
    backend.replace(module,'_issues_for_files',lambda root,files:batch(backend,module,root,files))
    install_cli(backend,module,config)
    if detectors:
        for entry in ['lint_file','lint_file_content','lint_directory']:
            original=getattr(module,entry);signature=inspect.signature(original)
            def bind(entry,original,signature):
                @wraps(original)
                def run(*args,**kwargs):
                    bound=signature.bind(*args,**kwargs);bound.apply_defaults();return scan(backend,entry,module,bound.arguments)
                return run
            backend.replace(module,entry,bind(entry,original,signature))
