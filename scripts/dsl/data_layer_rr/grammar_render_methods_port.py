"""Transport the render-method APIs to native rules without executing source detectors."""
import ast
import sys
from pathlib import Path
from ast_transport import visit_columns


def install(suite,module):
    from ast_sources import Sources
    parsed_sources=Sources(ast.parse)
    suite.replace(ast,'parse',parsed_sources.parse)
    configuration=suite.native('grammar_render_methods','configuration',[])
    for row in configuration:
        if row['code']=='GRM_METADATA' and row['parameters']['key']!='autofix':
            suite.replace(module,row['parameters']['key'],row['parameters']['value'])
    def options():
        return dict(staticAllowlist=list(module.ALLOWLISTED_STATIC_CLASSES),legacyAllowlist=list(module.LEGACY_ALLOWLISTED_CLASSES),
            publicKey=module.ALLOWLIST_PUBLIC_KEY_HEX,signature=module.ALLOWLIST_SIGNATURE)
    def native(entry,files=(),extra=None):
        return suite.native('grammar_render_methods',entry,list(files),dict(options(),**(extra or {})))
    def scalar(entry,extra=None,default=None):
        return next((r['parameters']['value'] for r in native(entry,extra=extra) if r['code']=='GRM_VALUE'),default)
    suite.replace(module,'_compute_signature_message',lambda:scalar('_compute_signature_message').encode('utf-8'))
    suite.replace(module,'_get_public_key_bytes',lambda:bytes.fromhex(scalar('_get_public_key_bytes')))
    suite.replace(module,'_verify_allowlist_integrity',lambda:any(r['code']=='GRM_TRUE' for r in native('_verify_allowlist_integrity')))
    suite.replace(module,'print_autofix_hint',lambda:sys.stdout.write(next(r['parameters']['value'] for r in native('print_autofix_hint') if r['code']=='GRM_METADATA')) and None)
    def initialize(visitor,file_path,source_lines=None,class_name=None):
        visitor.file_path=file_path
        visitor.source_lines=[] if source_lines is None else source_lines
        if isinstance(visitor,module.RenderBodyAnalyzer):visitor.class_name=class_name
        visitor.violations=[]
        if isinstance(visitor,module.GrammarRenderMethodsCollector):
            visitor._class_stack=[];visitor._function_render_stack=[]
        elif isinstance(visitor,module.RenderMethodVisitor):
            visitor._current_class=None;visitor._in_render_method=False;visitor._current_method=None
        elif isinstance(visitor,module.DetailedRenderVisitor):visitor._class_stack=[]
        elif isinstance(visitor,module.RenderBodyAnalyzer):
            visitor._is_static_allowlisted=any(r['code']=='GRM_TRUE' for r in native('static_membership',extra=dict(className=class_name)))
            visitor._fstrings_in_grammar_exception=set()
    for name in ['GrammarRenderMethodsCollector','RenderMethodVisitor','DetailedRenderVisitor','RenderBodyAnalyzer']:
        suite.replace(getattr(module,name),'__init__',initialize)
    def graph(entry,tree,extra=None):return native(entry,extra=dict(visit_columns(tree),**(extra or {})))
    for cls,methods in [('RenderMethodVisitor',['_get_name','_get_call_name_recursive','_is_string_expr']),('RenderBodyAnalyzer',['_is_string_literal','_is_string_expr'])]:
        for method in methods:
            def factory(cls,method):
                def evaluate(visitor,node):
                    rows=graph(cls+'.'+method,node)
                    if method.startswith('_is_'):return any(r['code']=='GRM_TRUE' for r in rows)
                    return next((r['parameters']['value'] for r in rows if r['code']=='GRM_VALUE'),None)
                return evaluate
            suite.replace(getattr(module,cls),method,factory(cls,method))
    def line_source(visitor,lineno):return scalar('RenderBodyAnalyzer._get_line_source',dict(lineno=str(lineno),sourceLines=visitor.source_lines))
    suite.replace(module.RenderBodyAnalyzer,'_get_line_source',line_source)
    def expression_source(visitor,node):
        raw=parsed_sources.segment(node)
        source='async def __transport():\n    return (\n'+raw+'\n    )()\n'
        rows=native('RenderBodyAnalyzer._get_source',[dict(path='transport.py',source=source)])
        return next((r['parameters']['value'] for r in rows if r['code']=='GRM_VALUE'),'<unknown>')
    suite.replace(module.RenderBodyAnalyzer,'_get_source',expression_source)
    def scan(root,files):
        documents=[];paths=[]
        for value in files:
            path=Path(value)
            if not path.is_absolute():path=Path(root)/path
            if not path.is_file():continue
            try:source=path.read_text()
            except (OSError,UnicodeDecodeError):continue
            logical=f'input{len(paths)}'+path.suffix
            documents.append(dict(path=logical,source=source));paths.append(str(path))
        if not documents:return []
        rows=native('_violations_for_files',documents,dict(documentPaths=[d['path'] for d in documents],originalPaths=paths))
        result=[]
        for row in sorted(rows,key=lambda r:(r['path'],r['parameters'].get('ordinal',0))):
            if row['code']=='GRM_FACT':
                p=row['parameters'];result.append(module.RenderViolation(row['path'],row['line'],p['class_name'],p['method_name'],p['violation_type'],p['detail']))
        return result
    suite.replace(module,'_violations_for_files',scan)
    suite.replace(module,'scan_source',lambda file_path,source,tree=None:scan(module.PROJECT_ROOT,(file_path,)))
    def scan_file(file_path):
        try:file_path.read_text()
        except UnicodeDecodeError as error:
            print(f'Warning: Could not parse {file_path}: {error}',file=sys.stderr);return []
        return scan(module.PROJECT_ROOT,(file_path,))
    suite.replace(module,'scan_file',scan_file)


def configured_scan(module,values):
    """Apply caller-supplied configuration in the test host, then restore it."""
    names=['ALLOWLISTED_STATIC_CLASSES','LEGACY_ALLOWLISTED_CLASSES']
    previous={name:getattr(module,name) for name in names}
    try:
        for name in names:
            if name in values:setattr(module,name,values[name])
        return module.scan_file(values['file_path'])
    finally:
        for name,value in previous.items():setattr(module,name,value)
