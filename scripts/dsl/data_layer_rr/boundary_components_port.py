"""Transport boundary inputs and passive DTOs; run all linter decisions in native rules."""
from copy import deepcopy
from dataclasses import fields
import inspect
import json
from pathlib import Path
from ast_transport import scalar_columns
from typed_fact_transport import encode_bundles

CHECK='async_sync_boundary'


def query(backend,entry,files=(),options=None):return backend.native(CHECK,entry,list(files),options or {})


def values(rows):
    errors=sorted((r['parameters'] for r in rows if r['code']=='BS_VALUE_ERROR'),key=lambda p:(p['bundle'],p['ordinal'],p['stage']))
    if errors:raise OverflowError(errors[0]['detail'])
    if any(r['code']!='BS_VALUE' for r in rows):raise ValueError('Unexpected native boundary value')
    return [json.loads(r['parameters']['value_json']) for r in sorted(rows,key=lambda r:(r['parameters']['bundle'],r['parameters']['ordinal']))]


def install(backend,module,detectors=False):
    rows=query(backend,'_bs_configuration')
    if not rows or any(r['code']!='BS_CONFIG' for r in rows):raise ValueError('Missing native boundary configuration')
    config={r['parameters']['key']:json.loads(r['parameters']['value']) for r in rows}
    for name,spec in config['constants'].items():
        value=spec['value']
        if spec['kind']=='tuple':value=tuple(value)
        if spec['kind']=='set':value={tuple(row) for row in value}
        backend.replace(module,name,value)
    spec=config['extractor']
    for key in ['source_triggers','project_input_facts','node_kinds']:spec[key]=tuple(spec[key])
    spec['source_trigger_groups']=tuple(tuple(row) for row in spec['source_trigger_groups'])
    backend.replace(module,'ASYNC_SYNC_BOUNDARY_EXTRACTOR',type(module.ASYNC_SYNC_BOUNDARY_EXTRACTOR)(**spec))
    if hasattr(backend,'configuration_bindings'):backend.configuration_bindings[CHECK]=dict(nativeCall=len(backend.calls)-1,names=[*config['constants'],'ASYNC_SYNC_BOUNDARY_EXTRACTOR'])

    class Collector(module.AstEventCollector):
        def __init__(self,file_path):
            self.file_path=file_path;self._tree=None;self._facts=None
            for name,value in deepcopy(config['collector_defaults']).items():setattr(self,name,value)
        def on_node(self,node,context):
            if self._tree is None:
                while context.parent(node) is not None:node=context.parent(node)
                self._tree=node
        def _current_function(self):
            options=dict(bsStackNames=[json.dumps(v[0],ensure_ascii=False) for v in self._func_stack],bsStackFlags=[json.dumps(v[1]) for v in self._func_stack])
            result=values(query(backend,'_bs_current_function',options=options))
            if len(result)!=1:raise ValueError('One current boundary function required')
            return tuple(result[0])
        def finish_file(self):
            if self._facts is None:
                options=dict(scalar_columns(self._tree) if self._tree is not None else {},bsCollectorPath=self.file_path)
                self._facts=values(query(backend,'_bs_ast_collect',options=options))
                self.violations=[module.Violation(**v) for v in self._facts]
            return self._facts
    backend.replace(module,'_BoundaryCollector',Collector)
    backend.replace(module,'async_sync_boundary_fact_extractors',lambda root:{module.ASYNC_SYNC_BOUNDARY_EXTRACTOR.extractor_id:lambda path,_source:Collector(str(path.relative_to(root)))})

    def from_fact(raw):
        result=values(query(backend,'_bs_from_fact',[dict(path='boundary-values.json',source=json.dumps(dict(bundles=encode_bundles([dict(path='',value=[raw])])),ensure_ascii=False,allow_nan=False))]))
        if len(result)!=1:raise ValueError('One converted boundary value required')
        return None if result[0] is None else module.Violation(**result[0])
    def to_fact(value):
        raw={f.name:getattr(value,f.name) for f in fields(value)}
        result=values(query(backend,'_bs_to_fact',[dict(path='boundary-project.json',source=json.dumps(dict(rows=[raw]),ensure_ascii=False))]))
        if len(result)!=1:raise ValueError('One projected boundary value required')
        return result[0]
    backend.replace(module,'_violation_from_fact',from_fact);backend.replace(module,'_violation_to_fact',to_fact)

    def plugin(root):
        scope=dict(config['scope'])
        scope['directories']=tuple(root/p for p in scope['directories']);scope['explicit_files']=tuple(Path(p) for p in scope['explicit_files'])
        def report(view):
            family=module.ASYNC_SYNC_BOUNDARY_EXTRACTOR.extractor_id
            bundles=encode_bundles([dict(path=str(i),value=b.facts.get(family,[])) for i,b in enumerate(view.files.values())])
            rows=query(backend,'_report_facts',[dict(path='boundary-values.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False,allow_nan=False))],dict(bsOwnerRoot=str(root)))
            errors=[r for r in rows if r['code']=='BS_VALUE_ERROR']
            if errors:values(errors)
            if any(r['code'] not in {'SF_ROOT','SF_REPORT'} for r in rows):raise ValueError('Unexpected boundary reporter result')
            ordered=sorted((r['parameters'] for r in rows if r['code']=='SF_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal']))
            return [module.StructuredViolation(CHECK,p['source_code'],Path(p['file']),json.loads(p['line_json']),p['source_message'],p['severity']) for p in ordered]
        return module.LinterPlugin(**config['plugin'],file_scope=module.FileScopeSpec(**scope),required_facts=(module.ASYNC_SYNC_BOUNDARY_EXTRACTOR,),reporter=report)
    backend.replace(module,'async_sync_boundary_plugin',plugin)
    from boundary_port import install as install_properties,scan
    install_properties(backend,module)
    from boundary_display_port import install as install_display
    install_display(backend,module)
    from boundary_cli_port import install as install_cli
    install_cli(backend,module,config)
    if detectors:
        for entry in ['scan_tree','_get_call_name','_is_allowlisted','_is_informational']:
            signature=inspect.signature(getattr(module,entry))
            def bind(entry,signature):
                def run(*args,**kwargs):
                    bound=signature.bind(*args,**kwargs);bound.apply_defaults();return scan(backend,entry,module,bound.arguments)
                return run
            backend.replace(module,entry,bind(entry,signature))
