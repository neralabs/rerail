"""Run the approved full data-layer cold workloads; counts are not an acceptance gate."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import time
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import run

ROOT = Path(__file__).resolve().parents[3]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def resource_usage(path):
    result = {}
    for line in path.read_text().splitlines():
        match = re.fullmatch(r'(real|user|sys)\s+([\d.]+)', line.strip())
        if match:
            result[match[1] + 'Seconds'] = float(match[2])
        match = re.fullmatch(r'\s*(\d+)\s+maximum resident set size\s*', line)
        if match:
            result['peakRssBytes'] = int(match[1])
    if 'userSeconds' in result and 'sysSeconds' in result:
        result['cpuSeconds'] = result['userSeconds'] + result['sysSeconds']
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--addon', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--policies', type=Path, default=Path('packs/data-layer/candidates/rules'))
    parser.add_argument('--recordings', type=Path, default=Path('.rerail/profiles/full-repository-20261021'))
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    frozen = output / 'artifact'
    frozen.mkdir()
    shutil.copyfile(args.addon, frozen / 'engine.node')
    policies = frozen / 'policies'
    policies.mkdir()
    for policy in args.policies.glob('*.json'):
        shutil.copyfile(policy, policies / policy.name)
    driver = ROOT / 'conformance/data-layer/suite.ts'
    manifest = {'format': 'rerail.full-repository-measurement/v1',
                'nativeSha256': digest(frozen / 'engine.node'), 'driverSha256': digest(driver),
                'policySha256': {p.name: digest(p) for p in sorted(policies.glob('*.json'))},
                'acceptance': 'Full-repository cold CPU, wall, RSS and complete findings parity; compile reported separately. SQL counts are diagnostic.',
                'deadline': None, 'workers': 4, 'batchFiles': 32, 'detailedTracing': False,
                'sqlCapture': False, 'memoryCensus': False, 'phaseMarkers': True,
                'buildDescription': 'Matched historical reference: engine opt-level 1, dependencies 3, LTO off, codegen units 256, code-rule-profile, archived system-malloc shim.',
                'runs': []}
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    observer = SimpleNamespace(sql_profile=False, query_plans=False, trace='off', memory_every=6,
                               memory_tool='none', memory_count=0, external='none', sample_count=0,
                               sample_every=6, sample_seconds=1, process_every=5)
    for key in list(os.environ):
        if key.startswith('RERAIL_PROFILE') or key in ['RERAIL_SQL_PROFILE_DIR', 'RERAIL_SQL_QUERY_PLANS', 'RERAIL_CODE_RULE_RELATIONAL_PROFILE', 'RERAIL_CODE_RULE_ANCHORS']:
            os.environ.pop(key)
    os.environ['RERAIL_ENGINE_PATH'] = str(frozen / 'engine.node')
    os.environ['RERAIL_NATIVE_WORKERS'] = '4'
    os.environ['DYLD_INSERT_LIBRARIES'] = '/tmp/librerail_jemalloc_check_shim.dylib'
    manifest['allocatorSha256'] = digest(Path(os.environ['DYLD_INSERT_LIBRARIES']))
    for name, rules in [('first-50', 50), ('all-60', 60)]:
        recording = args.recordings.resolve() / (name + '-recording.json')
        data = json.loads(recording.read_text())
        if len(data['files']) != 9021 or len(data['events']) != rules or data['source_mismatches']:
            raise ValueError('Full corpus or rule-set identity mismatch')
        directory = output / name
        directory.mkdir()
        command = ['/usr/bin/time', '-lp', '-o', str(directory / 'resource.txt'),
                   'node', '--import', 'tsx', str(driver), '--recording', str(recording),
                   '--output', str(directory / 'parity.json'), '--policies', str(policies),
                   '--allocator', 'system-malloc-shim', '--cold-only', '--progress']
        receipt = {'name': name, 'rules': rules, 'files': len(data['files']),
                   'bytes': sum(len(f['source'].encode()) for f in data['files']),
                   'expectedFindings': sum(len(e['findings']) for e in data['events']),
                   'recordingSha256': digest(recording), 'command': command,
                   'loadAverageStart': os.getloadavg(), 'startedUnixSeconds': time.time(), 'status': 'running'}
        manifest['runs'].append(receipt)
        (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        print(json.dumps({'starting': name, 'pid': os.getpid(), 'output': str(directory)}), flush=True)
        started = time.monotonic()
        code, timed_out, _, process, _ = run.execute(observer, command, directory, float('inf'))
        receipt.update(exitCode=code, timedOut=timed_out, elapsedSeconds=time.monotonic() - started,
                       endedUnixSeconds=time.time(), process=process, resource=resource_usage(directory / 'resource.txt'),
                       loadAverageEnd=os.getloadavg(), status='complete')
        parity_path = directory / 'parity.json'
        if parity_path.exists():
            parity = json.loads(parity_path.read_text())
            receipt['result'] = {key: parity[key] for key in ['files', 'bytes', 'expected', 'actual', 'peakRssBytes', 'compilationMs', 'coldMs', 'phaseCpuMs']}
            receipt['result'].update(falsePositives=len(parity['falsePositives']), falseNegatives=len(parity['falseNegatives']), failures=len(parity['failures']))
        (directory / 'invocation.json').write_text(json.dumps(receipt, indent=2) + '\n')
        (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        print(json.dumps({'finished': name, 'exitCode': code, 'elapsedSeconds': receipt['elapsedSeconds'], 'result': receipt.get('result')}), flush=True)
    return int(any(r['exitCode'] != 0 for r in manifest['runs']))


if __name__ == '__main__':
    raise SystemExit(main())
