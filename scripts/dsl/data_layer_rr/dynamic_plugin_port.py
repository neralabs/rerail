"""Transport plugin metadata, fact values and passive public DTOs."""
import json
from pathlib import Path
from fact_transport import kind,members
from python_value_transport import encode
from dynamic_cli_port import one,emit

CHECK='dynamic_module_loading'


def fact_rows(rows):
    if any(r['code']!='DML_FACT' for r in rows):raise ValueError('Unexpected native extraction response')
    result=[]
    for p in sorted((r['parameters'] for r in rows),key=lambda p:p['ordinal']):
        result.append(dict(file_path=p['file_path'],line=json.loads(p['line_json']),line_content=json.loads(p['content_json']),description=json.loads(p['description_json'])))
    return result


def install(backend,module):
    def project(value):
        path,line,content,description=value
        rows=backend.native(CHECK,'_dynamic_fact_projection',[],dict(factPaths=[path.as_posix()],factLines=[json.dumps(line)],factContents=[json.dumps(content)],factDescriptions=[json.dumps(description)]))
        facts=fact_rows(rows)
        if len(facts)!=1:raise ValueError('One native fact projection required')
        return facts[0]
    def registry(root):
        def extract(path,source):
            logical=path.relative_to(root).as_posix()
            rows=backend.native(CHECK,'_dynamic_extract',[dict(path='source-0.py',source=source)],dict(allowlistedFiles=list(module.ALLOWLISTED_FILES),dmlDocuments=['source-0.py'],dmlPaths=[logical],factPath=path.as_posix()))
            return fact_rows(rows)
        return {module.DYNAMIC_MODULE_LOADING_EXTRACTOR.extractor_id:extract}
    def plugin(root):
        root=Path(root)
        rows=backend.native(CHECK,'_dynamic_plugin_metadata',[],dict(env=['dml.plugin=true']),root=root)
        if any(r['code'] not in {'DML_PLUGIN_META','DML_PLUGIN_DIR'} for r in rows):raise ValueError('Unexpected native plugin metadata')
        metadata=json.loads(one(rows,'DML_PLUGIN_META','json'));scope=metadata['scope']
        scope['directories']=tuple(root/r['parameters']['path'] for r in sorted((r for r in rows if r['code']=='DML_PLUGIN_DIR'),key=lambda r:r['parameters']['ordinal']))
        scope['explicit_files']=tuple(Path(p) for p in scope['explicit_files'])
        def report(view):
            values=backend.native(CHECK,'_dynamic_cli_start',[],dict(cliSign='0',cliAllowed=list(module.ALLOWLISTED_FILES)))
            entries=[r['parameters']['value'] for r in sorted((r for r in values if r['code']=='DML_CLI_ALLOW'),key=lambda r:r['parameters']['ordinal'])]
            verified=module.verify_allowlist(module.LINTER_NAME,entries,module.ALLOWLIST_SIGNATURE)
            decision=backend.native(CHECK,'_dynamic_plugin_verified',[],dict(cliResult=str(int(bool(verified)))))
            emit([r for r in decision if r['code']!='DML_PLUGIN_ERROR'])
            errors=[r for r in decision if r['code']=='DML_PLUGIN_ERROR']
            if errors:
                if len(errors)!=1:raise ValueError('One native signature failure required')
                p=errors[0]['parameters'];return [module.StructuredViolation(module.LINTER_NAME,p['code_value'],Path(p['path']),p['line'],p['message'],p['severity'])]
            if one(decision,'DML_CLI_ACTION','value')!='facts':raise ValueError('Unexpected native plugin action')
            bundles=[]
            for bundle in view.files.values():
                families={}
                for name,value in bundle.facts.items():
                    families[name]=dict(kind=kind(value),rows=[dict(kind=kind(row),fields=[dict(key=str(k),encoded=encode(v)) for k,v in members(row)]) for _,row in members(value)])
                bundles.append(dict(path=str(bundle.path),facts=families))
            rows=backend.native(CHECK,'_report_facts',[dict(path='reporter-facts.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False,allow_nan=False))])
            if any(r['code'] not in {'SF_ROOT','SF_REPORT'} for r in rows):raise ValueError('Unexpected supplied fact response')
            ordered=sorted((r['parameters'] for r in rows if r['code']=='SF_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal']))
            return [module.StructuredViolation(module.LINTER_NAME,p['source_code'],Path(p['file']),json.loads(p['line_json']),p['source_message'],p['severity']) for p in ordered]
        return module.LinterPlugin(**dict(metadata['plugin'],name=module.LINTER_NAME,file_scope=module.FileScopeSpec(**scope),required_facts=(module.DYNAMIC_MODULE_LOADING_EXTRACTOR,),reporter=report))
    backend.replace(module,'_violation_to_fact',project);backend.replace(module,'dynamic_module_loading_fact_extractors',registry);backend.replace(module,'dynamic_module_loading_plugin',plugin)
