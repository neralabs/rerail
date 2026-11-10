"""Validate native production I/O parity and behavior-removal controls."""
import json
from collections import Counter
from migration import PIN,sha
from validate_supplied_facts_checkpoint import require,equal


def validate(value):
    require(value['format']=='rerail.dispatch-inventory-progress/v1' and value['admitted'] is False,'Wrong progress receipt')
    source=json.loads(value['source']['text']);require(sha(value['source']['text'].encode())==value['source']['sha256'],'Source bytes differ')
    require(source['sourcePin']==PIN and source['passed'] and source['complete'],'Incomplete source oracle')
    expected={r['input']['id']:r for r in source['cases']};require(len(expected)==23,'Wrong I/O coverage')
    require('scan-fifo' in expected,'Nonblocking scan witness omitted')
    engine=value['build']['sha256'];seen={m:[] for m in ['snapshot','workspace']};controls={m:[] for m in seen};executions=0
    for section in ['programs','harness']:
        for row in value[section].values():require(sha(row['text'].encode())==row['sha256'],'Snapshot bytes differ')
    def execution(native):
        nonlocal executions
        executions+=1;require(bool(native.get('programInputs')),'Native execution provenance omitted')
        for row in native['programInputs']:
            require(row['path']+'@'+row['sha256'] in value['programs'],'Consumed program differs')
    for report in value['ioReports']:
        require(report['engineSha256']==engine and report['passed'] and report['complete'] and not report['semanticFallbacks'],'I/O failure or fallback')
        require(report['baselineSha256']==value['source']['sha256'],'Wrong source recording')
        for name,digest in report['harnessFiles'].items():require(value['harness'][name]['sha256']==digest,'Harness differs')
        for row in report['cases']:
            identity=row['input']['id'];seen[report['mode']].append(identity);oracle=expected[identity]
            require(row['passed'] and equal(row['input'],oracle['input']) and equal(row['expected'],oracle['expected']) and equal(row['actual'],oracle['expected']),'I/O results differ')
            execution(row['native'])
            if 'error' in oracle['expected']:
                require(sum(r['code']=='SDR_IO_ERROR' for r in row['native']['diagnostics'])==1,'Error did not come from the native producer')
    for report in value['controlReports']:
        require(report['engineSha256']==engine and report['passed'] and report['complete'] and not report['semanticFallbacks'],'Control failure or fallback')
        for name,digest in report['harnessFiles'].items():require(value['harness'][name]['sha256']==digest,'Control harness differs')
        for row in report['cases']:
            controls[report['mode']].append(row['id']);require(row['passed'] and not equal(row['expected'],row['actual']),'Removal was not detected')
            require(sha(row['mutatedPolicy'].encode())==row['mutatedPolicySha256'],'Mutation differs');execution(row['execution'])
    policy_ids=[]
    for report in value['policyErrorChecks']:
        require(report['engineSha256']==engine and report['passed'] and report['complete'] and report['surface']=='policy','Policy error probe failed')
        for row in report['cases']:
            identity=row['input']['id'];policy_ids.append(identity)
            require(equal(row['actual'],expected[identity]['expected']),'Policy error result differs');execution(row['native'])
    require(Counter(policy_ids)==Counter(['directory','invalid-start']),'Policy error probes omitted')
    for mode in seen:
        require(Counter(seen[mode])==Counter(expected.keys()),'I/O cases omitted or repeated')
        require(Counter(controls[mode])==Counter(['SPD001','SPD002','SPD003','SPD004','SPD005','inventory-directory','inventory-invalid-start']),'Controls omitted or repeated')
    require(value['pending'],'Missing admission limits')
    return dict(ioCasesPerAPI=23,disabledBehaviorControlsPerAPI=7,nativeExecutions=executions,admitted=False)
