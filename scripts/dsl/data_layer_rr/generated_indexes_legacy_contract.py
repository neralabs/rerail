"""Replay every existing generated-index input from the earlier batch-ten corpora."""
import gzip
import json
from pathlib import Path
from contract_runner import main
from generated_indexes_contract import CHECK
from generated_indexes_port import install
from dto import canonical
from migration import ROOT,sha
from replay import write_fixture

BASE=ROOT/'gate/data-layer/fixtures/host/data-layer-port'
FILES=['batch-10-shape-seeds.json','batch-10-file-contract-regressions.json','batch-10-complete-api.json.gz','batch-10-lifecycle.json']


def cases(_module):
    result=[]
    for filename in FILES:
        path=BASE/filename;raw=path.read_bytes();corpus=json.loads(gzip.decompress(raw) if filename.endswith('.gz') else raw)
        records=corpus.get('records',corpus.get('cases',[]))
        for index,record in enumerate(records):
            if record.get('check')!=CHECK:continue
            entries=record.get('states',[record])
            for ordinal,item in enumerate(entries):
                files=[]
                for file in item.get('files',[]):
                    text=file.get('source',corpus.get('sources',{}).get(file.get('sourceSha256')))
                    if text is None:raise ValueError('Legacy source body is missing')
                    if file.get('sourceSha256'):assert sha(text.encode())==file['sourceSha256']
                    files.append(dict(path=file['path'],source=text))
                result.append(dict(id=f'{filename}:{index}:{ordinal}',kind='legacy',corpus=filename,corpusSha256=sha(raw),
                    identity=item.get('test',item.get('name',record.get('test'))),entry=item.get('entry','check_repo'),files=files,parameters=item.get('parameters',{})))
    return result


def exercise(module,case,scratch):
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    if case['entry']=='check_repo':value=module.check_repo(root)
    elif case['entry']=='check_source':value=module.check_source(root/case['files'][0]['path'],'ignored supplied text')
    elif case['entry']=='check_file':value=module.check_file(root/case['files'][0]['path'])
    else:raise ValueError('Untranslated legacy generated-index entry: '+case['entry'])
    return [v.replace(scratch.as_posix()+'/', '') for v in value]


if __name__=='__main__':
    main(CHECK,'rerail.generated-indexes-legacy-corpus/v1',cases,exercise,install,
         ['generated_indexes_legacy_contract.py','generated_indexes_port.py'])
