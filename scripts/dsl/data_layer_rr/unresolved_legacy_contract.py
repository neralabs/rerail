"""Replay every unresolved-reference input from the original batch-two recordings."""
import json
from pathlib import Path
from dto import canonical
from migration import ROOT,sha
from replay import write_fixture
from unresolved_components_contract import CHECK
from unresolved_production_contract import exercise as production_exercise


def cases(_module):
    result=[]
    for filename in ['batch-02-recording.json','batch-02-probes.json']:
        raw=(ROOT/'gate/data-layer/fixtures/host/data-layer-port'/filename).read_bytes();corpus=json.loads(raw)
        for i,row in enumerate(corpus['events']):
            if row.get('check')!=CHECK:continue
            files=[]
            for file in row['files']:
                source=file.get('source',corpus.get('sources',{}).get(file.get('sourceSha256')))
                if source is None:raise ValueError('Missing retained unresolved input')
                if file.get('sourceSha256'):assert sha(source.encode())==file['sourceSha256']
                files.append(dict(path=file['path'],source=source))
            result.append(dict(kind='legacy',id=f'{filename}:{i}',identity=row['test'],corpus=filename,corpusSha256=sha(raw),
                entry=row['entry'],production=bool(row.get('production_scope')),files=files))
    return result


def exercise(module,case,scratch):
    if case['production']:return production_exercise(module,case,scratch)
    if case['entry']not in {'analyze_file','probe.combined'}:raise ValueError('Untranslated historical unresolved entry')
    root=scratch.resolve()/'repo';root.mkdir();write_fixture(root,case['files'])
    values=[]
    for file in case['files']:values.extend(module.analyze_file(root/file['path']))
    return dict(value=canonical(values,scratch.resolve()))


def install(backend,module,detectors=False):
    from unresolved_port import install as install_public
    from unresolved_production_contract import install as install_production
    install_public(backend,module,detectors=detectors);install_production(backend,module,detectors=detectors)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.unresolved-legacy/v1',cases,exercise,install,['unresolved_legacy_contract.py','unresolved_production_contract.py'])
