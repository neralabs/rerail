"""Transport generated-index inputs and decode native decisions into passive DTOs."""
import argparse
from functools import wraps
import inspect
import json
from pathlib import Path
import sys

CHECK='generated_index_sources'


def ordered(rows):
    return sorted(rows,key=lambda r:(r['parameters']['ordinal'],r['parameters']['rank']))


def extract(suite,module,path,source):
    rows=suite.native(CHECK,'_collector_facts',[dict(path='input/source.py',source=source)],
        dict(apiInputDirs=['input'],apiBuffers=['input/source.py'],apiSourcePaths=[str(path)]))
    if any(r['code'] not in {'GIS_FACT','GIS_COLLECTED','GIS_COLLECTOR_ERROR'} for r in rows):raise ValueError('Unexpected native generated-index extraction')
    facts=[dict(line=r['parameters']['source_line'],message=r['parameters']['source_message']) for r in ordered([r for r in rows if r['code']=='GIS_FACT'])]
    errors=[r['parameters']['value'] for r in rows if r['code']=='GIS_COLLECTOR_ERROR']
    present=[r['parameters']['present'] for r in rows if r['code']=='GIS_COLLECTED']
    if len(present)!=1:raise ValueError('One native collection status required')
    family=module.GENERATED_INDEX_SOURCES_EXTRACTOR.extractor_id
    return SimpleBundle({family:facts} if present[0] else {},{family:errors[0]} if errors else {})


class SimpleBundle:
    def __init__(self,facts,errors):self.facts=facts;self.errors=errors


def install(suite,module,detectors=False):
    rows=suite.native(CHECK,'_gis_configuration',[])
    if not rows or any(r['code']!='GIS_CONFIG' for r in rows):raise ValueError('Missing native generated-index configuration')
    config={r['parameters']['key']:r['parameters']['value'] for r in rows if r['parameters']['key']!='file'}
    paths=tuple(Path(r['parameters']['value']) for r in rows if r['parameters']['key']=='file')
    data=json.loads(config['extractor'])
    for name in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']:data[name]=tuple(data[name])
    suite.replace(module,'GENERATED_INDEX_SOURCES_EXTRACTOR',type(module.GENERATED_INDEX_SOURCES_EXTRACTOR)(**data))
    suite.replace(module,'GENERATOR_FILES',paths)
    if hasattr(suite,'configuration_bindings'):suite.configuration_bindings[CHECK]=dict(nativeCall=len(suite.calls)-1,names=['GENERATED_INDEX_SOURCES_EXTRACTOR','GENERATOR_FILES'])
    from scripts.lint_lib.ast_collectors import AstEventCollector
    class NativeCollector(AstEventCollector):
        def __init__(self,path):self.path=path;self.source=''
        def start_file(self,path,source,context):self.path=path;self.source=source
        def finish_file(self):return extract(suite,module,self.path,self.source).facts.get(module.GENERATED_INDEX_SOURCES_EXTRACTOR.extractor_id,[])
    suite.replace(module,'_GeneratedIndexCollector',NativeCollector)
    suite.replace(module,'generated_index_sources_fact_extractors',lambda root:{module.GENERATED_INDEX_SOURCES_EXTRACTOR.extractor_id:NativeCollector})
    def plugin(scope):
        from fact_transport import encode_bundles
        def report(view):
            bundles=[dict(path=str(key),source_path=str(key),value=b.facts.get(module.GENERATED_INDEX_SOURCES_EXTRACTOR.extractor_id)) for key,b in view.files.items()]
            rows=suite.native(CHECK,'_report_facts',[dict(path='reporter-facts.json',source=json.dumps(dict(bundles=encode_bundles(bundles)),ensure_ascii=False))])
            if any(r['code'] not in {'SF_ROOT','SF_REPORT'} for r in rows):raise ValueError('Unexpected native generated-index report')
            values=sorted((r['parameters'] for r in rows if r['code']=='SF_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal']))
            return [module.StructuredViolation(CHECK,p['source_code'],Path(p['file']),json.loads(p['line_json']),p['source_message'],p['severity']) for p in values]
        return module.LinterPlugin(**json.loads(config['plugin']),file_scope=scope,required_facts=(module.GENERATED_INDEX_SOURCES_EXTRACTOR,),reporter=report)
    suite.replace(module,'_generated_index_sources_plugin',plugin)
    suite.replace(module,'generated_index_sources_plugin',lambda root:plugin(module.FileScopeSpec(explicit_files=paths)))
    def parse(value):
        rows=suite.native(CHECK,'_parse_violation_line',[],dict(gisParseValue=value))
        if len(rows)!=1:raise ValueError('One native generated-index parse result required')
        r=rows[0]
        if r['code']=='GIS_NONE':return None
        if r['code']!='GIS_PARSED':raise ValueError('Unexpected native generated-index parse')
        p=r['parameters'];return Path(p['file']),json.loads(p['line_json']),p['source_message']
    suite.replace(module,'_parse_violation_line',parse)
    def format(value):
        payload=dict(path=str(value.path),line=value.line,message=value.message)
        rows=suite.native(CHECK,'_format_finding',[dict(path='generated-values.json',source=json.dumps(payload,ensure_ascii=False))])
        if len(rows)!=1 or rows[0]['code']!='GIS_FORMATTED':raise ValueError('One native generated-index format result required')
        return rows[0]['parameters']['value']
    suite.replace(module,'_format_finding',format)
    def unavailable(path,source,tree=None):
        rows=suite.native(CHECK,'_extract_generated_index_source_facts',[])
        if len(rows)!=1 or rows[0]['code']!='GIS_UNAVAILABLE':raise ValueError('One native generated-index extractor error required')
        raise RuntimeError(rows[0]['parameters']['value'])
    suite.replace(module,'_extract_generated_index_source_facts',unavailable)
    def cli():
        parser=argparse.ArgumentParser(description=config['description'])
        parser.add_argument('--check',action='store_true',help=config['check_help'])
        parser.add_argument('--root',type=Path,default=module.REPO_ROOT)
        args=parser.parse_args();values=module.check_repo(args.root)
        rows=suite.native(CHECK,'_cli_report',[],dict(gisCliValues=values,gisCliCheck=str(int(args.check))))
        if any(r['code'] not in {'GIS_CLI_OUTPUT','GIS_CLI_EXIT'} for r in rows):raise ValueError('Unexpected native generated-index CLI result')
        for r in sorted((r for r in rows if r['code']=='GIS_CLI_OUTPUT'),key=lambda r:r['parameters']['ordinal']):sys.stdout.write(r['parameters']['value'])
        statuses=[r['parameters']['status'] for r in rows if r['code']=='GIS_CLI_EXIT']
        if len(statuses)!=1:raise ValueError('One native generated-index CLI status required')
        return statuses[0]
    suite.replace(module,'cli_main',cli)
    module._rerail_generated_extract=lambda path,source:extract(suite,module,path,source)
    module._rerail_generated_production=lambda root:production(suite,root)
    if detectors:
        for name in ['check_file','check_source','check_repo','_run_fact_engine_for_files']:
            original=getattr(module,name);signature=inspect.signature(original)
            def bind(entry,signature,original):
                @wraps(original)
                def run(*args,**kwargs):
                    values=signature.bind(*args,**kwargs);values.apply_defaults()
                    return scan(suite,entry,module,values.arguments)
                return run
            suite.replace(module,name,bind(name,signature,original))


def scan(suite,entry,module,args):
    if entry=='check_repo':paths=[Path(args['root'])/p for p in module.GENERATOR_FILES]
    elif entry=='_run_fact_engine_for_files':paths=list(map(Path,args['files']))
    else:paths=[Path(args['path'])]
    paths=[p if p.is_absolute() else module.REPO_ROOT/p for p in paths]
    selected=suite.native(CHECK,'_select_files',[],dict(gisCandidatePaths=[str(p) for p in paths],
        gisCandidateIsFile=[str(int(p.is_file())) for p in paths]))
    if any(r['code']!='GIS_SELECTED_FILE' for r in selected):raise ValueError('Unexpected native generated-index file selection')
    paths=[Path(r['parameters']['path']) for r in sorted(selected,key=lambda r:r['parameters']['order_key'])]
    files=[];originals={}
    for index,path in enumerate(paths):
        try:source=path.read_text(encoding='utf-8')
        except FileNotFoundError:continue
        logical=f'input/{index}/{path.name}';files.append(dict(path=logical,source=source));originals[logical]=path
    rows=suite.native(CHECK,entry,files,dict(apiInputDirs=['input'],apiBuffers=list(originals),apiSourcePaths=[str(p) for p in originals.values()]))
    if any(r['code']!='missing_source' for r in rows):raise AssertionError('Unexpected generated-index diagnostic')
    order={f['path']:i for i,f in enumerate(files)}
    return [f"{originals[r['path']]}:{r['line']}: {r['message']}" for r in sorted(rows,key=lambda r:(order[r['path']],r['parameters']['ordinal'],r['parameters']['rank']))]


def production(suite,root):
    from first_ten_production_contract import normalize
    from migration import compare
    rows=suite.native(CHECK,'_production_contract',[],root=Path(root))
    errors=[r for r in rows if r['code']=='GIS_READ_ERROR']
    if errors:
        p=errors[0]['parameters']
        if p['kind']=='InvalidUtf8':
            reason={'invalid-start':'invalid start byte','invalid-continuation':'invalid continuation byte','unexpected-end':'unexpected end of data'}.get(p['reason'],p['reason'])
            raise UnicodeDecodeError('utf-8',bytes.fromhex(p['bytes_hex']),p['start'],p['end'],reason)
        raise OSError(p['errno'],p['detail'],str(root/errors[0]['path']))
    if any(r['code'] not in {'DL_META','DL_FILE','DL_FACT','DL_REPORT'} for r in rows):raise ValueError('Unexpected native generated-index production result')
    meta={r['parameters']['key']:r['parameters']['value'] for r in rows if r['code']=='DL_META'}
    files=sorted((r['parameters'] for r in rows if r['code']=='DL_FILE'),key=lambda p:p['path_order'])
    facts={p['path']:({meta['extractor']:[]} if p['fact_present'] else {}) for p in files}
    for p in sorted((r['parameters'] for r in rows if r['code']=='DL_FACT'),key=lambda p:tuple(p[k] for k in ['path_order','order0','order1','order2'])):facts[p['path']][meta['extractor']].append(dict(line=p['f_line'],message=p['f_message']))
    reports=sorted((r['parameters'] for r in rows if r['code']=='DL_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','source_order0','source_order1','source_order2']))
    findings=[dict(linter_name=CHECK,code=p['source_code'],path=p['file'],line=p['source_line'],message=p['source_message'],severity=meta['findingSeverity']) for p in reports]
    public=suite.native(CHECK,None,[],root=Path(root))
    public.sort(key=lambda r:(r['path'],r['parameters']['ordinal'],r['parameters']['rank']))
    compare(findings,[dict(linter_name=CHECK,code=r['code'],path=str(root/r['path']),line=r['line'],message=r['message'],severity=meta['findingSeverity']) for r in public])
    return normalize(dict(selectedFiles=[p['path'] for p in files],facts=facts,findings=findings,plugin={k:meta[k] for k in ['name','enforcement','description']}),root)


def normalize(values,root):
    rows=[]
    for value in values:
        head,separator,message=value.partition(': ');path,colon,line=head.rpartition(':')
        if not separator or not colon:raise ValueError('Malformed generated-index finding')
        path=Path(path)
        if path.is_absolute():path=path.relative_to(root)
        rows.append(dict(path=path.as_posix(),line=int(line),column=None,code='missing_source',message=message,severity=None))
    return rows
