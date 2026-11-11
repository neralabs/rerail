"""Transport connection/transaction file inputs and passive Violation DTOs."""
from pathlib import Path
import ast
from functools import wraps
import inspect
import json
import sys

CHECK='conn_or_txn_union'


def emit_logs(rows):
    for row in rows:
        if row['code']=='CUT_SIGNATURE_LOG':sys.stderr.write(row['parameters']['text'])


def signed_options(module):
    return dict(cutAllowOverride='true',cutAllowEntries=sorted(module.ALLOWLIST),cutAllowSignature=module._ALLOWLIST_SIGNATURE)


def production(backend,module,root):
    from first_ten_production_contract import normalize
    from migration import compare
    rows=backend.native(CHECK,'_production_contract',[],signed_options(module),root=root)
    errors=[r for r in rows if r['code']=='CUT_READ_ERROR']
    if errors:
        p=errors[0]['parameters']
        if p['kind']=='InvalidUtf8':
            reason={'invalid-start':'invalid start byte','invalid-continuation':'invalid continuation byte','unexpected-end':'unexpected end of data'}.get(p['reason'],p['reason'])
            raise UnicodeDecodeError('utf-8',bytes.fromhex(p['bytes_hex']),p['start'],p['end'],reason)
        raise OSError(p['errno'],p['detail'],str(root/errors[0]['path']))
    emit_logs(rows)
    if any(r['code'] not in {'DL_META','DL_FILE','DL_FACT','DL_REPORT','CUT_SIGNATURE_LOG'} for r in rows):raise ValueError('Unexpected native union production response')
    meta={r['parameters']['key']:r['parameters']['value'] for r in rows if r['code']=='DL_META'}
    if not meta:raise ValueError('Missing native union production metadata')
    files=sorted((r['parameters'] for r in rows if r['code']=='DL_FILE'),key=lambda p:p['path_order'])
    facts={p['path']:({meta['extractor']:[]} if p['fact_present'] else {}) for p in files}
    for p in sorted((r['parameters'] for r in rows if r['code']=='DL_FACT'),key=lambda p:(p['path_order'],p['ordinal'])):
        facts[p['path']][meta['extractor']].append(dict(line_number=p['line_number'],function_name=p['function_name'],violation_type=p['violation_type'],details=p['details']))
    findings=[dict(linter_name=CHECK,code=p['source_code'],path=p['file'],line=p['source_line'],message=p['source_message'],severity=meta['findingSeverity']) for p in sorted((r['parameters'] for r in rows if r['code']=='DL_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','source_order0','source_order1','source_order2']))]
    public=backend.native(CHECK,None,[],signed_options(module),root=root)
    compare(findings,[dict(linter_name=CHECK,code=r['code'],path=str(root/r['path']),line=r['line'],message=r['message'],severity=meta['findingSeverity']) for r in sorted(public,key=lambda r:(Path(r['path']).parts,r['parameters'].get('ordinal',0)))])
    return normalize(dict(selectedFiles=[p['path'] for p in files],facts=facts,findings=findings,plugin={k:meta[k] for k in ['name','enforcement','description']}),root)


def extract(backend,module,path,source):
    rows=backend.native(CHECK,'_collector_facts',[dict(path='impl/collector.py',source=source)],dict(signed_options(module),cutCollectorName=Path(path).name,apiInputDirs=['impl']))
    if any(r['code'] not in {'CUT_FACT','CUT_COLLECTED','CUT_COLLECTOR_ERROR'} for r in rows):raise ValueError('Unexpected native union collection')
    family=module.CONN_OR_TXN_UNION_EXTRACTOR.extractor_id
    present=[r['parameters']['present'] for r in rows if r['code']=='CUT_COLLECTED']
    if len(present)!=1:raise ValueError('One native union collection status required')
    facts=[{k:r['parameters'][k] for k in ['line_number','function_name','violation_type','details']} for r in sorted((r for r in rows if r['code']=='CUT_FACT'),key=lambda r:r['parameters']['ordinal'])]
    errors=[r['parameters']['value'] for r in rows if r['code']=='CUT_COLLECTOR_ERROR']
    from types import SimpleNamespace
    return SimpleNamespace(facts={family:facts} if present[0] else {},errors={family:errors[0]} if errors else {})


def helper(backend,module,owner,entry,argument):
    options=signed_options(module)
    if entry in {'_get_qualified_name','_is_allowlisted'}:
        options.update(cutClassStack=list(owner._class_stack),cutFunctionName=argument)
    else:
        from ast_transport import columns
        options.update(columns(argument));pending=[argument];values=[]
        while pending:
            node=pending.pop();value=getattr(node,'value',None)
            values.append(value if isinstance(value,str) else '')
            pending.extend(reversed(list(ast.iter_child_nodes(node))))
        options['cutLiteralStrings']=values
    rows=backend.native(CHECK,'ConnectionTransactionCollector.'+entry,[],options)
    if entry=='_get_all_type_names':
        if any(r['code']!='CUT_HELPER_NAME' for r in rows):raise ValueError('Unexpected native union type names')
        return {r['parameters']['value'] for r in rows}
    code='CUT_HELPER_QUALIFIED' if entry=='_get_qualified_name' else 'CUT_HELPER_BOOL'
    if len(rows)!=1 or rows[0]['code']!=code:raise ValueError('One native union helper value required')
    value=rows[0]['parameters']['value'];return value if entry=='_get_qualified_name' else bool(value)


def install(backend,module,detectors=False):
    rows=backend.native(CHECK,'_cut_configuration',[])
    if not rows or any(r['code']!='CUT_CONFIG' for r in rows):raise ValueError('Missing native union configuration')
    config={r['parameters']['key']:r['parameters']['value'] for r in rows}
    data=json.loads(config['extractor'])
    for key in ['source_triggers','source_trigger_groups','project_input_facts','node_kinds']:data[key]=tuple(data[key])
    backend.replace(module,'CONN_OR_TXN_UNION_EXTRACTOR',type(module.CONN_OR_TXN_UNION_EXTRACTOR)(**data))
    for name,key in [('ALLOWLIST','allowlist'),('CONNECTION_TYPES','connections'),('TRANSACTION_TYPES','transactions')]:backend.replace(module,name,set(json.loads(config[key])))
    backend.replace(module,'_ALLOWLIST_SIGNATURE',config['signature'])
    backend.replace(module,'SOURCE_TYPE_TRIGGERS',tuple(json.loads(config['source_type_triggers'])))
    if hasattr(backend,'configuration_bindings'):backend.configuration_bindings[CHECK]=dict(nativeCall=len(backend.calls)-1,names=['CONN_OR_TXN_UNION_EXTRACTOR','ALLOWLIST','CONNECTION_TYPES','TRANSACTION_TYPES','SOURCE_TYPE_TRIGGERS','_ALLOWLIST_SIGNATURE'])
    class Collector(module.AstEventCollector):
        def __init__(self,file_path,violations=None,_class_stack=None):
            self.file_path=file_path;self.violations=[] if violations is None else violations
            self._class_stack=[] if _class_stack is None else _class_stack;self.source=''
        def start_file(self,path,source,context):self.file_path=path;self.source=source
        def finish_file(self):
            family=module.CONN_OR_TXN_UNION_EXTRACTOR.extractor_id
            facts=extract(backend,module,self.file_path,self.source).facts.get(family,[])
            self.violations.extend(module.Violation(self.file_path,p['line_number'],p['function_name'],p['violation_type'],p['details']) for p in facts)
            return facts
    for entry in ['_get_all_type_names','_has_connection_type','_has_transaction_type','_has_connection_and_transaction','_get_qualified_name','_is_allowlisted']:
        def bind(entry):return lambda owner,argument:helper(backend,module,owner,entry,argument)
        setattr(Collector,entry,bind(entry))
    backend.replace(module,'ConnectionTransactionCollector',Collector)
    def factory(path,_source):return Collector(path)
    backend.replace(module,'conn_or_txn_union_fact_extractors',lambda root:{module.CONN_OR_TXN_UNION_EXTRACTOR.extractor_id:factory})
    def project(value):
        payload=dict(line=value.line_number,function=value.function_name,type=value.violation_type,details=value.details)
        rows=backend.native(CHECK,'_violation_to_fact',[dict(path='conn-value.json',source=json.dumps(payload,ensure_ascii=False))])
        if len(rows)!=1 or rows[0]['code']!='CUT_PROJECTED':raise ValueError('One native union fact projection required')
        p=rows[0]['parameters']
        return dict(line_number=json.loads(p['line_json']),function_name=json.loads(p['function_json']),violation_type=json.loads(p['type_json']),details=json.loads(p['details_json']))
    backend.replace(module,'_violation_to_fact',project)
    def plugin(root):
        paths=list(root.iterdir())
        options=dict(cutDirectoryNames=[p.name for p in paths],cutDirectoryIsDir=[str(int(p.is_dir())) for p in paths],cutDirectoryExists=[str(int(p.exists())) for p in paths])
        rows=backend.native(CHECK,'_cut_directories',[],options)
        if any(r['code']!='CUT_DIRECTORY' for r in rows):raise ValueError('Unexpected native union directory selection')
        directories=tuple(Path(r['parameters']['directory']) for r in sorted(rows,key=lambda r:(r['parameters']['rank'],r['parameters']['directory'])))
        def report(view):
            from fact_transport import encode_bundles
            family=module.CONN_OR_TXN_UNION_EXTRACTOR.extractor_id
            bundles=encode_bundles([dict(path=str(key),source_path=str(key),value=bundle.facts.get(family)) for key,bundle in view.files.items()])
            rows=backend.native(CHECK,'_report_facts',[dict(path='conn-reporter-facts.json',source=json.dumps(dict(bundles=bundles),ensure_ascii=False))],dict(signed_options(module),cutOwnerRoot=str(root)))
            emit_logs(rows)
            if any(r['code'] not in {'SF_ROOT','SF_REPORT','CUT_SIGNATURE_ERROR','CUT_SIGNATURE_LOG'} for r in rows):raise ValueError('Unexpected native union report')
            reports=[]
            for r in sorted((r for r in rows if r['code']=='SF_REPORT'),key=lambda r:tuple(r['parameters'][k] for k in ['order0','order1','order2','order3','order4','bundle','ordinal'])):
                p=r['parameters'];reports.append(module.StructuredViolation(CHECK,p['source_code'],Path(p['file']),json.loads(p['line_json']),p['source_message'],p['severity']))
            reports.extend(module.StructuredViolation(CHECK,'unsigned_allowlist',Path(r['parameters']['file']),1,r['message']) for r in rows if r['code']=='CUT_SIGNATURE_ERROR')
            return reports
        return module.LinterPlugin(**json.loads(config['plugin']),file_scope=module.FileScopeSpec(directories=directories,include_tests=False),required_facts=(module.CONN_OR_TXN_UNION_EXTRACTOR,),reporter=report)
    backend.replace(module,'conn_or_txn_union_plugin',plugin)
    module._rerail_union_extract=lambda path,source:extract(backend,module,path,source)
    if detectors:
        original=module.analyze_file;signature=inspect.signature(original)
        @wraps(original)
        def analyze(*args,**kwargs):
            values=signature.bind(*args,**kwargs);values.apply_defaults()
            return scan(backend,'analyze_file',module,values.arguments)
        backend.replace(module,'analyze_file',analyze)


def scan(suite, entry, module, args):
    path = Path(args['file_path'])
    decode_error = None
    try:
        source = path.read_text(encoding='utf-8')
    except UnicodeDecodeError as error:
        decode_error, source = error, ''
    except OSError:
        source = ''
    root = Path(args.get('root') or Path.cwd())
    absolute = path if path.is_absolute() else Path.cwd() / path
    logical = absolute.relative_to(root).as_posix() if absolute.is_relative_to(root) else absolute.as_posix().lstrip('/')
    # A direct file API does not inherit production discovery roots or exclusions.
    # Preserve the caller path as data; the rule applies its explicit-file contract.
    rows = suite.native('conn_or_txn_union', entry, [dict(path='impl/input.py', source=source)],
        dict(signed_options(module),apiInputDirs=['impl'],apiSourcePath=logical,decodeError=str(decode_error is not None).lower()))
    if any(row['code'] == 'CUT_DECODE_ERROR' for row in rows):
        if len(rows) != 1 or decode_error is None:
            raise AssertionError('Invalid native decoding error')
        raise decode_error
    if any(row['code'] not in {'UNION', 'SEPARATE_PARAMS'} for row in rows):
        raise AssertionError('Invalid connection/transaction diagnostic')
    return [module.Violation(file_path=absolute, line_number=row['line'],
        function_name=row['parameters']['function_name'], violation_type=row['code'], details=row['parameters']['details'])
        for row in sorted(rows, key=lambda row: row['parameters']['ordinal'])]
