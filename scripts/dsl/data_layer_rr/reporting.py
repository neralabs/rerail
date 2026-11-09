"""Compare complete plugin reporter values with native diagnostics and compiled metadata.

This covers snapshot reporting. It does not certify the production frontend or persistent workspaces.
"""
import argparse
import importlib
import json
from pathlib import Path
import sys
import tempfile

from migration import ROOT, PIN, git, sha, read_json, write_json, compare
from replay import execute_native
from fresh import selected_cases


def reporter_file_groups(rows):
    """Ignore worker completion order between files; retain each file's complete ordered sequence."""
    return sorted(rows, key=lambda row: row['path'])


def run(args):
    harness_root = ROOT / 'scripts/dsl'
    harness_before = {p.resolve(): sha(p.read_bytes()) for p in harness_root.rglob('*.py')}
    corpus = read_json(args.cases)
    native = args.command == 'native'
    if not native and (args.part, args.parts) != (0, 1):
        raise ValueError('Record the complete reporter source oracle before native sharding')
    selected = selected_cases(corpus['cases'], args.part, args.parts)
    oracle = read_json(args.oracle) if native else None
    report = dict(format='rerail.rr-plugin-report-contract/v1', fullMigrationAcceptance=False,
        originalPytestCases=0, execution='native-snapshot' if native else 'offline-source', sourcePin=PIN,
        casesSha256=sha(args.cases.read_bytes()), results=[], part=args.part, parts=args.parts,
        totalCases=len(corpus['cases']), selectedCaseIndices=[index for index, _ in selected])
    modules = {}
    if native:
        engine = args.engine.resolve()
        report['engineSha256'] = sha(engine.read_bytes())
        report['oracleSha256'] = sha(args.oracle.read_bytes())
        if oracle['sourcePin'] != PIN or oracle['casesSha256'] != report['casesSha256'] or oracle['execution'] != 'offline-source':
            raise ValueError('Reporter oracle belongs to different source or cases')
        if any(row['status'] != 'passed' for row in oracle['results']) or len(oracle['results']) != len(corpus['cases']):
            raise ValueError('Incomplete reporter oracle')
    else:
        source = args.source.resolve()
        if git(source, 'rev-parse', 'HEAD') != PIN:
            raise ValueError('Wrong source pin')
        sys.path[:0] = [str(source), str(source / 'scripts')]
        modules = {check: importlib.import_module('scripts.lint_' + check) for check in {c['check'] for c in corpus['cases']}}
        from scripts.lint_lib.fact_engine import FactEngine
        from scripts.lint_lib.fact_store import JsonFactStore
    paths = [Path(__file__), *[Path(__file__).with_name(n) for n in ['migration.py', 'replay.py']]]
    report['harnessFiles'] = {p.relative_to(ROOT).as_posix(): sha(p.read_bytes()) for p in paths}
    for index, case in selected:
        report['activeCaseId'] = case['id']
        write_json(args.out, report)
        if native:
            rule = args.rule_root / (case['check'].replace('_', '-') + '.rr')
            options=({'repositoryRoot':str(args.source.resolve())} if case['check']=='transaction_callback_wrappers' else {})
            if case['check']=='sql_grammar_escape_hatches':
                options['sourceModulePath']=str(args.source.resolve()/'scripts/lint_sql_grammar_escape_hatches.py')
            result = execute_native(engine, rule, case['files'], options, workspace_name=case.get('workspaceName', 'corpus'))
            report['sourceDetectorModules'] = [name for name in sys.modules if name.startswith('scripts.lint_')]
            if report['sourceDetectorModules']:
                raise ValueError('The native reporter imported source detectors')
            metadata = result['metadata']
            if len(metadata['policies']) != 1:
                raise ValueError('Reporter contract requires one policy')
            policy = metadata['policies'][0]
            modes = [text for key, text in metadata['settings'] if key == 'mode']
            if modes not in [['block'], ['log']]:
                raise ValueError('No source severity adapter for the compiled mode: ' + repr(modes))
            identity = policy['id']
            prefix = next((p for p in ['data-layer-', 'data-layer:'] if identity.startswith(p)), None)
            if prefix is None or any(r['policy'] != identity for r in result['diagnostics']):
                raise ValueError('Unexpected native policy identity')
            severities = {}
            for key, text in metadata['settings']:
                if key == 'severity':
                    words = text.split()
                    if len(words) != 2:
                        raise ValueError('Unsupported compiled severity metadata: ' + text)
                    severities[words[0]] = words[1]
            native_rows = result['diagnostics']
            if case['check'] == 'backend_type_strings':
                native_rows = sorted(native_rows, key=lambda r: (r['path'], r['line'], r['code'],
                    r['parameters']['pattern'], r['parameters']['occurrence']))
            if case['check'] == 'removed_compat_surfaces':
                native_rows = sorted(native_rows, key=lambda r: (r['path'], r['parameters']['ordinal']))
            if case['check'] == 'serverplan_grammar_surface':
                native_rows = sorted(native_rows, key=lambda r: (r['path'], r['parameters']['ordinal'], r['parameters']['witness']))
            if case['check'] == 'conn_or_txn_union':
                native_rows = sorted(native_rows, key=lambda r: (r['path'], r['parameters'].get('ordinal', -1)))
            if case['check'] == 'async_patterns':
                native_rows = sorted(native_rows, key=lambda r: (r['path'], r['parameters']['ordinal'], r['parameters']['rank']))
            if case['check'] == 'async_sync_boundary':
                native_rows = sorted(native_rows, key=lambda r: (r['path'], r['parameters']['ordinal']))
            if case['check']=='plan_scope_ownership':
                from plan_ownership_port import ordered
                native_rows=ordered(native_rows)
            if case['check']=='transaction_closures':
                from transaction_port import ordered
                native_rows=ordered(native_rows)
            if case['check']=='transaction_callback_wrappers':
                from callback_wrappers_port import ordered
                native_rows=ordered(native_rows)
            if case['check']=='transaction_patterns':
                from patterns_port import ordered
                native_rows=ordered(native_rows)
            if case['check']=='lazy_result_usage':
                native_rows=sorted(native_rows,key=lambda r:(r['path'],r['line'],r['parameters']['exit_order'],-r['parameters']['entry_order']))
            if case['check']=='regex_sql_parsing':
                native_rows=sorted(native_rows,key=lambda r:'regex_sql_parsing:'+r['code']+':'+r['path']+':'+str(r['line'])+':'+r['message'])
            if case['check']=='grammar_allowlist_burndown':
                native_rows=sorted(native_rows,key=lambda r:r['parameters']['spec_ordinal'])
            if case['check']=='grammar_fragment_usage':
                from grammar_fragment_usage_port import ordered
                native_rows=ordered(native_rows)
            if case['check']=='grammar_callable_constructors':
                native_rows=sorted(native_rows,key=lambda r:(r['path'],r['line'],r['code'],r['message']))
            if case['check']=='grammar_str_params':
                native_rows=sorted(native_rows,key=lambda r:(r['path'],r['parameters'].get('ordinal',0),r['parameters'].get('first',0),r['parameters'].get('second',0)))
            if case['check']=='grammar_render_methods':
                native_rows=sorted(native_rows,key=lambda r:(r['path'],r['parameters'].get('ordinal',0)))
            if case['check']=='sql_grammar_escape_hatches':
                native_rows=sorted(native_rows,key=lambda r:(r['path'],r['line'],r['parameters'].get('phase',0),r['parameters'].get('ordinal',0)))
            if case['check']=='render_raw_strings':
                native_rows=sorted(native_rows,key=lambda r:(r['path'],r['line'],r['code'],r['parameters']['ordinal']))
            if case['check']=='render_fstrings':
                native_rows=sorted(native_rows,key=lambda r:(r['path'],r['line'],r['parameters']['ordinal']))
            if case['check']=='module_level_caches':
                native_rows=sorted(native_rows,key=lambda r:(r['path'],r['line'],r['code'],r['parameters'].get('visit',0),r['parameters'].get('suborder',0)))
            actual = [dict(linter_name=identity.removeprefix(prefix).replace('-', '_'), code=r['code'],
                path=r['path'], line=r['line'], message=r['message'],
                severity=severities.get(r['clause'], 'info' if modes == ['log'] else 'hard-fail')) for r in native_rows]
            if case['check']=='module_level_caches':
                uncaught=[r for r in native_rows if r['code']=='MC_UNCAUGHT_CONFIG']
                if uncaught:
                    if len(native_rows)!=1 or uncaught[0]['parameters']['kind']!='KeyError':
                        raise ValueError('No reporter exception transport for this native result')
                    actual=dict(error=dict(type='builtins.KeyError',args=dict(kind='tuple',items=[uncaught[0]['parameters']['value']])))
            program = result['programInputs']
            phases = result['phases']
        else:
            with tempfile.TemporaryDirectory(prefix='rr-source-report-') as temporary:
                root = Path(temporary)
                if 'workspaceName' in case:
                    name = case['workspaceName']
                    if Path(name).name != name or name in {'', '.', '..'}:
                        raise ValueError('Invalid fixture workspace name')
                    root = root / name
                    root.mkdir()
                for file in case['files']:
                    path = root / file['path']
                    if path.is_absolute() and not path.is_relative_to(root) or '..' in Path(file['path']).parts:
                        raise ValueError('Invalid reporter fixture path')
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(file['source'])
                module = modules[case['check']]
                if case['check']=='raw_execute_regression':
                    module.REPO_ROOT=root
                if case['check'] == 'lazy_result_usage':
                    module.BASELINE_FILE = root / 'scripts/lazy_result_baseline.json'
                if case['check'] == 'module_level_caches':
                    module.ALLOWLIST_FILE = root / 'scripts/allowlists/lint_module_level_caches_allowlist.json'
                if case['check'] == 'serverplan_grammar_surface':
                    from scripts.lint_lib.serverplan_facts import serverplan_fact_extractors
                    module.ROOT = root
                    plugin = module.serverplan_grammar_surface_plugin()
                    extractors = serverplan_fact_extractors(root)
                else:
                    plugin = getattr(module, case['check'] + '_plugin')(root)
                    extractor_name={'transaction_closures':'transaction_closure','transaction_callback_wrappers':'transaction_callback_wrapper','transaction_patterns':'transaction_pattern'}.get(case['check'],case['check'])
                    extractors = getattr(module, extractor_name + '_fact_extractors')(root)
                fact_engine = FactEngine(root=root, fact_store=JsonFactStore(root / 'tmp/lint_fact_store'),
                    extractors=extractors)
                view = fact_engine.materialize((plugin,))
                try:
                    actual = [dict(linter_name=v.linter_name, code=v.code,
                        path=v.path.relative_to(root).as_posix() if v.path.is_absolute() else v.path.as_posix(),
                        line=v.line, message=v.message, severity=v.severity) for v in plugin.reporter(view)]
                except KeyError as error:
                    if case['check']!='module_level_caches':
                        raise
                    actual=dict(error=dict(type='builtins.KeyError',args=dict(kind='tuple',items=list(error.args))))
                if case['check'] == 'module_level_caches' and isinstance(actual,list):
                    for finding in actual:
                        finding['message'] = finding['message'].replace(str(root)+'/', '')
            program, phases = None, None
        # Keep the source reporter order. Adapt native rows to that interface's order.
        if native and case['check'] not in {'removed_compat_surfaces', 'serverplan_grammar_surface', 'conn_or_txn_union','transaction_closures','transaction_callback_wrappers','transaction_patterns','lazy_result_usage','module_level_caches','render_fstrings','render_raw_strings','regex_sql_parsing','sql_grammar_escape_hatches','grammar_render_methods','grammar_str_params','grammar_callable_constructors','grammar_fragment_usage','grammar_allowlist_burndown'}:
            last = {'raw_execute_regression':'message','backend_type_strings': 'code', 'error_handling': 'message', 'no_api_root_imports': 'message', 'connection_caches': 'message', 'backend_coupling': 'code', 'type_safety_patterns': 'message', 'sync_async_signatures': 'message', 'data_layer_facade_purity': 'code', 'forwardref': 'code', 'unresolved_forward_refs': 'message', 'data_layer_compat_aliases': 'code', 'hnsw_architecture': 'code', 'async_patterns': 'code', 'async_sync_boundary': 'code', 'plan_scope_ownership':'code'}[case['check']]
            actual.sort(key=lambda row: (row['path'], row['line'], row[last], *([row['message']] if case['check'] == 'hnsw_architecture' else [])))
        row = dict(id=case['id'], value=actual, status='passed', programInputs=program, phases=phases)
        if native:
            row['metadata'] = metadata
        if case['check']=='raw_execute_regression':
            if native:
                description=[text for key,text in metadata['settings'] if key=='description']
                if len(description)!=1:raise ValueError('Raw-execute metadata requires one description')
                row['sourcePlugin']=dict(name=identity.removeprefix(prefix).replace('-','_'),
                                         enforcement={'block':'hard-fail','log':'info'}[modes[0]],description=json.loads(description[0]))
            else:
                row['sourcePlugin']=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description)
        report['results'].append(row)
        if native:
            expected = oracle['results'][index]
            if expected['id'] != case['id']:
                raise ValueError('Reporter case identity changed')
            try:
                if case['check']=='raw_execute_regression':compare(expected['sourcePlugin'],row['sourcePlugin'])
                if case['check'] in {'hnsw_architecture', 'removed_compat_surfaces'}:
                    row['comparisonContract'] = 'File groups are unordered (source FactEngine.as_completed); every in-file value, duplicate, and order is exact'
                    compare(reporter_file_groups(expected['value']), reporter_file_groups(actual))
                else:
                    compare(expected['value'], actual)
            except AssertionError as error:
                row.update(status='failed', expected=expected['value'])
                write_json(args.out, report)
                raise AssertionError(case['id'] + ': ' + str(error)) from error
    report.pop('activeCaseId', None)
    if native:
        if sha(engine.read_bytes()) != report['engineSha256']:
            raise ValueError('Reporter executable changed')
        report['sourceDetectorModules'] = [name for name in sys.modules if name.startswith('scripts.lint_')]
        if report['sourceDetectorModules']:
            raise ValueError('The native reporter imported source detectors')
    else:
        from data_layer_probe_record import verify_sources
        report['sourceFilesChecked'] = verify_sources(source, source, PIN)
        report['sourceFiles'] = {f'scripts/lint_{check}.py': sha((source / f'scripts/lint_{check}.py').read_bytes()) for check in modules}
    for imported in tuple(sys.modules.values()):
        filename = getattr(imported, '__file__', None)
        if filename and Path(filename).suffix == '.py':
            path = Path(filename).resolve()
            if path.is_relative_to(harness_root):
                if path not in harness_before:
                    raise ValueError('Reporter imported new code during the run')
                report['harnessFiles'].setdefault(path.relative_to(ROOT).as_posix(), harness_before[path])
    if any(sha((ROOT / path).read_bytes()) != digest for path, digest in report['harnessFiles'].items()):
        raise ValueError('Reporter harness changed')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['source', 'native'])
    parser.add_argument('--source', type=Path)
    parser.add_argument('--engine', type=Path)
    parser.add_argument('--oracle', type=Path)
    parser.add_argument('--cases', type=Path, required=True)
    parser.add_argument('--rule-root', type=Path, default=ROOT / 'gate/data-layer/packs/data-layer/candidates/rr',
                        help='Policy directory; mutation controls supply disabled policies')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--part', type=int, default=0)
    parser.add_argument('--parts', type=int, default=1)
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.rule_root = args.rule_root.resolve()
    report = run(args)
    write_json(args.out, report)
    print(json.dumps(dict(cases=len(report['results']), execution=report['execution'], fullMigrationAcceptance=False)))


if __name__ == '__main__':
    main()
