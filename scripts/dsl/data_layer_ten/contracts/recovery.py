# SPDX-License-Identifier: Apache-2.0
"""Original constructor results with a large, unrelated lexical inventory."""
import argparse
import copy
import json
from pathlib import Path
import sys
import tempfile


def record(source, output):
    from data_layer_rule_audit import fixtures_path
    from data_layer_batch_seven import normalize
    from data_layer_probe_record import verify_sources
    from data_layer_ten.common import PIN
    sys.path[:0]=[str(source),str(source/'scripts')]
    from scripts import check_versioned_merged_identity as original
    baseline=json.loads(fixtures_path('host/data-layer-port/batch-07-probes.json').read_text())
    stress=dict(path='impl/common/l2070_server_plan/steps/unrelated_inventory.py',source=''.join(f'def unrelated_{i}():\n    return normal()\n' for i in range(7000)))
    events=[]
    for name in ['catalog-alias-chain','catalog-alias-killed']:
        event=copy.deepcopy(next(event for event in baseline['events'] if event['test']=='probe:batch7:'+name))
        event['test']='recovery:'+name;event['files'].append(stress)
        with tempfile.TemporaryDirectory(prefix='rerail-identity-inventory-') as directory:
            root=Path(directory)
            for file in event['files']:
                path=root/file['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_text(file['source'])
            result=original.collect_violations(root,expected_callers=frozenset(event['oracle_config']['options']['expectedCallers']))
            event['findings']=normalize(event['check'],result,event['files'],lambda path:Path(path).relative_to(root).as_posix() if Path(path).is_absolute() else str(path))
        events.append(event)
    assert not events[0]['findings'] and events[1]['findings']
    output.write_text(json.dumps(dict(format=baseline['format'],commit=PIN,source_mismatches=[],pytest_exit_code=0,checks=['versioned_merged_identity'],collected_tests=0,uncaptured_tests=[],events=events,loaded_source_files_checked=verify_sources(source,source,PIN)),indent=2)+'\n')


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--source',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args();record(args.source.resolve(),args.output)
