"""Transport admin configuration and passive plugin DTOs from native rules."""
import json
from pathlib import Path
import re

CHECK='admin_async_purity'


def install(backend,module,detectors=False):
    rows=backend.native(CHECK,'_admin_configuration',[])
    if not rows or any(r['code']!='ADMIN_CONFIG' for r in rows):raise ValueError('Missing native admin configuration')
    config={r['parameters']['key']:json.loads(r['parameters']['value']) for r in rows}
    for name,value in config['constants'].items():backend.replace(module,name,value)
    backend.replace(module,'FORBIDDEN_PATTERNS',[(re.compile(p['pattern'],p['flags']),p['name']) for p in config['patterns']])
    backend.replace(module,'SKIP_PATTERNS',[re.compile(p['pattern'],p['flags']) for p in config['skips']])
    spec=dict(config['extractor'])
    for name in ['source_triggers','project_input_facts','node_kinds']:spec[name]=tuple(spec[name])
    spec['source_trigger_groups']=tuple(tuple(group) for group in spec['source_trigger_groups'])
    backend.replace(module,'ADMIN_ASYNC_PURITY_EXTRACTOR',type(module.ADMIN_ASYNC_PURITY_EXTRACTOR)(**spec))
    if hasattr(backend,'configuration_bindings'):
        backend.configuration_bindings[CHECK]=dict(nativeCall=len(backend.calls)-1,names=[*config['constants'],'FORBIDDEN_PATTERNS','SKIP_PATTERNS','ADMIN_ASYNC_PURITY_EXTRACTOR'])

    def plugin(root):
        scope=dict(config['scope'])
        scope['directories']=tuple(root/p for p in scope['directories'])
        scope['explicit_files']=tuple(Path(p) for p in scope['explicit_files'])
        return module.LinterPlugin(**config['plugin'],file_scope=module.FileScopeSpec(**scope),
            required_facts=(module.ADMIN_ASYNC_PURITY_EXTRACTOR,),reporter=module._report_admin_async_purity)
    backend.replace(module,'admin_async_purity_plugin',plugin)
