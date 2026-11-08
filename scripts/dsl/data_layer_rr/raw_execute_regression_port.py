"""Transport file arguments and configuration; native relations decide violations."""
from pathlib import Path


def ordered(rows):
    return sorted(rows,key=lambda r:(r['parameters'].get('file_order',0),r['parameters'].get('ordinal',0)))


def install(suite,module):
    def configuration():
        return dict(constantsSupplied=True,bannedCalls=list(module.BANNED_CALL_ATTRS),
                    bannedDefs=list(module.BANNED_DEF_NAMES),mixedTextOrder=list(module.MIXED_HELPER_TEXT),
                    compatibilityFiles=list(module.COMPATIBILITY_STUB_FILES))
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
        return [r['parameters']['value'] for r in ordered(rows) if r['code']=='RRE_STRING']
    suite.replace(module,'_check_text',text)
    def scan(entry,path,text=None,tree=None,include_text=False):
        if text is None:text=path.read_text(encoding='utf-8')
        documents=[]
        if path.is_file() and path.suffix=='.py':documents=[dict(path='input.py',source=path.read_text(encoding='utf-8'))]
        rows=native(entry,documents,dict(argumentPath=str(path),argumentRoot=str(module.REPO_ROOT),gateText=text,includeText=include_text))
        return [r['parameters']['value'] for r in ordered(rows) if r['code']=='RRE_STRING']
    suite.replace(module,'_check_ast',lambda path,text=None,tree=None:scan('_check_ast',path,text,tree))
