"""Transport admin configuration and passive plugin DTOs from native rules."""
import json
import builtins
from dataclasses import fields
from pathlib import Path
import re
from typed_fact_transport import tagged

CHECK='admin_async_purity'


def install(backend,module,detectors=False):
    rows=backend.native(CHECK,'_admin_configuration',[])
    if not rows or any(r['code']!='ADMIN_CONFIG' for r in rows):raise ValueError('Missing native admin configuration')
    config={r['parameters']['key']:json.loads(r['parameters']['value']) for r in rows}
    for name,value in config['constants'].items():backend.replace(module,name,value)
    backend.replace(module,'FORBIDDEN_PATTERNS',[(re.compile(p['pattern'],p['flags']),p['name']) for p in config['patterns']])
    backend.replace(module,'SKIP_PATTERNS',[re.compile(p['pattern'],p['flags']) for p in config['skips']])
    spec=dict(config['extractor'])
    for name in ['source_triggers','project_input_facts']:spec[name]=tuple(spec[name])
    spec['source_trigger_groups']=tuple(tuple(group) for group in spec['source_trigger_groups'])
    backend.replace(module,'ADMIN_ASYNC_PURITY_EXTRACTOR',type(module.ADMIN_ASYNC_PURITY_EXTRACTOR)(**spec))
    if hasattr(backend,'configuration_bindings'):
        backend.configuration_bindings[CHECK]=dict(nativeCall=len(backend.calls)-1,names=[*config['constants'],'FORBIDDEN_PATTERNS','SKIP_PATTERNS','ADMIN_ASYNC_PURITY_EXTRACTOR'])

    def project(value):
        raw={f.name:getattr(value,f.name) for f in fields(value)}
        rows=backend.native(CHECK,'_admin_to_fact',[dict(path='admin-project.json',source=json.dumps(dict(rows=[raw]),ensure_ascii=False,allow_nan=False))])
        if len(rows)!=1 or rows[0]['code']!='ADMIN_VALUE':raise ValueError('One native admin projection required')
        return json.loads(rows[0]['parameters']['value_json'])
    backend.replace(module,'_violation_to_fact',project)

    def registry(root):
        def extract(path,source):
            logical=str(path.relative_to(root));buffer='l8000_admin/tools/_admin-source.py'
            rows=backend.native(CHECK,'_admin_extract',[dict(path=buffer,source=source)],
                dict(adminDocuments=[buffer],adminPaths=[logical],allowlistedFiles=list(module.ALLOWLISTED_FILES)))
            if any(r['code']!='AAP001' for r in rows):raise ValueError('Unexpected native admin extraction output')
            ordered=sorted(rows,key=lambda r:(r['line'],r['parameters']['ordinal']))
            return dict(violations=[dict(file=r['path'],line=r['line'],pattern=r['parameters']['pattern'],text=r['parameters']['text']) for r in ordered])
        return {module.ADMIN_ASYNC_PURITY_EXTRACTOR.extractor_id:extract}
    backend.replace(module,'admin_async_purity_fact_extractors',registry)

    def report(view):
        family=module.ADMIN_ASYNC_PURITY_EXTRACTOR.extractor_id
        bundles=[dict(value=tagged(bundle.facts.get(family))) for bundle in view.files.values()]
        rows=backend.native(CHECK,'_report_facts',[dict(path='admin-values.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False,allow_nan=False))],
            dict(adminReportRoot=str(module.ROOT),adminReportCwd=str(Path.cwd())))
        errors=sorted((r['parameters'] for r in rows if r['code']=='ADMIN_REPORT_ERROR'),key=lambda p:(p['bundle'],p['ordinal'],p['stage']))
        if errors:raise getattr(builtins,errors[0]['error_type'])(errors[0]['detail'])
        if any(r['code'] not in {'SF_ROOT','SF_REPORT'} for r in rows):raise ValueError('Unexpected native admin report output')
        ordered=sorted((r['parameters'] for r in rows if r['code']=='SF_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal']))
        return [module.StructuredViolation(CHECK,p['source_code'],Path(p['file']),json.loads(p['line_json']),p['source_message'],p['severity']) for p in ordered]
    backend.replace(module,'_report_admin_async_purity',report)

    def plugin(root):
        scope=dict(config['scope'])
        scope['directories']=tuple(root/p for p in scope['directories'])
        scope['explicit_files']=tuple(Path(p) for p in scope['explicit_files'])
        return module.LinterPlugin(**config['plugin'],file_scope=module.FileScopeSpec(**scope),
            required_facts=(module.ADMIN_ASYNC_PURITY_EXTRACTOR,),reporter=module._report_admin_async_purity)
    backend.replace(module,'admin_async_purity_plugin',plugin)
    from admin_cli_port import install as install_cli
    install_cli(backend,module,config)
    if detectors:
        from admin_public_port import install as install_public
        install_public(backend,module)
