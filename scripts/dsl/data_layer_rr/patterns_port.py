"""Transport transaction-pattern API inputs and native values; keep rule decisions in .rr."""
import ast, json, os
from pathlib import Path
from callback_wrappers_port import ast_input


def categories(value, *, merge=False):
    keys=list(value);member_keys=[];names=[]
    for category,items in value.items():
        for name in items:member_keys.append(category);names.append(name)
    return dict(categoriesProvided='true',categoryKeys=keys,categoryMemberKeys=member_keys,
        categoryMemberNames=names,mergeCategories='true' if merge else 'false')


def signature(value):
    keys=list(value);member_keys=[];values=[]
    for category,items in value.items():
        for item in items:member_keys.append(category);values.append(item)
    return dict(signatureInput='true',signatureKeys=keys,signatureValueKeys=member_keys,signatureValues=values)


def query(suite,entry,options=None,tree=None,files=None):
    options=dict(options or {})
    if tree is not None:
        syntax,_=ast_input(tree);nodes=[];pending=[tree]
        while pending:
            node=pending.pop();nodes.append(node);pending.extend(reversed(list(ast.iter_child_nodes(node))))
        syntax['astLiteralText']=[str(n.value) if isinstance(n,ast.Constant) else '' for n in nodes]
        options.update(syntax)
    return suite.native('transaction_patterns',entry,files if files is not None else [dict(path='_transport.py',source='pass\n')],options)


def ordered(rows):
    return sorted(rows,key=lambda row:(row['path'],row['parameters']['exit_order'],-row['parameters']['entry_order']))


def findings(rows):
    return [dict(name=r['parameters']['name'],param=r['parameters']['param'],line=r['line'],file=r['path'],
        category=r['parameters']['category'],internal_txn_calls=r['parameters']['calls'].split(',') if r['parameters']['calls'] else None)
        for r in ordered([r for r in rows if r['code']=='TP_FINDING'])]


def graph(suite,entry,owner,tree):
    if isinstance(owner,Path):options=dict(originalPath=str(owner))
    else:options=dict(originalPath=str(owner.file_path),**categories(owner.categorized_functions))
    return findings(query(suite,entry,options,tree))


def root_for(path,baseline_file):
    resolved=path.resolve()
    for parent in (resolved.parent,*resolved.parents):
        if (parent/baseline_file).exists() or (parent/'pyproject.toml').exists():return parent
    return resolved.parent


def scan(suite,module,entry,root,paths,categorized,provided=None):
    files=[];originals=[];scopes=[]
    for i,path in enumerate(paths):
        path=Path(path);actual=path if path.is_absolute() else root/path
        try:source=actual.read_text(encoding='utf-8')
        except (OSError,UnicodeDecodeError):continue
        logical=f'input-{i}/'+path.name
        files.append(dict(path=logical,source=source));originals.append(str(actual))
        scopes.append(actual.relative_to(root).as_posix() if actual.is_relative_to(root) else actual.as_posix())
    options=dict(**categories(categorized),documentPaths=[f['path'] for f in files],originalPaths=originals,scopePaths=scopes)
    if provided is not None:options['providedSource']=provided
    return findings(query(suite,entry,options,files=files))


def install(suite,module):
    constants=query(suite,'constants');groups={}
    for row in constants:groups.setdefault(row['parameters']['kind'],[]).append(row['parameters']['value'])
    for name,kind,constructor in [('CORE_LEGITIMATE_FUNCTIONS','core',set),('ALLOWED_INIT_CLASS_PATTERNS','init',set),
        ('TS_BRIDGE_MODULE_PATTERNS','bridge',list)]:suite.replace(module,name,constructor(groups.get(kind,[])))
    suite.replace(module,'BASELINE_PUBLIC_KEY_HEX',groups['key'][0]);suite.replace(module,'BASELINE_FILE',groups['file'][0])
    def infer(func_name,file_path):
        rows=query(suite,'_infer_category',dict(functionName=func_name,originalPath=file_path))
        return next(r['parameters']['category'] for r in rows if r['code']=='TP_INFER')
    def init(func_name,class_name):
        options=dict(functionName=func_name)
        if class_name is not None:options['className']=class_name
        return bool(query(suite,'_is_connection_class_init',options)[0]['parameters']['answer'])
    def merged(baseline):
        value=baseline.get('categories',{}) if baseline is not None else {}
        rows=query(suite,'_get_categorized_functions',categories(value,merge=True));result={}
        for row in sorted([r for r in rows if r['code']=='TP_CATEGORY_KEY'],key=lambda r:r['parameters']['i']):result[row['parameters']['category']]=set()
        for row in rows:
            if row['code']=='TP_CATEGORY_MEMBER':result[row['parameters']['category']].add(row['parameters']['name'])
        return result
    def message(value):
        rows=query(suite,'_compute_signature_message',signature(value))
        return next(r['parameters']['value'].encode('utf-8') for r in rows if r['code']=='TP_SIGNATURE_MESSAGE')
    def verify(categories,signature_b64):
        rows=query(suite,'_verify_signature',dict(**signature(categories),publicKey=module.BASELINE_PUBLIC_KEY_HEX,signature=signature_b64))
        return bool(next(r['parameters']['answer'] for r in rows if r['code']=='TP_BOOL'))
    def visit(owner,node):owner.findings=graph(suite,'ConnectionParameterFinder.visit',owner,node)
    def internal(owner,node,single=False):
        options=dict(internalProbe='true')
        if owner.conn_param_name is not None:options['connectionName']=owner.conn_param_name
        if single:options['internalSingle']='true'
        rows=query(suite,'InternalTransactionFinder._check_call' if single else 'InternalTransactionFinder.visit',options,node)
        owner.internal_txn_calls.extend(r['parameters']['method'] for r in sorted(rows,key=lambda r:r['parameters']['visit']) if r['code']=='TP_INTERNAL')
    def check(node,conn_param_name):
        finder=module.InternalTransactionFinder(conn_param_name=conn_param_name);internal(finder,node);return finder.internal_txn_calls
    def facts(root_dir,files,categorized_functions):return scan(suite,module,'_facts_for_files',root_dir,files,categorized_functions)
    def analyze_source(file_path,source,categorized_functions,tree=None):
        return scan(suite,module,'analyze_source',root_for(file_path,module.BASELINE_FILE),[file_path],categorized_functions,source)
    def analyze_file(file_path,categorized_functions):
        source=file_path.read_text();return analyze_source(file_path,source,categorized_functions)
    def collect(root_dir,categorized_functions,directories=None):
        files=[]
        for directory in directories if directories is not None else ['api','impl']:
            path=root_dir/directory
            if not path.exists():continue
            for parent,dirs,names in os.walk(path):
                dirs[:]=[name for name in dirs if name not in {'__pycache__','tests'}]
                for name in names:
                    file=Path(parent)/name
                    if name.endswith('.py') and 'test_' not in name and '/tests/' not in str(file):files.append(file)
        return module._facts_for_files(root_dir,tuple(sorted(files)),categorized_functions) if files else []
    def load(root_dir):
        path=root_dir/module.BASELINE_FILE
        if not path.exists():return None
        try:return json.loads(path.read_text())
        except Exception:return None
    def token(root_dir):
        import hashlib
        try:return hashlib.sha256((root_dir/module.BASELINE_FILE).read_bytes()).hexdigest()[:16]
        except OSError:return 'missing'
    functions={'_infer_category':infer,'_is_connection_class_init':init,'_get_categorized_functions':merged,
        '_compute_signature_message':message,'_verify_signature':verify,'_check_for_internal_transactions':check,
        '_facts_for_files':facts,'analyze_source':analyze_source,'analyze_file':analyze_file,'_collect_current_findings':collect,
        '_load_baseline':load,'_get_public_key_bytes':lambda:bytes.fromhex(module.BASELINE_PUBLIC_KEY_HEX),
        '_baseline_cache_token':token,'_root_for_analysis_file':lambda path:root_for(path,module.BASELINE_FILE)}
    for name,fn in functions.items():suite.replace(module,name,fn)
    suite.replace(module.ConnectionParameterFinder,'visit',visit)
    suite.replace(module.InternalTransactionFinder,'visit',internal)
    suite.replace(module.InternalTransactionFinder,'_check_call',lambda owner,node:internal(owner,node,single=True))
