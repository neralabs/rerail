"""Source-recorded public filesystem APIs, including ignored caller syntax and explicit scope."""
import ast
from pathlib import Path
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from replay import write_fixture

CHECK='plan_scope_side_effects'
BAD='with open_write_plan() as p:\n lazy.resolve()\n'
AUDIT='with open_write_plan() as p:\n n=q.count()\n if n: pass\n'


def cases(_module):
    def f(path,source=BAD):return dict(path=path,source=source)
    groups=dict(empty=[],positive=[f('a.py')],ordering=[f(p)for p in ['z.py','a.py','sub/z.py','sub/a.py','a-/z.py']],
        skipped=[f(p+'/a.py')for p in ['tmp','.git','.venv','venv','__pycache__','tests','test','normal']],
        suffixes=[f(p)for p in ['.py','.hidden.py','a.py','a.PY','a.txt']],
        directory_py=[f('dir.py/inside.py'),f('normal/a.py')],
        file_links=[f('a.py'),dict(path='alias.py',symlink='a.py')],
        directory_links=[f('sub/a.py'),dict(path='alias.py',symlink='sub')],
        broken=[dict(path='broken.py',symlink='missing')],
        loop=[f('sub/a.py'),dict(path='sub/up',symlink='..')],
        unicode=[f('é\nname.py'),f('名前.py')])
    rows=[dict(kind='iter',id='iter-'+name,files=files)for name,files in groups.items()]
    choices=[('none',[],[]),('positive',[f('a.py')],['a.py']),('audit',[f('a.py',AUDIT)],['a.py']),
        ('clean',[f('a.py','pass\n')],['a.py']),('duplicates',[f('a.py')],['a.py','./a.py','a.py']),
        ('explicit-skipped',[f('tmp/a.py')],['tmp/a.py']),('tests',[f('tests/a.py')],['tests/a.py']),
        ('suffixes',groups['suffixes'],[p['path']for p in groups['suffixes']]),
        ('directory',groups['directory_py'],['dir.py']),('missing',[],['missing.py']),
        ('link',groups['file_links'],['alias.py','a.py']),('broken',groups['broken'],['broken.py']),
        ('relative-dots',[f('sub/keep.py'),f('a.py')],['sub/../a.py']),
        ('order',[f('z.py'),f('a.py')],['z.py','a.py']),('unicode',groups['unicode'],[p['path']for p in groups['unicode']]),
        ('syntax',[f('a.py','with open_write_plan(:\n pass\n')],['a.py']),
        ('decode',[dict(path='a.py',binary='ff')],['a.py'])]
    rows.extend(dict(kind='explicit',id='explicit-'+name,files=files,selected=selected)for name,files,selected in choices)
    rows.append(dict(kind='explicit',id='explicit-external',files=[],outside=[f('outside.py')],selected=['$OUT/outside.py']))
    rows.append(dict(kind='repo',id='repo-external-link',files=[],outside=[f('outside.py')],outside_link=True))
    for name,files,path,text,tree in [('disk-positive',[f('a.py')],'a.py','pass\n','pass\n'),
            ('disk-clean',[f('a.py','pass\n')],'a.py',BAD,BAD),('missing',[],'absent.py',BAD,BAD),
            ('explicit-skip',[f('tmp/a.py')],'tmp/a.py','pass\n','pass\n'),
            ('decode',[dict(path='a.py',binary='ff')],'a.py','pass\n','pass\n'),
            ('syntax',[f('a.py','with open_write_plan(:\n pass\n')],'a.py',BAD,BAD)]:
        rows.append(dict(kind='content',id='content-'+name,files=files,path=path,source=text,tree=tree))
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();outside=scratch/'outside';outside.mkdir()
    for directory,files in [(root,case['files']),(outside,case.get('outside',[]))]:
        write_fixture(directory,[f for f in files if 'binary'not in f])
        for f in files:
            if 'binary'in f:
                p=directory/f['path'];p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(bytes.fromhex(f['binary']))
    if case.get('outside_link'):(root/'alias.py').symlink_to(outside/'outside.py')
    try:
        with patch.object(module,'REPO_ROOT',root):
            if case['kind']=='iter':value=module._iter_python_files(root)
            elif case['kind']=='repo':value=module.scan_repo(root)
            elif case['kind']=='explicit':
                selected=[outside/p.removeprefix('$OUT/')if p.startswith('$OUT/')else root/p for p in case['selected']]
                value=module._run_fact_engine_for_files(root,selected)
            else:value=module.scan_file_content(case['path'],case['source'],ast.parse(case['tree']))
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from side_effects_scanners import install as scanners
    scanners(backend,module,detectors)


if __name__=='__main__':main(CHECK,'rerail.side-effects-files/v1',cases,exercise,install,['side_effects_files_contract.py','side_effects_scanners.py'])
