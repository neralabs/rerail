"""Replay every executor input retained in the original batch-two corpora."""
import contextlib
import io
import json
from contract_runner import main
from dto import canonical
from migration import ROOT,sha
from replay import write_fixture
from executor_cli_contract import CHECK
from executor_port import install as native_install
from executor_production_contract import exercise as production_exercise,install as production_install

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
                if source is None:raise ValueError('Missing earlier executor source')
                if item.get('sourceSha256'):assert sha(source.encode())==item['sourceSha256']
                files.append(dict(path=item['path'],source=source))
            result.append(dict(kind='legacy',id=f'{filename}:{index}',corpus=filename,corpusSha256=sha(raw),identity=record['test'],entry=record['entry'],production=bool(record.get('production_scope')),files=files))
    return result


def exercise(module,case,scratch):
    if case['production']:return production_exercise(module,case,scratch)
    if case['entry'] not in {'check_file','probe.combined'}:raise ValueError('Untranslated earlier executor entry')
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();write_fixture(root,case['files']);values=[];out=io.StringIO();err=io.StringIO()
    with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
        for file in case['files'] or [dict(path='missing.py')]:values.extend(module.check_file(root/file['path']))
    return dict(value=canonical(values,scratch),stdout=out.getvalue().replace(str(scratch),'$SCRATCH'),stderr=err.getvalue().replace(str(scratch),'$SCRATCH'))


def install(backend,module,detectors=False):
    native_install(backend,module,detectors=detectors);production_install(backend,module,detectors=detectors)


if __name__=='__main__':
    main(CHECK,'rerail.executor-legacy/v1',cases,exercise,install,['executor_legacy_contract.py','executor_port.py','executor_production_contract.py'])
