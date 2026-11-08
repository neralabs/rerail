"""Transport file batches and metadata; execute fragment decisions in native .rr."""
import sys
from pathlib import Path


def ordered(rows):return sorted(rows,key=lambda r:(r['path'],r['line'],r['parameters'].get('origin',-1),r['parameters'].get('tag','')))

def install(suite,module):
    formatted={}
    def native(entry,documents=(),options=None):
        return suite.native('grammar_fragment_usage',entry,list(documents),options)
    def render(v):
        key=(str(v.file),v.line,v.class_name,v.message)
        if key not in formatted:
            rows=native('format',options=dict(formatPath=key[0],formatLine=str(key[1]),formatClass=key[2],formatMessage=key[3]))
            formatted[key]=next(r['parameters']['value'] for r in rows if r['code']=='GFU_FORMAT')
        return formatted[key]
    suite.replace(module.Violation,'__str__',render)
    def files(paths,root=None):
        documents=[];original=[]
        for value in paths:
            path=Path(value)
            if not path.is_file():continue
            logical=path.relative_to(root).as_posix() if root is not None else 'input'+str(len(documents))+path.suffix
            documents.append(dict(path=logical,source=path.read_text(encoding='utf-8')));original.append(str(path))
        if not documents:return []
        return native(None if root is not None else 'files',documents,dict(documentPaths=[d['path'] for d in documents],originalPaths=original,originalNames=[Path(p).name for p in original]))
    def scan(paths):
        rows=ordered(files(paths))
        return sorted([module.Violation(Path(r['path']),r['line'],r['parameters']['class_name'],r['parameters']['detail']) for r in rows if r['code']=='GFU_FACT'],key=lambda v:(str(v.file),v.line))
    def check_file(file_path):
        if not file_path.is_file():raise FileNotFoundError(f'Python input file does not exist: {file_path}')
        return scan([file_path])
    suite.replace(module,'check_file',check_file)
    suite.replace(module,'check_directory',lambda directory:scan(sorted(p for p in directory.rglob('*.py') if p.is_file() and 'tests' not in p.parts)))
    suite.replace(module,'_run_fact_engine_for_files',scan)
    def hint():sys.stdout.write(next(r['parameters']['value'] for r in native('print_autofix_hint') if r['code']=='GFU_HINT'))
    suite.replace(module,'print_autofix_hint',hint)
    plugins={}
    def plugin(root,file_scope):
        def reporter(view):raise NotImplementedError('Use the native whole-batch policy, not the legacy fact view')
        value=module.LinterPlugin(name='grammar_fragment_usage',enforcement='hard-fail',description='SqlFragment is restricted to render surfaces in grammar classes',file_scope=file_scope,required_facts=(module.GRAMMAR_FRAGMENT_USAGE_EXTRACTOR,),reporter=reporter)
        plugins[id(value)]=(value,Path(root));return value
    suite.replace(module,'_grammar_fragment_usage_plugin',plugin)
    def default_plugin(root):
        directories=[root/name for name in ['_internal','api','data_layer','impl']]+sorted(root.glob('l[0-9]*'))
        return plugin(root,module.FileScopeSpec(directories=tuple(p for p in directories if p.is_dir()),include_tests=False))
    suite.replace(module,'grammar_fragment_usage_plugin',default_plugin)
    suite.replace(module,'grammar_fragment_usage_fact_extractors',lambda root:{module.GRAMMAR_FRAGMENT_USAGE_EXTRACTOR.extractor_id:module._FragmentUsageCollector})
    from scripts.lint_lib.linter_plugin import StructuredLinterResult
    def run_plugins(engine,values,*,max_workers=None,**kwargs):
        result=[]
        for value in values:
            if id(value) not in plugins:raise NotImplementedError('This migration adapter only executes its native policy')
            _,root=plugins[id(value)];rows=files(sorted(root.rglob('*.py')),root)
            violations=tuple(module.StructuredViolation(linter_name=value.name,code=r['code'],path=Path(r['path']),line=r['line'],message=r['message'],severity=r['parameters']['report_severity']) for r in ordered(rows) if r['code'] in {'sqlfragment_usage','python_parse_error'})
            result.append(StructuredLinterResult(name=value.name,enforcement=value.enforcement,violations=violations,duration_ms=0))
        return result
    suite.replace(module.FactEngine,'run_plugins',run_plugins)
