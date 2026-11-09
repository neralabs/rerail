"""Transport file arguments and configuration; native relations decide violations."""
from pathlib import Path


def ordered(rows):
    return sorted(rows,key=lambda r:(r['parameters'].get('file_order',0),r['parameters'].get('ordinal',0)))


def install(suite,module):
    def configuration():
        return dict(constantsSupplied=True,bannedCalls=list(module.BANNED_CALL_ATTRS),
                    bannedDefs=list(module.BANNED_DEF_NAMES),mixedTextOrder=list(module.MIXED_HELPER_TEXT),
                    compatibilityFiles=list(module.COMPATIBILITY_STUB_FILES),excludedParts=list(module.EXCLUDED_PARTS),
                    excludedPrefixes=list(module.EXCLUDED_PREFIXES))
    def native(entry,documents=(),options=None):
        return suite.native('raw_execute_regression',entry,list(documents),dict(configuration(),**(options or {})))
    def relative(path):
        rows=native('_relative',options=dict(argumentPath=str(path),argumentRoot=str(module.REPO_ROOT)))
        error=next((r['parameters']['value'] for r in rows if r['code']=='RRE_ERROR'),None)
        if error is not None:raise ValueError(error)
        return next(r['parameters']['value'] for r in rows if r['code']=='RRE_VALUE')
    suite.replace(module,'_relative',relative)
    def text(path,text=None):
        if text is None:text=path.read_text(encoding='utf-8')
        rows=native('_check_text',options=dict(argumentPath=str(path),argumentRoot=str(module.REPO_ROOT),sourceText=text))
        errors(rows)
        return [r['parameters']['value'] for r in ordered(rows) if r['code']=='RRE_STRING']
    suite.replace(module,'_check_text',text)
    def errors(rows):
        value=next((r['parameters']['value'] for r in rows if r['code']=='RRE_ERROR'),None)
        if value is not None:raise ValueError(value)
    def scan(entry,path,text=None,tree=None,include_text=False):
        if text is None and entry=='_check_ast':text=path.read_text(encoding='utf-8')
        options=dict(argumentPath=str(path),argumentRoot=str(module.REPO_ROOT),gateText=text or '',includeText=include_text)
        prepared=native('_prepare_file',options=dict(options,preparedEntry=entry));errors(prepared)
        if not next(r['parameters']['value'] for r in prepared if r['code']=='RRE_BOOL'):return []
        documents=[]
        if path.is_file() and path.suffix=='.py':documents=[dict(path='input.py',source=path.read_text(encoding='utf-8'))]
        rows=native(entry,documents,options);errors(rows)
        if entry in {'_facts_for_file','scan_file_content'}:
            return [dict(file_path=r['parameters']['path'],line=r['parameters']['line'],message=r['parameters']['detail']) for r in ordered(rows) if r['code']=='RRE_FACT']
        return [r['parameters']['value'] for r in ordered(rows) if r['code']=='RRE_STRING']
    suite.replace(module,'_check_ast',lambda path,text=None,tree=None:scan('_check_ast',path,text,tree))
    suite.replace(module,'_facts_for_file',lambda path,*,include_text:scan('_facts_for_file',path,include_text=include_text))
    suite.replace(module,'_violation_strings_for_file',lambda path,*,include_text:scan('_violation_strings_for_file',path,include_text=include_text))
    suite.replace(module,'scan_file_content',lambda path,text,tree=None:scan('scan_file_content',path,text,tree,True))
    def excluded(path):
        rows=native('_is_excluded',options=dict(argumentPath=str(path),argumentRoot=str(module.REPO_ROOT)));errors(rows)
        return bool(next(r['parameters']['value'] for r in rows if r['code']=='RRE_BOOL'))
    suite.replace(module,'_is_excluded',excluded)
    def fact(text):
        rows=native('_violation_text_to_fact',options=dict(factText=text));errors(rows)
        p=next(r['parameters'] for r in rows if r['code']=='RRE_FACT')
        return dict(file_path=p['path'],line=p['line'],message=p['detail'])
    suite.replace(module,'_violation_text_to_fact',fact)
    def stub(context):
        rows=native('_FunctionContext.is_raising_stub',options=dict(hasRaise=context.has_raise,hasRaiseHelper=context.has_raise_helper,hasBannedCall=context.has_banned_call))
        return bool(next(r['parameters']['value'] for r in rows if r['code']=='RRE_BOOL'))
    suite.replace(module._FunctionContext,'is_raising_stub',property(stub))
    def materializer(name,class_stack):
        rows=native('_is_query_raw_materializer',options=dict(queryName=name,queryClasses=list(class_stack)))
        return bool(next(r['parameters']['value'] for r in rows if r['code']=='RRE_BOOL'))
    suite.replace(module,'_is_query_raw_materializer',materializer)
