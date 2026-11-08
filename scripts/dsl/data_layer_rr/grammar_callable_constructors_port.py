"""Transport grammar-file batches and frozen violation DTOs; execute detection natively."""
from pathlib import Path


def install(suite,module):
    def formatted(violation):
        if '_rr_key' not in vars(violation):
            rows=suite.native('grammar_callable_constructors','format',[],dict(formatPath=str(violation.path),formatLine=str(violation.line),formatCode=str(violation.code),formatDetail=str(violation.detail)))
            row=next(r for r in rows if r['code']=='GCC_FORMAT')['parameters']
            object.__setattr__(violation,'_rr_key',row['key']);object.__setattr__(violation,'_rr_formatted',row['formatted'])
        return vars(violation)
    suite.replace(module.Violation,'key',property(lambda violation:formatted(violation)['_rr_key']))
    suite.replace(module.Violation,'format',lambda violation:formatted(violation)['_rr_formatted'])
    def collect(root):
        documents=[];original=[]
        for path in sorted(Path(root).rglob('*.py')):
            try:source=path.read_text(encoding='utf-8')
            except (OSError,UnicodeDecodeError):continue
            logical=f'input{len(original)}.py';documents.append(dict(path=logical,source=source));original.append(str(path))
        if not documents:return []
        rows=suite.native('grammar_callable_constructors','collect_violations',documents,dict(documentPaths=[d['path'] for d in documents],originalPaths=original))
        values=[]
        for row in rows:
            if row['code']!='GCC_FACT':continue
            p=row['parameters'];violation=module.Violation(Path(row['path']),row['line'],p['violation_type'],p['detail'])
            object.__setattr__(violation,'_rr_key',p['key']);object.__setattr__(violation,'_rr_formatted',p['formatted']);values.append(violation)
        return sorted(values,key=lambda v:(str(v.path),v.line,v.code,v.detail))
    suite.replace(module,'collect_violations',collect)
