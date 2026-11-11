"""Replay every connection-union input retained in the earlier batch-two corpora."""
import gzip
import json
from contract_runner import main
from conn_union_contract import CHECK
from conn_union_production_contract import exercise as production_exercise,install
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
    if case['entry'] not in {'analyze_file','probe.combined'}:raise ValueError('Untranslated legacy connection-union entry')
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    values=[]
    for file in case['files']:values.extend(module.analyze_file(root/file['path'],root=root))
    return canonical(values,scratch)


if __name__=='__main__':
    main(CHECK,'rerail.conn-union-legacy-corpus/v1',cases,exercise,install,
         ['conn_union_legacy_contract.py','conn_union_production_contract.py','conn_union_port.py'],guard_modules=('scripts.lint_lib.signing',))
