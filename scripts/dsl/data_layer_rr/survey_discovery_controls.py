"""Mutation controls for actual directory discovery, not supplied selected files."""
import argparse
import json
from pathlib import Path

from migration import PIN, ROOT, sha, write_json
from survey_discovery_contract import CASES, FORMAT, check_positives, native_case, program
from survey_source_controls import inline


SPECS=[
    ('modern-excludes-api','modern-base-and-layers','sgd_public_path("api") :-',', !sgd_modern()',''),
    ('layer-roots','modern-base-and-layers','sgd_public_path(path) :- sgd_layer',' :- ',' :- 1 = 0, '),
    ('nested-link-pruning','nested-directory-link-pruned','sgd_walk(root, @replace_all','dir_metadata(path, name, _, 0, _)','dir_metadata(path, name, _, _, _)'),
    ('explicit-link-root','explicit-directory-link-root','sgd_walk(root, root) :-',', !sgd_blocked(root)',', dir_metadata(parent, name, _, 0, _), !sgd_blocked(root)'),
    ('explicit-file-bypasses-exclusions','explicit-files-bypass-exclusions','sgd_file(path, path) :-',', suffix(".py", name)',', !sgd_blocked(path), suffix(".py", name)'),
    ('path-components','component-path-order','sgd_key(i, @replace_all', '@replace_all(path, "/", "\\u0000")','path'),
    ('public-deduplication','public-expansion-deduplicates','sgd_chosen(i, j, path) :-','!sgd_earlier_path(i, j, path)','1 = 1'),
    ('explicit-duplicates','explicit-repeated-root','sgd_chosen(i, j, path) :-','!sgd_has_public() ; ',''),
    ('dot-py-has-no-suffix','suffix-and-hidden-files','sgd_file(root, @replace_all',', name != ".py"',''),
    ('explicit-file-exists','broken-explicit-file-link','sgd_file(path, path) :-','dir_metadata(parent, name, 1, _, _)','dir_metadata(parent, name, _, _, _)'),
    ('weighted-assembly-order','weighted-worker-order','sgs_scan_path(path, ordinal) :- balanced_rows','sgs_scan_path(path, ordinal)','sgs_scan_path(path, occurrence)'),
    ('worker-setting','weighted-workers-1','sgd_worker_override(n) :-','@python_int_clamped(text, 1, total, limit)','@python_int_clamped("8", 1, total, limit)'),
    ('file-size-weights','weighted-worker-order','sgd_work(0, i, @replace_all',', bytes) :-',', 0) :-'),
    ('fifty-file-boundary','worker-boundary-51','sgs_scan_path(path, ordinal) :- sgd_count','total <= 50','total <= 51'),
]


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['engine','baseline','output']:p.add_argument('--'+name,type=Path,required=True)
    args=p.parse_args();baseline=json.loads(args.baseline.read_text())
    if baseline['format']!=FORMAT or baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:p.error('Complete pinned recording required')
    if [{k:c[k] for k in CASES[0]} for c in baseline['cases']]!=CASES:p.error('Complete unchanged corpus required')
    check_positives(baseline['cases']);cases={c['id']:c for c in baseline['cases']}
    result=dict(format='rerail.survey-discovery-controls/v1',sourcePin=PIN,fullMigrationAcceptance=False,
                baselineSha256=sha(args.baseline.read_bytes()),engineSha256=sha(args.engine.read_bytes()),controls=[],complete=False,passed=False)
    write_json(args.output,result)
    source=args.output.parent/'controls/base.rr';source.parent.mkdir(parents=True,exist_ok=True);source.write_text(program())
    source=inline(source)
    for name,label,prefix,before,after in SPECS:
        lines=[];changed=[]
        for line in source.splitlines():
            replacement=line.replace(before,after,1) if line.startswith(prefix) else line
            if replacement!=line:changed.append(dict(before=line,after=replacement))
            lines.append(replacement)
        if len(changed)!=1:raise ValueError(f'{name}: expected one changed production clause, got {len(changed)}')
        rule=args.output.parent/'controls'/f'{name}.rr';rule.write_text('\n'.join(lines)+'\n')
        actual,inputs=native_case(args.engine,rule,cases[label])
        if actual==cases[label]['expected']:raise AssertionError(f'{name}: mutation survived')
        result['controls'].append(dict(name=name,case=label,changed=changed,expected=cases[label]['expected'],actual=actual,
                                       program=rule.read_text(),programInputs=inputs))
        write_json(args.output,result)
        print(f'{name}: complete result mismatch through successful native execution',flush=True)
    result.update(complete=True,passed=len(result['controls'])==len(SPECS));write_json(args.output,result)


if __name__=='__main__':main()
