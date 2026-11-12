"""Retain complete boundary component parity without granting full rule admission."""
import argparse
import gzip
import json
from pathlib import Path
import shutil
import contract_checkpoint
from migration import sha
from validate_supplied_facts_checkpoint import require

FORMAT='rerail.boundary-component-progress/v1'
MODES=('snapshot','workspace')
CONFIG_NAMES={'SOURCE_TRIGGERS','ALLOWLIST','INFORMATIONAL_DIRS','CATEGORY_LABELS','ASYNC_SYNC_BOUNDARY_EXTRACTOR'}


def validate(value):
    require(value['format']==FORMAT,'Wrong boundary component checkpoint')
    summary=contract_checkpoint.validate(dict(value,format='rerail.recorded-contract-progress/v1'))
    require(summary['check']=='async_sync_boundary' and summary['recordedCasesPerAPI']==173 and summary['originalCasesPerAPI']==5 and summary['originalPositiveObservationsPerAPI']==3,'Incomplete boundary component scope')
    for mode in MODES:
        original=value['originals'][mode];binding=original['configurationBindings'].get('async_sync_boundary',{})
        require(set(binding.get('names',[]))==CONFIG_NAMES,'Missing native boundary configuration binding')
        call=original['calls'][binding['nativeCall']]
        require(call['entry']=='_bs_configuration' and call['status']=='passed' and call['diagnostics'] and all(d['code']=='BS_CONFIG' for d in call['diagnostics']),'Missing native configuration execution')
        for report in value['reports'][mode]:
            for row in report['cases']:
                if row['input']['kind']!='extract':continue
                expected=dict(row['expected']['value']['items']);facts=dict(expected['facts']['items'])
                if facts.get('async_sync_boundary.violations'):
                    require(any(d['code']=='BS_VALUE' for call in row['native'] for d in call['diagnostics']),'Positive collection did not execute natively')
                elif row['input']['id']=='extract-invalid':
                    require(not facts and expected['errors']['items'] and expected['errors']['items'][0][1].startswith('SyntaxError:'),'Invalid framework syntax-error boundary')
    return summary


def pack(work):
    stage=work/'component-checkpoint-stage';stage.mkdir(exist_ok=True)
    for src,dst in [('source.json','source.json'),('components-source.json','scanner-source.json')]:shutil.copyfile(work/src,stage/dst)
    for mode in MODES:
        shutil.copyfile(work/f'original-accepted-{mode}.json',stage/f'original-accepted-{mode}.json')
        for i in range(16):shutil.copyfile(work/f'components-accepted-{mode}-{i}.json',stage/f'scanner-accepted-{mode}-{i}.json')
    value=contract_checkpoint.pack(stage,'async_sync_boundary')
    value.update(format=FORMAT,remaining=['Complete production and file selection contracts.','Complete native rendering, CLI and ratchet administration.','Complete earlier inputs, retained edits, valid removal controls and full admission.'])
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(path.parents[3]).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('command',choices=['pack','validate']);p.add_argument('path',type=Path);p.add_argument('--work',type=Path);a=p.parse_args()
    if a.command=='pack':
        value=pack(a.work);a.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(a.path,'rt') as f:value=json.load(f)
    print(json.dumps(validate(value),sort_keys=True))
