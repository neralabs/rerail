"""Transport file batches and supplied trees; obtain render decisions from native relations."""
import ast
from pathlib import Path
from ast_transport import visit_columns


def ordered(rows):
    return sorted(rows,key=lambda r:(r['path'],r['parameters'].get('ordinal',-1)))


def report(module,path,stats,rows):
    violations=[module.FStringViolation(file=path,method_name=r['parameters']['method_name'],
        class_name=r['parameters']['class_name'] if r['parameters']['has_class'] else None,
        line_number=r['line'],fstring_preview=r['parameters']['preview']) for r in ordered(rows) if r['code']=='RF_FACT']
    result=module.FileReport(path=path,violations=violations,render_methods_scanned=stats['render_methods_scanned'])
    result._native_count=stats['violation_count'];result._native_include=stats['include_report']
    return result


def scan(suite,module,root,paths,entry="_reports_for_files"):
    root=Path(root);files=[];original={}
    for value in paths:
        path=Path(value)
        if not path.is_absolute():path=root/path
        try:source=path.read_text(encoding='utf-8')
        except (OSError,UnicodeDecodeError):continue
        logical=path.relative_to(root).as_posix() if path.is_relative_to(root) else 'external-'+str(len(files))+'/'+path.name
        if logical in original:continue
        original[logical]=path;files.append(dict(path=logical,source=source))
    options=dict(documentPaths=[f['path'] for f in files],originalPaths=[str(original[f['path']]) for f in files])
    rows=suite.native('render_fstrings',entry,files,options)
    reports=[]
    for path in sorted(original.values()):
        selected=[r for r in rows if r['path']==str(path)]
        stats=next((r['parameters'] for r in selected if r['code']=='RF_REPORT'),None)
        if stats is not None:reports.append(report(module,path,stats,selected))
    return reports


def root_for_path(module,path):
    project=Path(module.__file__).parent.parent
    return project if path.is_relative_to(project) else path.parent


def graph(suite,path,tree):
    options=visit_columns(tree);nodes=[];pending=[tree]
    while pending:
        node=pending.pop();nodes.append(node);pending.extend(reversed(list(ast.iter_child_nodes(node))))
    options['astConstantText']=[str(n.value) if isinstance(n,ast.Constant) else '' for n in nodes]
    options['originalPath']=str(path)
    return suite.native('render_fstrings','collector',[],options)


def collector_result(module,values,suite=None):
    path,tree=values['filepath'],values['tree'];source=values.get('source','')
    if suite:
        rows=graph(suite,path,tree)
        if values.get('stats'):
            return dict(render_methods_scanned=next(r['parameters']['render_methods_scanned'] for r in rows if r['code']=='RF_REPORT'))
        result=[]
        for r in ordered(rows):
            if r['code']!='RF_FACT':continue
            p=r['parameters'];root=values['root']
            filepath=path.relative_to(root).as_posix() if path.is_relative_to(root) else path.as_posix()
            result.append(dict(path=filepath,method_name=p['method_name'],class_name=p['class_name'] if p['has_class'] else None,line=r['line'],preview=p['preview']))
        return result
    from scripts.lint_lib.ast_collectors import AstTraversalContext
    collector=module.RenderFStringStatsCollector() if values.get('stats') else module.RenderFStringCollector(path,values['root'])
    parents={tree:None};children={};pending=[tree]
    while pending:
        n=pending.pop();children[n]=tuple(ast.iter_child_nodes(n))
        for c in children[n]:parents[c]=n
        pending.extend(reversed(children[n]))
    context=AstTraversalContext(path,source,parents,children);collector.start_file(path,source,context);pending=[(tree,False)]
    while pending:
        n,leaving=pending.pop();kind=type(n).__name__
        if leaving:
            collector.on_exit_node(n,context);handler=getattr(collector,'on_exit_'+kind,None)
        else:
            collector.on_node(n,context);handler=getattr(collector,'on_'+kind,None)
            pending.append((n,True));pending.extend((c,False) for c in reversed(children[n]))
        if handler:handler(n,context)
    return collector.finish_file()


def formatted(suite,reports,root,mode):
    options=dict(formatReportPaths=[],formatReportOriginalPaths=[],formatReportErrors=[],formatReportMethods=[],formatViolationOwners=[],formatViolationLines=[],formatViolationMethods=[],formatViolationClasses=[],formatViolationPreviews=[])
    for i,report in enumerate(reports):
        options['formatReportOriginalPaths'].append(report.path.as_posix())
        try:relative,error=report.path.relative_to(root).as_posix(),''
        except ValueError as exception:relative,error='',str(exception)
        options['formatReportPaths'].append(relative)
        options['formatReportErrors'].append(error)
        options['formatReportMethods'].append(str(report.render_methods_scanned))
        for v in report.violations:
            options['formatViolationOwners'].append(str(i))
            options['formatViolationLines'].append(str(v.line_number))
            options['formatViolationMethods'].append(v.method_name)
            options['formatViolationClasses'].append('' if v.class_name is None else v.class_name)
            options['formatViolationPreviews'].append(v.fstring_preview)
    rows=suite.native('render_fstrings','format',[],options)
    errors=[r for r in rows if r['code']=='RF_FORMAT_ERROR' and r['parameters']['mode']==mode]
    if errors:raise ValueError(errors[0]['parameters']['value'])
    return next(r['parameters']['value'] for r in rows if r['code']=='RF_FORMAT' and r['parameters']['mode']==mode)


def install(suite,module):
    def empty(path):
        result=module.FileReport(path=path);result._native_count=0;return result
    def analyze_file(filepath):
        try:source=filepath.read_text(encoding='utf-8')
        except UnicodeDecodeError:return empty(filepath)
        return analyze_source(filepath,source)
    def analyze_source(filepath,source,tree=None):
        gate=suite.native('render_fstrings','source_gate',[],dict(providedSource=source))
        if not any(r['code']=='RF_READ' for r in gate):return empty(filepath)
        rows=scan(suite,module,root_for_path(module,filepath),[filepath])
        return rows[0] if rows else empty(filepath)
    def for_files(root,files):
        return [r for r in scan(suite,module,root,files) if r._native_include]
    def collect_reports(root,target_dirs=None):
        targets=target_dirs if target_dirs is not None else ['impl/common/l2100_sql_grammar']
        paths=[]
        for target in targets:
            directory=root/target
            if directory.exists():paths.extend(directory.rglob('*.py'))
        return [r for r in scan(suite,module,root,paths,"collect_reports") if r._native_include]
    def is_render_method(name):return bool(suite.native('render_fstrings','_is_render_method',[],dict(nameProbe=name)))
    for name,value in [('analyze_file',analyze_file),('analyze_source',analyze_source),('_reports_for_files',for_files),
            ('collect_reports',collect_reports),('_is_render_method',is_render_method)]:suite.replace(module,name,value)
    suite.replace(module,'format_summary',lambda reports,root:formatted(suite,reports,root,'summary'))
    suite.replace(module,'format_detailed',lambda reports,root:formatted(suite,reports,root,'detailed'))
    suite.replace(module.FileReport,'violation_count',property(lambda self:self._native_count))
