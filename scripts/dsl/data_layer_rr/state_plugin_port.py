"""Transport native plugin metadata, supplied Python values, and public DTOs."""
import importlib
import json
from pathlib import Path
import struct
import sys

CHECK='serverplan_state_tripwires'


def encode_view(view):
    values=[]
    def encode(value,parent=-1,key='',ordinal=0):
        n=len(values);kind=type(value).__name__
        text=value if isinstance(value,str) else str(value) if type(value) is int else (
            'true' if value else 'false') if type(value) is bool else (
            struct.pack('>d',value).hex() if type(value) is float else '')
        values.append(dict(kind=kind,text=text,parent=parent,key_kind=type(key).__name__,
                           key=key if isinstance(key,str) else '',ordinal=ordinal))
        if isinstance(value,(list,tuple)):
            for i,child in enumerate(value):encode(child,n,'',i)
        elif isinstance(value,dict):
            for i,(k,child) in enumerate(value.items()):encode(child,n,k,i)
        return n
    bundles=[]
    for bundle in view.files.values():
        bundles.append(dict(families=[dict(name=name,root=encode(value)) for name,value in bundle.facts.items()]))
    return dict(bundles=bundles,values=values)


def configuration(module):
    production=importlib.import_module('l2070_batching.production_ir_allowlist')
    path=Path(module.ALLOWLIST_PATH).absolute()
    return dict(moduleRoot=module.ROOT.as_posix(),allowlistPath=path.as_posix(),
        generatedPath=module.GENERATED_RETURN_KINDS_PATH.as_posix(),linterPath=Path(module.__file__).as_posix(),
        productionIrEntries=list(production.ALLOWED_PRODUCTION_IR_TYPES),productionIrSignature=production.ALLOWLIST_SIGNATURE,
        nativeWorkEntries=list(module.SERVERPLAN_NATIVE_PYTHON_WORK_ALLOWLIST),
        preprocessEntries=list(module.SERVERPLAN_PREPROCESS_RUNTIME_VALUE_ALLOWLIST),
        callbackEntries=list(module.SERVERPLAN_RUNTIME_CALLBACK_ALLOWLIST),pythonWorkSignature=module.PYTHON_WORK_ALLOWLIST_SIGNATURE,
        env=['state.allowlist='+path.parent.as_posix(),'state.plugin.allowlist='+path.as_posix()])


def raise_read_error(p):
    if p['io_kind']=='InvalidUtf8':
        reason={'invalid-start':'invalid start byte','invalid-continuation':'invalid continuation byte','unexpected-end':'unexpected end of data'}[p['reason']]
        raise UnicodeDecodeError('utf-8',bytes.fromhex(p['bytes_hex']),p['start'],p['end'],reason)
    raise OSError(p['errno'],__import__('os').strerror(p['errno']),p['path'])


def install(backend,module):
    def plugin(root):
        rows=backend.native(CHECK,'_state_plugin_metadata',[],dict(env=['state.plugin.metadata=true']),root=Path(root))
        if any(r['code'] not in {'SPT_PLUGIN_META','SPT_PLUGIN_DIR','SPT_PLUGIN_RETURN_KIND','SPT_PLUGIN_RETURN_NAME'} for r in rows):raise ValueError('Unexpected native State plugin metadata')
        meta=[r['parameters']['json'] for r in rows if r['code']=='SPT_PLUGIN_META']
        if len(meta)!=1:raise ValueError('One State plugin descriptor required')
        data=json.loads(meta[0]);scope=data['scope']
        scope['directories']=tuple(Path(r['parameters']['path']) for r in sorted(
            (r for r in rows if r['code']=='SPT_PLUGIN_DIR'),key=lambda r:r['parameters']['ordinal']))
        scope['explicit_files']=tuple(Path(p) for p in scope['explicit_files'])
        specs=[]
        for config in data['extractors']:
            for key in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']:
                if key in config:config[key]=tuple(config[key])
            specs.append(module.AstCollectorSpec(**config))
        # This is the factory's published collector configuration, supplied by
        # the existing native generated-return-kind relation.
        generated={r['parameters']['name']:set() for r in sorted(
            (r for r in rows if r['code']=='SPT_PLUGIN_RETURN_NAME'),key=lambda r:r['parameters']['ordinal'])}
        for row in rows:
            if row['code']=='SPT_PLUGIN_RETURN_KIND':
                p=row['parameters'];generated.setdefault(p['name'],set()).add(p['kind'])
        module.ServerPlanStateTripwireCollector.project_return_kinds={name:frozenset(kinds) for name,kinds in generated.items()}
        def report(view):
            payload=json.dumps(encode_view(view),ensure_ascii=False,allow_nan=False)
            rows=backend.native(CHECK,'_state_plugin_report',[dict(path='state-plugin-facts.json',source=payload)],configuration(module))
            known={'SPT_PLUGIN_REPORT','SPT_PLUGIN_STDERR','SPT_PLUGIN_FACT_ERROR','SPT_PLUGIN_IO_ERROR'}
            if any(r['code'] not in known for r in rows):raise ValueError('Unexpected native State plugin response')
            for p in sorted((r['parameters'] for r in rows if r['code']=='SPT_PLUGIN_STDERR'),key=lambda p:p['ordinal']):sys.stderr.write(p['text'])
            errors=[r for r in rows if r['code']=='SPT_PLUGIN_FACT_ERROR']
            if errors:
                if len(errors)!=1:raise ValueError('One first native fact error required')
                raise TypeError(errors[0]['parameters']['detail'])
            errors=[r for r in rows if r['code']=='SPT_PLUGIN_IO_ERROR']
            if errors:
                if len(errors)!=1:raise ValueError('One native allowlist read error required')
                raise_read_error(errors[0]['parameters'])
            ordered=sorted((r['parameters'] for r in rows if r['code']=='SPT_PLUGIN_REPORT'),
                key=lambda p:tuple(p[key] for key in ['phase','group_order','bundle','ordinal']))
            return [module.StructuredViolation(CHECK,p['source_code'],Path(p['path']),json.loads(p['line_json']),p['message']) for p in ordered]
        return module.LinterPlugin(**data['plugin'],file_scope=module.FileScopeSpec(**scope),required_facts=tuple(specs),reporter=report)
    backend.replace(module,'serverplan_state_tripwires_plugin',plugin)
