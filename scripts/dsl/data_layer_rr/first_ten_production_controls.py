"""Require every rule's positive production contract to reject a disabled collector."""
import argparse
from pathlib import Path
import re
import tempfile
from unittest.mock import patch

import first_ten_production_contract as contract
from migration import ROOT, PIN, read_json, sha, write_json


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['engine', 'baseline', 'out']: parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--part', type=int, default=0); parser.add_argument('--parts', type=int, default=1)
    args = parser.parse_args(); assert 0 <= args.part < args.parts
    source = read_json(args.baseline); assert source['passed'] and source['sourcePin'] == PIN
    paths = [Path(__file__), *[Path(__file__).with_name(n) for n in ['first_ten_production_contract.py', 'replay.py', 'migration.py']]]
    hashes = {p.relative_to(ROOT).as_posix(): sha(p.read_bytes()) for p in paths}
    report = dict(format='rerail.first-ten-production-controls/v1', passed=False, fullMigrationAcceptance=False,
        sourcePin=PIN, engineSha256=sha(args.engine.read_bytes()), baselineSha256=sha(args.baseline.read_bytes()),
        harnessFiles=hashes, part=args.part, parts=args.parts, cases=[])
    with tempfile.TemporaryDirectory(prefix='first-ten-production-controls-', dir=ROOT/'.rerail') as folder:
        control_root = Path(folder)
        for check in contract.CHECKS[args.part::args.parts]:
            relative = Path('gate/data-layer/packs/data-layer/candidates/rr')/(check.replace('_', '-')+'.rr')
            original = (ROOT/relative).read_text()
            pattern = re.compile(r'(?m)^ppe_\w+\([^\n]*? :- [\s\S]*?\.\n')
            changed, count = pattern.subn(lambda m: m.group()[:-2]+', 0 = 1.\n', original)
            assert count > 0 and changed != original
            target = control_root/relative; target.parent.mkdir(parents=True, exist_ok=True); target.write_text(changed)
            case = next(c for c in source['cases'] if c['input']['id'] == check+':roots')
            assert case['expected']['findings']
            with patch.object(contract, 'ROOT', control_root):
                actual, native = contract.native_case(args.engine.absolute(), case['input'])
            rejected = actual != case['expected']
            report['cases'].append(dict(check=check, input=case['input'], expected=case['expected'], actual=actual,
                rejected=rejected, native=native, disabledCollectorClauses=count,
                originalPolicySha256=sha(original.encode()), mutatedPolicy=changed))
            write_json(args.out, report)
    for path, digest in hashes.items(): assert digest == sha((ROOT/path).read_bytes())
    report['passed'] = all(c['rejected'] for c in report['cases']); write_json(args.out, report)
    print(dict(controls=len(report['cases']), passed=report['passed'])); raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__': main()
