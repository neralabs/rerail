"""Replay all earlier batch-two type-safety inputs against the pinned source."""
import json
from contract_runner import main
from dto import canonical
from migration import ROOT,sha
from replay import write_fixture
from types_cli_contract import CHECK
from types_port import install as native_install
from types_production_contract import exercise as production_exercise,install as production_install

FILES=['batch-02-recording.json','batch-02-probes.json']


def cases(_module):
    result=[]
    for filename in FILES:
        raw=(ROOT/'gate/data-layer/fixtures/host/data-layer-port'/filename).read_bytes();corpus=json.loads(raw)
        for index,record in enumerate(corpus['events']):
            if record.get('check')!=CHECK:continue
            files=[]
            for item in record['files']:
                source=item.get('source',corpus.get('sources',{}).get(item.get('sourceSha256')))
                if source is None:raise ValueError('Missing earlier source body')
                if item.get('sourceSha256'):assert sha(source.encode())==item['sourceSha256']
                files.append(dict(path=item['path'],source=source))
            result.append(dict(kind='legacy',id=f'{filename}:{index}',corpus=filename,corpusSha256=sha(raw),identity=record['test'],entry=record['entry'],production=bool(record.get('production_scope')),files=files))
    return result


def exercise(module,case,scratch):
    if case['production']:return production_exercise(module,case,scratch)
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files']);values=[]
    entries=['check_file_for_patterns','check_function_signatures'] if case['entry']=='probe.combined' else [case['entry']]
    if any(e not in {'check_file_for_patterns','check_function_signatures'} for e in entries):raise ValueError('Untranslated earlier type-safety entry')
    for file in case['files'] or [dict(path='missing.py')]:
        for entry in entries:values.extend(getattr(module,entry)(root/file['path'],root))
    return canonical(values,scratch)


def install(backend,module,detectors=False):
    native_install(backend,module,detectors=detectors);production_install(backend,module,detectors=detectors)


if __name__=='__main__':
    main(CHECK,'rerail.type-safety-legacy/v1',cases,exercise,install,['types_legacy_contract.py','types_port.py','types_production_contract.py'])
