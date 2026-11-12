"""Replay every boundary input retained in the original batch-two corpora."""
import json
from pathlib import Path
from contract_runner import main
from dto import canonical
from migration import ROOT,sha
from replay import write_fixture
from boundary_production_contract import CHECK
from boundary_production_contract import exercise as production_exercise


def cases(_module):
    rows=[]
    for filename in ['batch-02-recording.json','batch-02-probes.json']:
        raw=(ROOT/'gate/data-layer/fixtures/host/data-layer-port'/filename).read_bytes();corpus=json.loads(raw)
        for i,record in enumerate(corpus['events']):
            if record.get('check')!=CHECK:continue
            files=[]
            for f in record['files']:
                text=f.get('source',corpus.get('sources',{}).get(f.get('sourceSha256')))
                if text is None:raise ValueError('Missing historical boundary input')
                if f.get('sourceSha256'):assert sha(text.encode())==f['sourceSha256']
                files.append(dict(path=f['path'],source=text))
            rows.append(dict(kind='legacy',id=f'{filename}:{i}',identity=record['test'],corpus=filename,corpusSha256=sha(raw),entry=record['entry'],production=bool(record.get('production_scope')),files=files))
    return rows


def exercise(module,case,scratch):
    if case['production']:return production_exercise(module,case,scratch)
    if case['entry'] not in {'scan_tree','probe.combined'}:raise ValueError('Untranslated historical boundary entry')
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    return dict(value=canonical(module.scan_tree(root,[f['path'] for f in case['files']]),scratch))



def install(backend,module,detectors=False):
    from boundary_components_port import install as native_install
    from boundary_production_contract import install as production_install
    native_install(backend,module,detectors=detectors);production_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.boundary-legacy/v1',cases,exercise,install,['boundary_legacy_contract.py','boundary_production_contract.py'])
