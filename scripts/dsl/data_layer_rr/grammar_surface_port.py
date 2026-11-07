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
        return shared.ProjectServerPlanFacts(files=tuple(sorted((
            shared.ServerPlanFileFacts.model_validate(bundle.facts[spec.extractor_id])
            for bundle in view.files.values() if spec.extractor_id in bundle.facts), key=lambda f: f.path)))

    def messages(root, facts):
        return [f"{root / str(file.path)}:{hit.line}: {hit.message}"
                for file in facts.files for hit in file.grammar_surface_hits]

    def view(root, *, required_facts, files=None, fact_store=None, max_workers=None):
        if tuple(required_facts) != (spec,):
            raise AssertionError('Untranslated ServerPlan fact demand')
        root = Path(root)
        paths = files if files is not None else list(root.rglob('*'))
        originals, sources = {}, []
        for i, path in enumerate(paths):
            path = Path(path)
            if not path.is_file() or path.suffix != '.py':
                continue
            if files is None and any(p in {'.git', '.venv', 'venv', '__pycache__', 'tmp'}
                                     for p in path.relative_to(root).parts):
                continue
            key = f'input-{i}.py'
            originals[key] = path
            sources.append(dict(path=key, source=path.read_text(encoding='utf-8')))
        rows = suite.native(CHECK, 'collector', sources)
        if any(row['clause'] not in {'Surface', 'Materialized'} for row in rows):
            raise AssertionError('Unexpected ServerPlan collector response')
        bundles = {}
        for marker in rows:
            if marker['code'] != 'SPG_FILE':
                continue
            path = originals[marker['path']]
            location = path.relative_to(root).as_posix() if path.is_relative_to(root) else path.as_posix()
            hits = tuple(shared.GrammarSurfaceHit(kind=row['parameters']['kind'], line=row['line'],
                         symbol=row['parameters']['name'], message=row['message'])
                         for row in ordered([row for row in rows if row['path'] == marker['path'] and row['clause'] == 'Surface']))
            facts = shared.ServerPlanFileFacts(path=location, grammar_surface_hits=hits)
            source = next(file['source'] for file in sources if file['path'] == marker['path'])
            bundles[path] = PerFileFactBundle(path, hashlib.sha256(source.encode()).hexdigest(),
                                             facts={spec.extractor_id: facts.model_dump(mode='json')})
        return ProjectFactView(root, bundles, required_facts)

    def plugin():
        def report(view):
            return [StructuredViolation(linter_name=CHECK, code='SPG-' + str(hit.kind),
                        path=module.ROOT / str(file.path), line=hit.line, message=str(hit.message))
                    for file in project(view, spec).files for hit in file.grammar_surface_hits]
        return LinterPlugin(CHECK, 'hard-fail', 'Deleted ServerPlan grammar helper surface',
                            FileScopeSpec(broad_repo_scope=True, include_tests=True), (spec,), report)

    for owner in (shared, module):
        suite.replace(owner, 'build_serverplan_fact_engine_view', view)
        suite.replace(owner, 'project_serverplan_facts_from_view', project)
        suite.replace(owner, 'grammar_surface_messages', messages)
    suite.replace(module, 'serverplan_grammar_surface_plugin', plugin)
