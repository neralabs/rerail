from pathlib import Path
import sys,tempfile,os
sys.path.insert(0,'scripts/dsl/data_layer_rr')
from replay import execute_repository,write_fixture
from migration import write_json,read_json,MANIFEST,sha
engine=Path(read_json(MANIFEST)['engineSource']['cliArtifact']['path']).absolute()
result=dict(engineSha256=sha(engine.read_bytes()),expected=[['error','ff'],['line','Index()'],['listing','a.py'],['listing','bad.py'],['metadata','8']],cases=[],passed=False)
with tempfile.TemporaryDirectory() as temporary:
 root=Path(temporary)/'repo';root.mkdir();write_fixture(root,[dict(path='pkg/a.py',source='Index()\n')]);(root/'pkg/bad.py').write_bytes(b'\xff')
 os.environ['RERAIL_TAPE_CACHE']=str(Path(temporary)/'cache.sqlite')
 for mode in ['snapshot','workspace']:
  r=execute_repository(engine,Path('test/fixtures/rr/ground-world-requests.rr').absolute(),root,{},mode)
  actual=sorted([[v['parameters']['key'],v['parameters']['value']] for v in r['diagnostics']])
  result['cases'].append(dict(mode=mode,actual=actual,native=r,passed=actual==result['expected']))
  write_json(Path(sys.argv[1]),result)
result['passed']=all(c['passed'] for c in result['cases']);write_json(Path(sys.argv[1]),result)
assert result['passed'],result
