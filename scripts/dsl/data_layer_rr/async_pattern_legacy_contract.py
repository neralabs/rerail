"""Replay every async-pattern input retained in the original batch-two corpora."""
import ast
import json
from pathlib import Path
from contract_runner import main
from dto import canonical
from migration import ROOT,sha
from replay import write_fixture
from async_pattern_components_contract import CHECK
from async_pattern_production_contract import exercise as production_exercise


def cases(_module):
    rows=[]
    for filename in ['batch-02-recording.json','batch-02-probes.json']:
        raw=(ROOT/'gate/data-layer/fixtures/host/data-layer-port'/filename).read_bytes();corpus=json.loads(raw)
        for i,record in enumerate(corpus['events']):
            if record.get('check')!=CHECK:continue
            files=[]
            for f in record['files']:
                text=f.get('source',corpus.get('sources',{}).get(f.get('sourceSha256')))
                if text is None:raise ValueError('Missing historical async-pattern input')
                if f.get('sourceSha256'):assert sha(text.encode())==f['sourceSha256']
                files.append(dict(path=f['path'],source=text))
            rows.append(dict(kind='legacy',id=f'{filename}:{i}',identity=record['test'],corpus=filename,corpusSha256=sha(raw),entry=record['entry'],production=bool(record.get('production_scope')),files=files))
    return rows


def exercise(module,case,scratch):
    if case['production']:return production_exercise(module,case,scratch)
    if case['entry'] not in {'analyze_source','probe.combined'}:raise ValueError('Untranslated historical async-pattern entry')
    value=[]
    for f in case['files']:value.extend(module.analyze_source(Path(f['path']),f['source'],tree=ast.parse(f['source'])))
    return dict(value=canonical(value,scratch))


def install(backend,module,detectors=False):
    from async_pattern_port import install as native_install
    from async_pattern_production_contract import install as production_install
    native_install(backend,module,detectors=detectors);production_install(backend,module,detectors=detectors)


if __name__=='__main__':main(CHECK,'rerail.async-pattern-legacy/v1',cases,exercise,install,['async_pattern_legacy_contract.py','async_pattern_components_contract.py','async_pattern_production_contract.py'])
