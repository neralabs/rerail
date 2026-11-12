"""Replay every retained admin input from the original batch-two corpora."""
import json
from dto import canonical
from migration import ROOT,sha
from replay import write_fixture
from admin_public_contract import CHECK
from admin_production_contract import exercise as production_exercise


def cases(_module):
    rows=[]
    for filename in ['batch-02-recording.json','batch-02-probes.json']:
        data=(ROOT/'gate/data-layer/fixtures/host/data-layer-port'/filename).read_bytes();corpus=json.loads(data)
        for i,record in enumerate(corpus['events']):
            if record.get('check')!=CHECK:continue
            files=[]
            for file in record['files']:
                text=file.get('source',corpus.get('sources',{}).get(file.get('sourceSha256')))
                if text is None:raise ValueError('Missing historical admin source input')
                if file.get('sourceSha256'):assert sha(text.encode())==file['sourceSha256']
                files.append(dict(path=file['path'],source=text))
            rows.append(dict(kind='legacy',id=f'{filename}:{i}',identity=record['test'],corpus=filename,corpusSha256=sha(data),entry=record['entry'],production=bool(record.get('production_scope')),files=files))
    return rows


def exercise(module,case,scratch):
    if case['production']:return production_exercise(module,case,scratch)
    if case['entry'] not in {'_collect_violations','probe.combined'}:raise ValueError('Untranslated historical admin entry')
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    return dict(value=canonical(module._collect_violations(root),scratch))


def install(backend,module,detectors=False):
    from admin_components_port import install as install_components
    from admin_production_contract import install as install_production
    install_components(backend,module,detectors=detectors);install_production(backend,module,detectors=detectors)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.admin-legacy/v1',cases,exercise,install,['admin_legacy_contract.py','admin_production_contract.py'])
