"""Discover original suites independently, then collect complete modules in bounded shards."""
import argparse
import ast
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from migration import ROOT, PIN, MANIFEST, git, read_json, sha, write_json


def modules_for(source, checks):
    owners = {'scripts.lint_' + check: {check} for check in checks}
    registry = ast.parse((source / 'scripts/lint_all.py').read_text())
    imported = {alias.asname or alias.name: node.module for node in ast.walk(registry)
                if isinstance(node, ast.ImportFrom) for alias in node.names}
    for node in ast.walk(registry):
        if not isinstance(node, ast.Call):
            continue
        values = {arg.arg: arg.value.value for arg in node.keywords
                  if isinstance(arg.value, ast.Constant) and isinstance(arg.value.value, str)}
        check = values.get('name')
        if check not in checks:
            continue
        script = values.get('script')
        if script:
            owners.setdefault('scripts.' + Path(script).stem, set()).add(check)
        plugin = values.get('plugin_name', check) + '_plugin'
        if plugin in imported:
            owners.setdefault(imported[plugin], set()).add(check)
    owners.setdefault('scripts.check_versioned_merged_identity', set()).add('versioned_merged_identity')
    owners.setdefault('scripts.verify_cached_pure_lazy_allowlist', set()).add('cached_pure_lazy_allowlist')
    owners.setdefault('scripts.conditional_architecture_facts', set()).add('conditional_architecture')
    return {module: values & set(checks) for module, values in owners.items()}


def references(text, owners, checks, script_modules=()):
    tree = ast.parse(text)
    modules, strings = set(), []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            modules.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom):
            modules.add(node.module or '')
            modules.update((node.module + '.' if node.module else '') + alias.name for alias in node.names)
        elif isinstance(node, ast.Constant) and isinstance(node.value, str):
            strings.append(node.value)
    detectors = set()
    for module, checks_for_module in owners.items():
        aliases = {module, module.removeprefix('scripts.')}
        if modules & aliases or any(any(value == alias or value.startswith(alias + '.') for alias in aliases) for value in strings):
            detectors.update(checks_for_module)
    named = set(checks) & set(strings)
    integration = named if 'scripts.lint_all' in modules else set()
    # A bare status such as "permission_error" is ordinary application data.
    # Named checks need an actual linter-module import template to own a suite.
    dynamic_linter_import = any(value in {'scripts.lint_', 'lint_', 'scripts/lint_'} for value in strings)
    framework_context = (dynamic_linter_import or any(m.startswith('scripts.') or m in script_modules for m in modules)
                         or any(value == 'scripts' or value.startswith('scripts/') for value in strings))
    if framework_context:
        detectors.update(named - integration)
    adjacent = {check for check in checks if any(re.search(r'(?<!\w)(?:scripts/)?lint_' + re.escape(check) + r'\.py\b', value) for value in strings)}
    return dict(detectors=sorted(detectors), integrationReferences=sorted(integration - detectors),
                workflowReferences=sorted(adjacent - detectors))


def discover(source):
    if git(source, 'rev-parse', 'HEAD') != PIN:
        raise ValueError('Wrong source pin')
    checks = [r['check'] for r in read_json(ROOT / MANIFEST)['rules']]
    owners = modules_for(source, checks)
    entries = subprocess.check_output(['git', '-C', str(source), 'ls-tree', '-rz', PIN]).split(b'\0')
    script_modules = set()
    for entry in entries:
        if not entry: continue
        name = entry.split(b'\t', 1)[1].decode()
        if name.startswith('scripts/') and name.endswith('.py'):
            script = '.'.join(Path(name).with_suffix('').parts)
            script_modules.update({script, script.removeprefix('scripts.')})
    files, scanned = {}, 0
    for entry in entries:
        if not entry:
            continue
        meta, name = entry.split(b'\t', 1)
        name = name.decode()
        path = Path(name)
        if path.suffix != '.py' or not (path.name.startswith('test_') or path.name.endswith('_test.py')):
            continue
        raw = (source / path).read_bytes()
        scanned += 1
        if hashlib.sha1(b'blob ' + str(len(raw)).encode() + b'\0' + raw).hexdigest() != meta.split()[2].decode():
            raise ValueError('Changed original module: ' + name)
        found = references(raw.decode(), owners, checks, script_modules)
        if any(found.values()):
            files[name] = dict(**found, sha256=sha(raw))
    return dict(format='rerail.rr-original-suite-discovery/v1', sourcePin=PIN, checks=checks,
                scannedTestFiles=scanned, files=files, sourceModules={k: sorted(v) for k, v in owners.items() if v})


def collect(source, manifest, part, parts):
    if manifest['sourcePin'] != PIN or git(source, 'rev-parse', 'HEAD') != PIN or not 0 <= part < parts:
        raise ValueError('Wrong source pin or collection shard')
    files = sorted(manifest['files'])[part::parts]
    collector = Path(__file__).with_name('collect_file.py')
    with tempfile.TemporaryDirectory(prefix='rerail-rr-inventory-') as temporary:
        def one(item):
            index, name = item
            if sha((source / name).read_bytes()) != manifest['files'][name]['sha256']:
                raise ValueError('Changed original module: ' + name)
            output = Path(temporary) / f'{index}.json'
            result = subprocess.run([sys.executable, str(collector), str(source), name, str(output)],
                capture_output=True, text=True, timeout=100,
                env={**os.environ, 'PYTEST_ADDOPTS': '', 'PYTHONHASHSEED': '0'})
            if result.returncode:
                raise ValueError(name + ': original collection failed\n' + result.stdout + result.stderr)
            return name, read_json(output)
        with ThreadPoolExecutor(max_workers=2) as workers:
            rows = list(workers.map(one, enumerate(files)))
    return dict(format='rerail.rr-original-suite-collection/v1', sourcePin=PIN, part=part, parts=parts,
                discoverySha256=sha(json.dumps(manifest, sort_keys=True).encode()),
                collectorSha256=sha(collector.read_bytes()), files=dict(rows))


def assemble(manifest, shards):
    files, cases, adjacent, integration = {}, {}, {}, {}
    expected_parts = shards[0]['parts'] if shards else 0
    if len(shards) != expected_parts or {r['part'] for r in shards} != set(range(expected_parts)):
        raise ValueError('Collection shards are missing or duplicated')
    for shard in shards:
        if shard['parts'] != expected_parts or shard['sourcePin'] != PIN or shard['discoverySha256'] != sha(json.dumps(manifest, sort_keys=True).encode()) or shard['collectorSha256'] != shards[0]['collectorSha256']:
            raise ValueError('Collection shard belongs to another discovery')
        if set(shard['files']) != set(sorted(manifest['files'])[shard['part']::expected_parts]):
            raise ValueError('Collection shard omitted original modules')
        for name, collected in shard['files'].items():
            if name in files or collected['exitCode'] or collected['errors'] or collected['deselected'] or set(collected['before']) != set(collected['cases']):
                raise ValueError('Incomplete original collection: ' + name)
            if len(collected['before']) != len(collected['cases']) or any(
                    identity.split('::', 1)[0] != name or case['file'] != name
                    for identity, case in collected['cases'].items()):
                raise ValueError('Original case belongs to another module: ' + name)
            files[name] = dict(**manifest['files'][name], cases=len(collected['cases']))
            target = cases if files[name]['detectors'] else integration if files[name]['integrationReferences'] else adjacent
            if set(target) & set(collected['cases']):
                raise ValueError('Duplicate original cases')
            target.update(collected['cases'])
    if set(files) != set(manifest['files']):
        raise ValueError('Original modules are missing')
    return dict(format='rerail.data-layer-original-test-inventory/v1', sourcePin=PIN,
                scannedTestFiles=manifest['scannedTestFiles'], unparsedTestFiles=[], files=files, cases=cases,
                adjacentWorkflowCases=adjacent, suiteIntegrationCases=integration,
                discoverySha256=shards[0]['discoverySha256'], collectorSha256=shards[0]['collectorSha256'],
                note='Independent whole-module collection. Counts and static assertion sites do not certify native execution.')


def reclassify(prior, discovery):
    """Reuse unchanged case collections after reviewing file ownership evidence."""
    if prior['sourcePin'] != PIN or discovery['sourcePin'] != PIN or prior.get('unparsedTestFiles'):
        raise ValueError('Ownership review requires a complete collection at the source pin')
    if set(discovery['files']) - set(prior['files']):
        raise ValueError('Newly discovered modules require fresh pytest collection')
    all_cases = {}
    for key in ['cases', 'suiteIntegrationCases', 'adjacentWorkflowCases']:
        for identity, case in prior.get(key, {}).items():
            if identity in all_cases: raise ValueError('Original case occurs in multiple categories')
            all_cases[identity] = case
    files, cases, integration, adjacent, review = {}, {}, {}, {}, []
    for name, old in prior['files'].items():
        new = discovery['files'].get(name)
        if new is None or old['detectors'] != new['detectors']:
            review.append(dict(file=name, before=old['detectors'], after=new['detectors'] if new else [],
                cases=old['cases'], sourceSha256=old['sha256']))
        if new is None: continue
        if old['sha256'] != new['sha256']:
            raise ValueError('Original module changed; collect it again: '+name)
        selected = {identity: case for identity,case in all_cases.items() if identity.split('::',1)[0] == name}
        if len(selected) != old['cases']: raise ValueError('Prior collection omitted cases: '+name)
        files[name] = dict(new, cases=len(selected))
        target = cases if new['detectors'] else integration if new['integrationReferences'] else adjacent
        target.update(selected)
    return dict(format=prior['format'], sourcePin=PIN, scannedTestFiles=discovery['scannedTestFiles'],
        unparsedTestFiles=[], files=files, cases=cases, suiteIntegrationCases=integration,
        adjacentWorkflowCases=adjacent, discoverySha256=sha(json.dumps(discovery,sort_keys=True).encode()),
        collectorSha256=prior['collectorSha256'], ownershipReview=review,
        note='Unchanged independent pytest collections; ownership now requires a module, script, or linter-framework context. Historical inventory is retained.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['discover', 'collect', 'assemble'])
    parser.add_argument('--source', type=Path)
    parser.add_argument('--manifest', type=Path)
    parser.add_argument('--part', type=int, default=0)
    parser.add_argument('--parts', type=int, default=1)
    parser.add_argument('--shards', type=Path, nargs='+')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    result = (discover(args.source.resolve()) if args.command == 'discover' else
              collect(args.source.resolve(), read_json(args.manifest), args.part, args.parts) if args.command == 'collect' else
              assemble(read_json(args.manifest), [read_json(p) for p in args.shards]))
    write_json(args.out, result)
    print(json.dumps({k: len(v) if isinstance(v, (list, dict)) else v for k, v in result.items() if k in ['files', 'cases', 'scannedTestFiles', 'part']}))


if __name__ == '__main__':
    main()
