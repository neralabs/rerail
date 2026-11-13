"""Compare the real public scanners with disk, discovery, severity, and read-error witnesses."""
import contextlib
from pathlib import Path
from contract_runner import main
from dto import canonical
from replay import write_fixture
CHECK='plan_scope_side_effects'
BAD='with open_write_plan() as p:\n lazy.resolve()\n'
AUDIT='with open_write_plan() as p:\n n=q.count()\n if n: pass\n'


def cases(_module):
    groups=dict(empty=[],positive=[dict(path='impl/a.py',source=BAD)],audit=[dict(path='impl/a.py',source=AUDIT)],
        clean=[dict(path='impl/a.py',source='pass\n')],empty_file=[dict(path='impl/a.py',source='')],duplicates=[dict(path='impl/a.py',source=BAD+BAD)],
        trigger_absent=[dict(path='impl/a.py',source='with open_wr𝚒te_plan(): lazy.resolve()\n')],
        trigger_comment=[dict(path='impl/a.py',source='# open_write_plan\nwith open_wr𝚒te_plan(): lazy.resolve()\n')],
        paths=[dict(path=p,source=BAD)for p in ['z.py','a.py','tests/a.py','test/a.py','conform/a.py','tmp/a.py','.git/a.py','.venv/a.py','venv/a.py','__pycache__/a.py','.mypy_cache/a.py','.pytest_cache/a.py','.ruff_cache/a.py','.py','.hidden.py','a.PY','other.txt']],
        file_links=[dict(path='impl/a.py',source=BAD),dict(path='impl/alias.py',symlink='a.py')],
        directory_links=[dict(path='impl/a.py',source=BAD),dict(path='alias',symlink='impl')],
        directory_loop=[dict(path='impl/a.py',source=BAD),dict(path='impl/up',symlink='..')],
        broken_file=[dict(path='impl/broken.py',symlink='absent')],
        unicode=[dict(path='impl/é\nname.py',source=BAD),dict(path='impl/名前.py',source=AUDIT)],
        invalid_syntax=[dict(path='impl/a.py',source='with open_write_plan(:\n pass\n')],
        latin1_cookie=[dict(path='impl/a.py',binary='2320636f64696e673a206c6174696e310a78e90a')])
    rows=[dict(kind='repo',id=k,files=v)for k,v in groups.items()]
    for i,value in enumerate(['ff','c080','e228a1','e282','f48fbfbf','f4908080','c2','6162630aff','efbbbf'+BAD.encode().hex()]):
        rows.append(dict(kind='repo',id='utf8-'+str(i),files=[dict(path='impl/a.py',binary=value)]))
    for severity in ['error','audit','warning','','ERROR','hard-fail',' error','é']:
        rows.append(dict(kind='blocking',id='blocking-'+severity,severities=[severity,'audit',severity,'error']))
    rows += [dict(kind='blocking',id='blocking-empty',severities=[]),dict(kind='blocking',id='blocking-alias',severities=['error'],repeat=True)]
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir()
    write_fixture(root,[f for f in case.get('files',[])if 'binary'not in f])
    for file in case.get('files',[]):
        if 'binary'in file:
            path=root/file['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(bytes.fromhex(file['binary']))
    try:
        if case['kind']=='repo':result=module.scan_repo(root)
        else:
            values=[module.Violation('C','caller.py',i+1,'message',severity)for i,severity in enumerate(case['severities'])]
            if case.get('repeat'):values=values*3
            result=module.blocking_violations(values)
            return dict(value=canonical(result,scratch),identities=[next(i for i,v in enumerate(values)if v is r)for r in result],input=canonical(values,scratch))
        return dict(value=canonical(result,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from side_effects_scanners import install as scanners
    scanners(backend,module,detectors)

if __name__=='__main__':main(CHECK,'rerail.side-effects-scanners/v1',cases,exercise,install,['side_effects_scanner_contract.py','side_effects_scanners.py'])
