"""Preserve complete executor parity evidence without granting full admission."""
import argparse
from collections import Counter,defaultdict
import gzip
import json
from pathlib import Path
from migration import ROOT,PIN,MANIFEST,read_json,sha
from scanner_checkpoint import walk
from validate_supplied_facts_checkpoint import require,equal

CHECK='async_executor'
MODES=('snapshot','workspace')
COUNTS=dict(public=61,components=32,production=15,cli=28,legacy=50)
PARTS=dict(public=4,components=4,production=2,cli=4,legacy=4)
ORIGINAL_PARTS=8
LINTER='scripts/tests/test_lint_async_executor.py'
PREFIX=LINTER+'::'
CONFIG_NAMES={'ALLOWED_PATTERNS', 'ASYNC_EXECUTOR_EXTRACTOR', 'STRICT_DIRECTORIES', 'NOQA_PATTERN', 'EXECUTOR_PATTERN'}
CONFIG_CASES={'test_strict_directories_no_trailing_slash', 'test_strict_directories_are_unique', 'test_allowed_patterns_includes_expected_exceptions', 'test_noqa_pattern_is_compiled_pattern_object', 'test_strict_directories_seven_entries', 'test_executor_pattern_is_compiled_pattern_object', 'test_allowed_patterns_is_list_of_strs', 'test_strict_directories_is_list_of_strs', 'test_allowed_patterns_every_entry_is_nonempty_string', 'test_allowed_patterns_are_unique', 'test_strict_directories_covers_every_backend', 'test_allowed_patterns_are_valid_regex', 'test_executor_pattern_compiled_regex', 'test_strict_directories_all_under_impl', 'test_allowed_patterns_nonempty', 'test_noqa_pattern_compiled_regex'}
STORAGE_CASES=set()
PROTOCOL_CASES={'test_should_check_file_is_callable', 'test_main_unknown_flag_raises_systemexit', 'test_module_has_docstring', 'test_module_docstring_mentions_run_in_executor', 'test_main_help_flag_exits_zero', 'test_is_allowed_is_callable', 'test_main_is_callable', 'test_module_exports_stable', 'test_check_file_is_callable'}
BOUND_CASES={PREFIX+n for n in CONFIG_CASES|STORAGE_CASES|PROTOCOL_CASES}
FORMAT='rerail.executor-progress/v1'
IO_CASES={
    PREFIX+'test_check_file_tolerates_unreadable_file':([], 'No readable source files (file discovery or I/O contract)'),
    PREFIX+'test_check_file_reports_warning_but_returns_empty_on_unreadable':([dict(path='mod.py',source='x = 1\n')], 'Test replaces the Python source reader; retain its I/O assertions upstream'),
    PREFIX+'test_check_file_tolerates_generic_exception_type':([dict(path='mod.py',source='x = 1\n')], 'Test replaces the Python source reader; retain its I/O assertions upstream'),
}


def validate(value):
    require(value['format']==FORMAT and value['admitted'] is False and value['fullMigrationAcceptance'] is False and value['remaining'],'Invalid progress scope')
    for section in ['programs','harness']:
        for item in value[section].values():require((sha(item['text'].encode()) if item['text'] is not None else None)==item['sha256'],'Changed '+section+' snapshot')
    for name,item in value['sources'].items():require(sha(item['text'].encode())==item['sha256'] and equal(json.loads(item['text']),item['value']),'Changed '+name+' source snapshot')
    original=value['sources']['original'];source=original['value'];engine=value['engineSource']['cliArtifact']['sha256']
    require(source['commit']==PIN and source['pytest_exit_code']==0 and not source['source_mismatches'],'Invalid original source suite')
    inventory=value['inventory'];files={p for p,r in inventory['files'].items() if CHECK in r['detectors']};require(files=={LINTER} and inventory['sourcePin']==PIN,'Changed original ownership')
    wanted={p for p in inventory['cases'] if p.split('::')[0] in files}
    require(len(wanted)==117 and set(source['test_outcomes'])==wanted,'Missing original source assertions')
    expected=defaultdict(list)
    for row in source['events']:
        if row['check']==CHECK:expected[row['test'],row['entry']].append(row)
    positives=sum(bool(e.get('findings')) for rows in expected.values() for e in rows)
    require(sum(map(len,expected.values()))==42 and positives==28,'Missing original observations or positives')
    for mode in MODES:
        reports=value['originals'][mode];seen=Counter();observed=defaultdict(list);without=set()
        for report in reports:
            require(report['exitCode']==0 and report['mode']==mode and report['engineSha256']==engine and report['oracle'] is False,'Failed original engine or API')
            require(not report['semanticFallbacks'] and not report['hostProtocolCalls'] and not report['missingResults'] and not report.get('retainedCaseIds'),'Original assertions delegated or omitted')
            require(report['originalRecordingSha256']==original['sha256'] and report['sourceFiles'][LINTER]==inventory['files'][LINTER]['sha256'],'Changed original recording or tests')
            require(report['sourceFiles']['scripts/lint_async_executor.py']==value['sources']['public']['value']['sourceSha256'],'Mixed original linter modules')
            require(set(report['collected'])==wanted and report['policySha256'][CHECK]==value['ruleSha256'],'Changed original inventory or rule')
            binding=report['configurationBindings'].get(CHECK,{})
            require(set(binding.get('names',[]))==CONFIG_NAMES,'Missing native configuration binding')
            call=report['calls'][binding['nativeCall']]
            require(all(c['status']=='passed' for c in report['calls']),'Failed original native execution')
            require(call['check']==CHECK and call['entry']=='_exec_configuration' and call['status']=='passed' and call['diagnostics'] and all(r['code']=='EXEC_CONFIG' for r in call['diagnostics']),'Invalid native configuration execution')
            for identity,phases in report['cases'].items():
                seen[identity]+=1
                require(len(phases)==3 and {p['phase']:p['outcome'] for p in phases}==dict(setup='passed',call='passed',teardown='passed'),'Failed original test phase')
            executed={c['test'] for c in report['calls'] if c['check']==CHECK and c['status']=='passed'}
            missing=set(report['cases'])-executed
            require(set(report['casesWithoutNativeCalls'])==missing,'Hidden original passive coverage')
            without.update(missing)
            for row in report['detectorObservations']:observed[row['test'],row['entry']].append(row)
        require(seen==Counter(wanted) and without==BOUND_CASES,'Missing or repeated original case, or unexpected passive assertion')
        require(set(observed)==set(expected),'Missing original observation identity')
        for key,rows in observed.items():
            require(len(rows)==len(expected[key]),'Missing original observation')
            for row,oracle in zip(rows,expected[key],strict=True):
                if row['status']!='passed':
                    require(row['test'] in IO_CASES,'Unexpected retained original observation')
                    files,reason=IO_CASES[row['test']]
                    require(row['status']=='source-result-passed; legacy normalized DTO unavailable' and oracle['sourceResult']==[] and oracle['files']==files and not oracle.get('findings') and row['originalRetainedReason']==oracle['retained_reason']==reason,'Unvalidated original I/O result')
                    require(any(c['test']==row['test'] and c['entry']=='_exec_warnings' and c['status']=='passed' and any(d['code']=='EXEC_WARNING' for d in c['diagnostics']) for report in reports for c in report['calls']),'Missing native original I/O warning')
                require(equal(row['actualSourceResult'],oracle['sourceResult']) and equal(row['expectedSourceResult'],oracle['sourceResult']),'Changed original complete result')
    for group,count in COUNTS.items():
        item=value['sources'][group];source=item['value'];oracle={r['input']['id']:r for r in source['cases']}
        require(source['passed'] and source['mode']=='source' and source['sourcePin']==PIN and len(oracle)==count and source['totalCases']==count,'Missing '+group+' source cases')
        require(source['sourceSha256']==value['sources']['public']['value']['sourceSha256'],'Mixed source modules')
        for mode in MODES:
            reports=value['reports'][group][mode];rows=[r for report in reports for r in report['cases']]
            require(Counter(r['input']['id'] for r in rows)==Counter(oracle.keys()),'Missing or repeated '+group+' cases')
            for report in reports:
                require(report['passed'] and report['executionMode']==mode and report['engineSha256']==engine and report['oracle'] is False and not report['semanticFallbacks'],'Failed '+group+' engine or API')
                require(report['baselineSha256']==item['sha256'] and report['sourceSha256']==source['sourceSha256'],'Changed '+group+' oracle')
            for row in rows:
                expected=oracle[row['input']['id']]
                require(row['passed'] and equal(row['input'],expected['input']) and equal(row['expected'],expected['expected']) and equal(row['actual'],expected['expected']),'Changed '+group+' complete result')
                require(row['native'] or group=='cli' and row['input']['id'] in {'help','unknown-option'},'Missing '+group+' native execution')
    legacy=value['sources']['legacy']['value']['cases']
    require(Counter(r['input']['corpus'] for r in legacy)=={'batch-02-recording.json':42,'batch-02-probes.json':8},'Missing earlier input family')
    for row in legacy:require(row['input']['corpusSha256']==value['legacyCorpora'][row['input']['corpus']]['sha256'],'Changed earlier input corpus')
    executions=0
    for group in ['originals','reports']:
        for row in walk(value[group]):
            for name,digest in row.get('harnessFiles',{}).items():require(value['harness'][name]['sha256']==digest,'Mixed native harness')
            if 'programInputs' not in row:continue
            executions+=1
            require(any(p['path']==value['rulePath'] and p['sha256']==value['ruleSha256'] for p in row['programInputs']),'Mixed main rule cohort')
            for p in row['programInputs']:require(value['programs'][p['path']]['sha256']==p['sha256'],'Unbound consumed program')
    return dict(admitted=False,fullMigrationAcceptance=False,originalCasesPerAPI=117,originalPositiveObservationsPerAPI=28,originalConfigurationOrPassiveCasesPerAPI=25,
        configurationCasesPerAPI=len(CONFIG_CASES),storageCasesPerAPI=len(STORAGE_CASES),protocolCasesPerAPI=len(PROTOCOL_CASES),casesPerAPI=COUNTS,nativeExecutions=executions)


def pack(work):
    manifest=read_json(ROOT/MANIFEST);rule=next(r for r in manifest['rules'] if r['check']==CHECK)
    value=dict(format=FORMAT,admitted=False,fullMigrationAcceptance=False,engineSource=manifest['engineSource'],rulePath=rule['rulePath'],ruleSha256=rule['ruleSha256'],
        inventory=read_json(ROOT/'gate/data-layer/fixtures/host/data-layer-port/rr-original-test-inventory-v2.json'),sources={},programs={},harness={},
        originals={m:[read_json(work/f'accepted-original-{m}-{i}.json') for i in range(ORIGINAL_PARTS)] for m in MODES},
        reports={g:{m:[read_json(work/f'accepted-{g}-{m}-{i}.json') for i in range(PARTS[g])] for m in MODES} for g in COUNTS},
        legacyCorpora={n:dict(sha256=sha((ROOT/'gate/data-layer/fixtures/host/data-layer-port'/n).read_bytes())) for n in ['batch-02-recording.json','batch-02-probes.json']},
        remaining=['Complete persistent repository update sequences and behavior-removal controls.','Complete strict admission tests and admit the rule.'])
    for name,file in [('original','source.json'),*[(g,g+'-source-accepted.json') for g in COUNTS]]:
        text=(work/file).read_text();value['sources'][name]=dict(text=text,sha256=sha(text.encode()),value=json.loads(text))
    for group in ['originals','reports']:
        for row in walk(value[group]):
            for p in row.get('programInputs',[]):
                text=(ROOT/p['path']).read_text() if p['sha256'] else None;require((sha(text.encode()) if text is not None else None)==p['sha256'],'Program changed after execution');value['programs'][p['path']]=dict(text=text,sha256=p['sha256'])
            for p,digest in row.get('harnessFiles',{}).items():
                text=(ROOT/p).read_text();require(sha(text.encode())==digest,'Harness changed after execution');value['harness'][p]=dict(text=text,sha256=digest)
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['pack','validate']);parser.add_argument('path',type=Path);parser.add_argument('--work',type=Path);args=parser.parse_args()
    if args.command=='pack':
        value=pack(args.work);args.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))
