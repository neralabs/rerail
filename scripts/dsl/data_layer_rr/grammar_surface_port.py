"""Transport file I/O and ServerPlan DTOs; native relations own surface decisions."""
import hashlib
from pathlib import Path


CHECK = 'serverplan_grammar_surface'


def ordered(rows, breadth_first=False):
    def key(row):
        p = row['parameters']
        return ((p['depth'],) if breadth_first else ()) + (p['ordinal'], p['witness'])
    return sorted(rows, key=key)


def scan(suite, entry, module, arguments):
    if entry == 'iter_python_files':
        root = Path(arguments['root'])
        candidates = [path.relative_to(root).as_posix() for path in root.rglob('*')]
        rows = suite.native(CHECK, entry, [], dict(candidates=candidates))
        if any(row['code'] != 'SPG_DISCOVERY' for row in rows):
            raise AssertionError('Invalid ServerPlan discovery response')
        return sorted(root / row['parameters']['result'] for row in rows)
    path = Path(arguments['path'])
    try:
        source = path.read_text(encoding='utf-8')
    except (OSError, UnicodeDecodeError):
        source = ''
    rows = suite.native(CHECK, entry, [dict(path='input.py', source=source)], dict(filename=path.name))
    errors = [row for row in rows if row['code'] == 'SPG-SYNTAX']
    hits = [row for row in rows if row['clause'] == 'Surface']
    if len(errors) + len(hits) != len(rows):
        raise AssertionError('Unexpected ServerPlan direct response')
    return [f"{path}: {row['message']}" for row in errors] + [
        f"{path}:{row['line']}: {row['message']}" for row in ordered(hits, breadth_first=True)]


def install(suite, module):
    from scripts.lint_lib import serverplan_facts as shared
    from scripts.lint_lib.linter_plugin import FileScopeSpec, LinterPlugin, PerFileFactBundle, ProjectFactView, StructuredViolation
    spec = shared.SERVERPLAN_SURFACE_EXTRACTOR

    def project(view, extractor):
        if extractor.extractor_id != spec.extractor_id:
            raise AssertionError('Untranslated ServerPlan fact family')
        import json
        from model_transport import encode_models
        from projection_port import project_rows
        bundles=[dict(path=str(path),value=bundle.facts.get(spec.extractor_id)) for path,bundle in view.files.items()]
        rows=suite.native(CHECK,'_project_facts',[dict(path='reporter-facts.json',
            source=json.dumps(dict(bundles=encode_models(bundles)),ensure_ascii=False))])
        return project_rows(rows,shared)

    def messages(root, facts):
        import json
        from model_transport import encode_models
        from projection_port import message_rows
        bundles=[dict(path=str(i),value=file.model_dump(mode='json')) for i,file in enumerate(facts.files)]
        rows=suite.native(CHECK,'_project_messages',[dict(path='reporter-facts.json',
            source=json.dumps(dict(bundles=encode_models(bundles)),ensure_ascii=False))],dict(reportRoot=str(root)))
        return message_rows(rows)

    def view(root, *, required_facts, files=None, fact_store=None, max_workers=None):
        if tuple(required_facts) != (spec,):
            raise AssertionError('Untranslated ServerPlan fact demand')
        root = Path(root)
        paths = list(files) if files is not None else list(root.rglob('*'))
        candidates = [path.relative_to(root).as_posix() if path.is_relative_to(root) else path.as_posix()
                      for path in paths]
        selected = suite.native(CHECK, 'collector_paths', [], dict(candidates=candidates,
            regularFiles=[name for path, name in zip(paths, candidates) if path.is_file()],
            broad=str(files is None).lower()))
        if any(row['code'] != 'SPG_PATH' for row in selected):
            raise AssertionError('Unexpected ServerPlan file selection response')
        originals, sources = {}, []
        for row in sorted(selected, key=lambda row: row['parameters']['ordinal']):
            i = row['parameters']['ordinal']
            path = Path(paths[i])
            key = f'input-{i}.py'
            originals[key] = path
            sources.append(dict(path=key, source=path.read_text(encoding='utf-8')))
        rows = suite.native(CHECK, 'collector', sources)
        if any(row['clause'] not in {'Surface', 'Materialized'} for row in rows):
            raise AssertionError('Unexpected ServerPlan collector response')
        bundles, grouped = {}, {}
        contents = {file['path']: file['source'] for file in sources}
        for row in rows:
            if row['clause'] == 'Surface':
                grouped.setdefault(row['path'], []).append(row)
        for marker in rows:
            if marker['code'] != 'SPG_FILE':
                continue
            path = originals[marker['path']]
            location = path.relative_to(root).as_posix() if path.is_relative_to(root) else path.as_posix()
            hits = tuple(shared.GrammarSurfaceHit(kind=row['parameters']['kind'], line=row['line'],
                         symbol=row['parameters']['name'], message=row['message'])
                         for row in ordered(grouped.get(marker['path'], [])))
            facts = shared.ServerPlanFileFacts(path=location, grammar_surface_hits=hits)
            source = contents[marker['path']]
            bundles[path] = PerFileFactBundle(path, hashlib.sha256(source.encode()).hexdigest(),
                                             facts={spec.extractor_id: facts.model_dump(mode='json')})
        return ProjectFactView(root, bundles, required_facts)

    def plugin():
        def report(view):
            import json
            from model_transport import encode_models
            from surface_reporter_port import report_rows
            bundles=[dict(path=str(path),value=bundle.facts.get(spec.extractor_id)) for path,bundle in view.files.items()]
            rows=suite.native(CHECK,'_report_facts',[dict(path='reporter-facts.json',
                source=json.dumps(dict(bundles=encode_models(bundles)),ensure_ascii=False))],dict(reportRoot=str(module.ROOT)))
            return report_rows(rows,StructuredViolation)
        return LinterPlugin(CHECK, 'hard-fail', 'Deleted ServerPlan grammar helper surface',
                            FileScopeSpec(broad_repo_scope=True, include_tests=True), (spec,), report)

    for owner in (shared, module):
        suite.replace(owner, 'build_serverplan_fact_engine_view', view)
        suite.replace(owner, 'project_serverplan_facts_from_view', project)
        suite.replace(owner, 'grammar_surface_messages', messages)
    suite.replace(module, 'serverplan_grammar_surface_plugin', plugin)
    def cli_main():
        from text_cli_port import emit
        project_view = module.build_serverplan_fact_engine_view(module.ROOT,
            required_facts=(module.SERVERPLAN_SURFACE_EXTRACTOR,), files=module.iter_python_files(module.ROOT))
        facts = module.project_serverplan_facts_from_view(project_view, module.SERVERPLAN_SURFACE_EXTRACTOR)
        violations = module.grammar_surface_messages(module.ROOT, facts)
        return emit(suite, CHECK, dict(cliFile=violations))
    suite.replace(module, 'cli_main', cli_main)
