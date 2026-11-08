"""Transport render-fstring files and DTOs to the modern native rule."""
from pathlib import Path

def scan(suite, module, files):
    rows=suite.native('render_fstrings','_reports_for_files',[dict(path='input-'+str(i)+'.py',source=Path(p).read_text(encoding='utf-8')) for i,p in enumerate(files)])
    reports=[]
    for i,path in enumerate(files):
        selected=[r for r in rows if r['path']=='input-'+str(i)+'.py']
        stats=next(r['parameters'] for r in selected if r['code']=='RF_REPORT')
        report=module.FileReport(path=path,render_methods_scanned=stats['render_methods_scanned'])
        report._native_count=stats['violation_count'];reports.append(report)
    return reports

def install(suite,module):
    def analyze_file(filepath):return scan(suite,module,[filepath])[0]
    suite.replace(module,'analyze_file',analyze_file)
    suite.replace(module.FileReport,'violation_count',property(lambda self:self._native_count))
