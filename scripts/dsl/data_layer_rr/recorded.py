"""Verify complete, nonoverlapping detector replay ranges; keep this separate from full acceptance."""
import argparse
import gzip
import json
from pathlib import Path
from migration import ROOT, MANIFEST, compare, read_json, write_json, sha
from replay import expected_result


def summarize(reports, corpora):
    manifest = read_json(ROOT / MANIFEST)
    rules = {entry['check']: entry for entry in manifest['rules']}
    corpus_by_sha = {}
    for path in corpora:
        raw = path.read_bytes()
        corpus_by_sha[sha(raw)] = json.loads(gzip.decompress(raw) if path.suffix == '.gz' else raw)
    grouped = {}
    for path in reports:
        report = read_json(path)
        if report['corpusSha256'] not in corpus_by_sha:
            raise ValueError(f'{path}: unbound corpus')
        check = next((name for name, rule in rules.items() if rule['rulePath'] == report['rule']), None)
        if check is None or report['ruleSha256'] != rules[check]['ruleSha256']:
            raise ValueError(f'{path}: stale or unknown rule')
        if not report.get('programInputs'):
            raise ValueError(f'{path}: include provenance is missing')
        for file in report['programInputs']:
            current = ROOT / file['path']
            digest = sha(current.read_bytes()) if current.is_file() else None
            if digest != file['sha256']:
                raise ValueError(f'{path}: stale included source {file["path"]}')
        group = grouped.setdefault(check, dict(engineSha256=report['engineSha256'], corpusSha256=report['corpusSha256'], programInputs=report['programInputs'], results={}))
        if any(group[key] != report[key] for key in ['engineSha256', 'corpusSha256', 'programInputs']):
            raise ValueError(f'{check}: replay ranges use different engines or corpora')
        corpus = corpus_by_sha[report['corpusSha256']]
        originals = [record for record in corpus.get('records', corpus.get('events', [])) if record['check'] == check]
        for row in report['results']:
            index = row['index']
            if index in group['results']:
                raise ValueError(f'{check}: overlapping replay range at {index}')
            original = originals[index]
            if row['test'] != original['test'] or row['entry'] != original['entry']:
                raise ValueError(f'{check}: replay case identity changed')
            if row['status'] != 'passed':
                raise ValueError(f'{check}: failing replay at {index}')
            compare(expected_result(original), row['expected'])
            if row['positive'] != bool(expected_result(original)):
                raise ValueError(f'{check}: incorrect positive classification at {index}')
            compare(row['expected'], row['actual'])
            group['results'][index] = {k: value for k, value in row.items() if k != 'measurements'}
        group['selectedCases'] = len(originals)
    result = []
    for check, group in sorted(grouped.items()):
        if set(group['results']) != set(range(group['selectedCases'])):
            raise ValueError(f'{check}: omitted recorded detector calls')
        rows = [group['results'][i] for i in range(group['selectedCases'])]
        result.append(dict(check=check, status='recorded-detector-parity',
                           ruleSha256=rules[check]['ruleSha256'], engineSha256=group['engineSha256'],
                           corpusSha256=group['corpusSha256'], observations=len(rows),
                           programInputs=group['programInputs'],
                           positives=sum(row['positive'] for row in rows), results=rows))
    return dict(format='rerail.rr-recorded-detector-parity/v1', sourceCommit=manifest['sourceCommit'],
                scope='Recorded detector DTO parity only; excludes helper, host, configured, mutation and persistent-workspace acceptance',
                rules=result)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reports', type=Path, nargs='+', required=True)
    parser.add_argument('--corpora', type=Path, nargs='+', required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    report = summarize(args.reports, args.corpora)
    write_json(args.out, report)
    print(json.dumps([{k: row[k] for k in ['check', 'observations', 'positives']} for row in report['rules']], indent=2))
