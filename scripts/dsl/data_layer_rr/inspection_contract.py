"""Source observations for inspection regexes, discovery, read selection and CLI."""
import contextlib
import io
from pathlib import Path
import sys
from unittest.mock import patch
from dto import canonical

CHECK='no_source_inspection'
WITNESS='inspect.getsource(f)\n'


def cases(_module):
    rows=[]
    bases=['inspect.getsource(f)','inspect.getsourcelines(f)','inspect.getsourcefile(f)',
           'from inspect import getsource','from inspect import getsourcelines']
    for i,base in enumerate(bases):
        variants=[base,'é'+base,'\u0301'+base,'²'+base,'_x'+base,'# '+base,'\u2003 # '+base,
            'pass # '+base,repr(base),base+'; '+base,base.replace('inspect','INSPECT'),base.replace('inspect','inspect\u00a0')]
        for n,text in enumerate(variants):rows.append(dict(kind='content',id=f'pattern-{i}-{n}',source=text))
    for label,text in [('all-patterns','; '.join(bases)),('line-order','\n'.join(reversed(bases))),
        ('clean','pass'),('mixed-import','from inspect import signature, getsource, getsourcelines'),
        ('not-word','from inspect import xgetsource'),('word-tail','from inspect import getsourcefile'),
        ('import-tab','from\tinspect\timport\tgetsource'),('import-space','from\u00a0inspect\u00a0import\u00a0getsourcelines'),
        ('long-line','😀'*100+'; '+WITNESS),('invalid-python','if (: '+WITNESS)]:
        rows.append(dict(kind='content',id=label,source=text))
    for sep in ['\n','\r\n','\r','\v','\f','\x1c','\x85','\u2028','\u2029']:
        rows.append(dict(kind='content',id='separator-'+repr(sep),source='pass'+sep+WITNESS))
    for path in ['__pycache__/a.py','.pytest_cache/a.py','__pycache__','a/__pycache__','x__pycache__/a.py',
        'a.py','a.txt','.py','a/../__pycache__/a.py','__PYCACHE__/a.py','tests/a.py','conform/a.py','tmp/a.py']:
        rows.append(dict(kind='content',id='path-'+path,path=path,source=WITNESS))
    for group in ['defaults','empty','tests','duplicate','overlap','absolute-outside','relative-root','file-as-root',
                  'missing','invalid-utf8','directory-py','symlink-file','symlink-dir','read-spy']:
        rows.append(dict(kind='find',id='find-'+group,group=group))
    for group in ['clean','one','duplicates','ordering','outside-root-error','late-outside-root-error']:
        for args in [[],['--check']]:rows.append(dict(kind='cli',id='cli-'+group+repr(args),group=group,args=args))
    for arg in ['--help','--bad-flag']:rows.append(dict(kind='cli',id='cli-'+arg,group='clean',args=[arg]))
    rows.extend([dict(kind='cli-run',id='cli-run',args=['--check']),dict(kind='configuration',id='configuration')])
    return rows


def exercise(module,case,temporary):
    if case['kind']=='cli':
        from text_cli_contract import exercise as cli_exercise
        return cli_exercise(module,dict(case,check=CHECK),temporary)
    root=temporary/'project';root.mkdir();reads=[]
    def normalize(value):
        if isinstance(value,Path):return normalize(str(value))
        if isinstance(value,str):return value.replace(str(temporary),'$TMP')
        if isinstance(value,(tuple,list)):return [normalize(v) for v in value]
        if isinstance(value,dict):return {k:normalize(v) for k,v in value.items()}
        return value
    out,err=io.StringIO(),io.StringIO()
    with contextlib.ExitStack() as stack:
        kind=case['kind']
        if kind=='content':action=lambda:module.scan_file_content(root/case.get('path','a.py'),case['source'])
        elif kind in {'find','cli-run'}:
            for name in ['tests/z.py','tests/sub/a.py','tests/__pycache__/bad.py','tests/.pytest_cache/bad.py',
                         'tests/.py','tests/a.txt','conform/a.py','other/a.py']:
                p=root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(WITNESS)
            group=case.get('group','defaults')
            outside=temporary/'outside';outside.mkdir();(outside/'a.py').write_text(WITNESS)
            if group=='invalid-utf8':(root/'tests/bad.py').write_bytes(b'\xff')
            if group=='directory-py':(root/'tests/directory.py').mkdir()
            if group=='symlink-file':(root/'tests/alias.py').symlink_to('z.py')
            if group=='symlink-dir':(root/'tests/alias').symlink_to('sub',target_is_directory=True)
            if group=='read-spy':
                original=Path.read_text
                def read(path,*args,**kwargs):
                    if path.is_relative_to(root):reads.append(str(path))
                    return original(path,*args,**kwargs)
                stack.enter_context(patch.object(Path,'read_text',read))
            directories={'defaults':None,'empty':[],'tests':['tests'],'duplicate':['tests','tests'],
                'overlap':['tests','tests/sub'],'absolute-outside':[str(outside)],'relative-root':['tests'],
                'file-as-root':['tests/z.py'],'missing':['missing'],'invalid-utf8':None,'directory-py':None,
                'symlink-file':None,'symlink-dir':None,'read-spy':None}[group]
            if kind=='cli-run':
                stack.enter_context(patch.object(sys,'argv',['lint_no_source_inspection.py','--root',str(root),*case['args']]))
                action=module.cli_main
            else:
                if group=='relative-root':stack.enter_context(contextlib.chdir(temporary));root=Path('project')
                action=lambda:module.find_violations(root,directories)
        else:action=lambda:dict(directories=module.TEST_DIRECTORIES,excluded=module.EXCLUDED_PATHS,
            patterns=[dict(pattern=p.pattern,flags=p.flags,label=label) for p,label in module.FORBIDDEN_PATTERNS],
            extractor=canonical(module.NO_SOURCE_INSPECTION_EXTRACTOR))
        with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
            try:result=dict(value=action())
            except BaseException as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args)))
    return normalize(dict(result,stdout=out.getvalue(),stderr=err.getvalue(),reads=reads))


if __name__=='__main__':
    from contract_runner import main
    from inspection_port import install
    main(CHECK,'rerail.inspection-contract/v1',cases,exercise,install,
         ['inspection_contract.py','inspection_port.py','text_cli_contract.py','text_cli_port.py','contract_runner.py'])
