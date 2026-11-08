"""Transport file batches and DTOs for the native grammar-string-parameter rule."""
import sys
from pathlib import Path


def ordered(rows):return sorted(rows,key=lambda r:(r['path'],r['parameters'].get('ordinal',0),r['parameters'].get('first',0),r['parameters'].get('second',0)))


def install(suite,module):
    rows=suite.native('grammar_str_params','configuration',[])
    suite.replace(module,'ALLOWLISTED_STR_CLASSES',frozenset(r['parameters']['name'] for r in rows if r['code']=='GSP_CLASS'))
    suite.replace(module,'ALLOWLISTED_STR_PARAMS',frozenset((r['parameters']['cls'],r['parameters']['name']) for r in rows if r['code']=='GSP_PARAM'))
    suite.replace(module,'BASELINE_COUNTS',{r['parameters']['code']:r['parameters']['count'] for r in sorted(rows,key=lambda r:r['parameters'].get('ordinal',0)) if r['code']=='GSP_BASELINE'})
    def options():
        params=list(module.ALLOWLISTED_STR_PARAMS)
        return dict(classes=list(module.ALLOWLISTED_STR_CLASSES),paramClasses=[v[0] for v in params],paramNames=[v[1] for v in params])
    def native(entry,files=(),extra=None):return suite.native('grammar_str_params',entry,list(files),dict(options(),**(extra or {})))
    def hint():sys.stdout.write(next(r['parameters']['value'] for r in native('print_autofix_hint') if r['code']=='GSP_TEXT'))
    suite.replace(module,'print_autofix_hint',hint)
    def scan(root,paths):
        documents=[];original=[];seen=set()
        for value in paths:
            path=Path(value)
            if not path.is_absolute():path=Path(root)/path
            if path in seen or not path.is_file():continue
            seen.add(path)
            try:source=path.read_text()
            except (OSError,UnicodeDecodeError):continue
            logical=f'input{len(original)}'+path.suffix;documents.append(dict(path=logical,source=source));original.append(str(path))
        if not documents:return []
        rows=native('_violations_for_files',documents,dict(documentPaths=[d['path'] for d in documents],originalPaths=original))
        result=[]
        for row in ordered(rows):
            if row['code']=='GSP_FACT':
                p=row['parameters'];result.append(module.Violation(row['path'],row['line'],p['class_name'],p['param_name'],p['annotation'],p['violation_type'],p['detail']))
        return result
    suite.replace(module,'_violations_for_files',scan)
    def lint_source(filepath,source):
        rows=native('source_gate',extra=dict(providedSource=source))
        if not any(r['code']=='GSP_READ' for r in rows):return []
        return scan(module.ROOT,(filepath,))
    suite.replace(module,'lint_source',lint_source)
    suite.replace(module,'lint_file',lambda filepath:lint_source(filepath,filepath.read_text()))


def cli_probe(module,values):
    """Keep argument parsing and output as host protocol around native file analysis."""
    previous_root,previous_args=module.ROOT,sys.argv
    try:
        module.ROOT=values['root']
        sys.argv=['lint_grammar_str_params.py',*values['args']]
        return module.cli_main()
    finally:
        module.ROOT=previous_root;sys.argv=previous_args
