"""Replay every ForwardRef input retained in the earlier batch-two corpora."""
import gzip
import json
from contract_runner import main
from forwardref_contract import CHECK
from forwardref_production_contract import exercise as production_exercise,install as production_install
from forwardref_port import install as native_install
from dto import canonical
from migration import ROOT,sha
from replay import write_fixture

FILES=['batch-02-recording.json','batch-02-probes.json']


def cases(_module):
    result=[]
    for filename in FILES:
        path=ROOT/'gate/data-layer/fixtures/host/data-layer-port'/filename;raw=path.read_bytes()
        corpus=json.loads(gzip.decompress(raw) if filename.endswith('.gz') else raw)
        for index,record in enumerate(corpus['events']):
            if record.get('check')!=CHECK:continue
            files=[]
            for file in record['files']:
                source=file.get('source',corpus.get('sources',{}).get(file.get('sourceSha256')))
                if source is None:raise ValueError('Legacy source body is missing')
                if file.get('sourceSha256'):assert sha(source.encode())==file['sourceSha256']
                files.append(dict(path=file['path'],source=source))
            result.append(dict(id=f'{filename}:{index}',kind='legacy',corpus=filename,corpusSha256=sha(raw),identity=record['test'],
                entry=record['entry'],production=bool(record.get('production_scope')),files=files))
    return result


def exercise(module,case,scratch):
    if case['production']:return production_exercise(module,case,scratch)
    if case['entry'] not in {'lint_file','probe.combined'}:raise ValueError('Untranslated legacy ForwardRef entry')
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    values=[]
    for file in case['files']:values.extend(module.lint_file(root/file['path']))
    return canonical(values,scratch)


def install(backend,module,detectors=False):
    native_install(backend,module,detectors=detectors);production_install(backend,module,detectors=detectors)


if __name__=='__main__':
    main(CHECK,'rerail.forwardref-legacy-corpus/v1',cases,exercise,install,
         ['forwardref_legacy_contract.py','forwardref_production_contract.py','forwardref_port.py'],guard_modules=('scripts.lint_lib.signing',))
