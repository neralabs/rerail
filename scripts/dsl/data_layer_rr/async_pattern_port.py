"""Transport async-pattern inputs and passive DTOs; execute decisions in native rules."""
from dataclasses import fields
from copy import deepcopy
import json
from pathlib import Path
from ast_transport import syntax_columns
from typed_fact_transport import encode_bundles

CHECK='async_patterns'


def query(backend,entry,files=(),options=None):
    return backend.native(CHECK,entry,list(files),options or {})


def values(rows):
    errors=sorted((r['parameters'] for r in rows if r['code']=='AP_VALUE_ERROR'),key=lambda p:(p['bundle'],p['ordinal']))
    if errors:
        p=errors[0];raise {'OverflowError':OverflowError}[p['error_type']](p['detail'])
    if any(r['code']!='AP_VALUE' for r in rows):raise ValueError('Unexpected async-pattern value')
    return [json.loads(r['parameters']['value_json']) for r in sorted(rows,key=lambda r:(r['parameters']['bundle'],r['parameters']['ordinal']))]


def violation(module,value):
    if value is None:return None
    value=dict(value);value['file']=Path(value['file']);return module.Violation(**value)


def install(backend,module,detectors=False):
    rows=query(backend,'_ap_configuration')
    if not rows or any(r['code']!='AP_CONFIG' for r in rows):raise ValueError('Missing async-pattern configuration')
    config={r['parameters']['key']:json.loads(r['parameters']['value']) for r in rows}
    for name,spec in config['constants'].items():
        value=spec['value']
        if spec['kind']=='tuple':value=tuple(value)
        if spec['kind']=='frozenset':value=frozenset(value)
        backend.replace(module,name,value)
    spec=config['extractor']
    for name in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']:spec[name]=tuple(spec[name])
    backend.replace(module,'ASYNC_PATTERNS_EXTRACTOR',type(module.ASYNC_PATTERNS_EXTRACTOR)(**spec))
    if hasattr(backend,'configuration_bindings'):backend.configuration_bindings[CHECK]=dict(nativeCall=len(backend.calls)-1,names=[*config['constants'],'ASYNC_PATTERNS_EXTRACTOR'])

    def collect(owner,tree):
        options=syntax_columns(tree) if tree is not None else {};transport=options.pop('receiverTransport','pass\n')
        options.update(providedTree=str(tree is not None).lower(),providedSource=owner.source,sourceLines=owner.source.splitlines(),originalPath=str(owner.filepath),analysisPath='input.py',apOwnerRoot=str(owner.root))
        rows=query(backend,'_ap_ast_collect',[dict(path='api/_receiver_input.py',source=transport)],options)
        facts=values(rows);owner._facts=facts
        owner.violations=[module.Violation(owner.filepath,p['line'],p['code'],p['message'],p['context'],p['is_fallback']) for p in facts]
        return facts

    class Collector(module.AstEventCollector):
        def __init__(self,filepath,source,root):
            self.filepath=filepath;self.root=root;self.source=source;self.source_lines=source.splitlines();self.tree=None;self._facts=None
            for key,value in deepcopy(config['collector_defaults']).items():setattr(self,key,value)
            context=query(backend,'_ap_initial',options=dict(originalPath=str(filepath)))
            if len(context)!=4 or any(r['code']!='AP_INITIAL' for r in context):raise ValueError('Missing native async collector context')
            for row in context:setattr(self,row['parameters']['key'],bool(row['parameters']['value']))
        def on_node(self,node,context):
            if self.tree is None:
                while context.parent(node) is not None:node=context.parent(node)
                self.tree=node
        def finish_file(self):
            if self._facts is None:
                collect(self,self.tree)
            return self._facts
    from async_pattern_helpers_port import install as install_helpers
    install_helpers(backend,module,Collector)
    backend.replace(module,'AsyncPatternCollector',Collector)
    backend.replace(module,'async_patterns_fact_extractors',lambda root:{module.ASYNC_PATTERNS_EXTRACTOR.extractor_id:lambda path,source:Collector(path,source,root)})
    def collect_tree(tree,owner,_context):
        owner.tree=tree;collect(owner,tree)
    backend.replace(module,'_collect_async_pattern_node',collect_tree)

    def from_fact(raw,root):
        bundles=encode_bundles([dict(path='',value=[raw])])
        result=values(query(backend,'_ap_from_fact',[dict(path='async-pattern-values.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False,allow_nan=False))],dict(apOwnerRoot=str(root))))
        if len(result)!=1:raise ValueError('One converted async-pattern value required')
        return violation(module,result[0])
    def to_fact(value,root):
        raw={f.name:getattr(value,f.name) for f in fields(value)};raw['file']=str(value.file)
        result=values(query(backend,'_ap_to_fact',[dict(path='async-pattern-project.json',source=json.dumps(dict(rows=[raw]),ensure_ascii=False))],dict(apOwnerRoot=str(root))))
        if len(result)!=1:raise ValueError('One projected async-pattern value required')
        return result[0]
    backend.replace(module,'_violation_from_fact',from_fact);backend.replace(module,'_violation_to_fact',to_fact)

    def plugin(root):
        scope=dict(config['scope']);scope['directories']=tuple(root/p for p in scope['directories']);scope['explicit_files']=tuple(Path(p) for p in scope['explicit_files'])
        def report(view):
            family=module.ASYNC_PATTERNS_EXTRACTOR.extractor_id
            bundles=encode_bundles([dict(path=str(i),value=b.facts.get(family,[])) for i,b in enumerate(view.files.values())])
            rows=query(backend,'_report_facts',[dict(path='async-pattern-values.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False,allow_nan=False))],dict(apOwnerRoot=str(root)))
            errors=[r for r in rows if r['code']=='AP_VALUE_ERROR']
            if errors:values(errors)
            if any(r['code'] not in {'SF_ROOT','SF_REPORT'} for r in rows):raise ValueError('Unexpected async-pattern raw reporter value')
            ordered=sorted((r['parameters'] for r in rows if r['code']=='SF_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal']))
            return [module.StructuredViolation(CHECK,p['source_code'],Path(p['file']),json.loads(p['line_json']),p['source_message'],p['severity']) for p in ordered]
        return module.LinterPlugin(**config['plugin'],file_scope=module.FileScopeSpec(**scope),required_facts=(module.ASYNC_PATTERNS_EXTRACTOR,),reporter=report)
    backend.replace(module,'async_patterns_plugin',plugin)
    from async_pattern_files_port import install as install_files
    install_files(backend,module,config)
    from async_pattern_cli_port import install as install_cli
    install_cli(backend,module,config)
    if detectors:
        import inspect
        from async_patterns_port import scan
        for entry in ['analyze_source','lint_file','should_exclude']:
            signature=inspect.signature(getattr(module,entry))
            def bind(entry,signature):
                def run(*args,**kwargs):
                    bound=signature.bind(*args,**kwargs);bound.apply_defaults();return scan(backend,entry,module,bound.arguments)
                return run
            backend.replace(module,entry,bind(entry,signature))
