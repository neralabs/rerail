"""Record exact blanket-bypass APIs, supplied-source behavior, and native CLI contracts."""
import contextlib
import io
from pathlib import Path
import sys
from unittest.mock import patch
from dto import canonical

CHECK='no_blanket_grammar_bypass'


def cases(module):
    rows=[]
    def add(kind,name,**kw):rows.append(dict(kind=kind,id=name,**kw))
    for name in sorted(module.FORBIDDEN_PUBLIC_METHODS):
        for prefix in ['', 'async ']:add('methods',name+':'+prefix,source=f'class Conn:\n    {prefix}def {name}(self): pass\n')
    samples=[('clean','pass\n'),('both','_ALLOW_DIRECT_EXECUTION; allow_direct_execution\n'),
        ('duplicates','_ALLOW_DIRECT_EXECUTION; _ALLOW_DIRECT_EXECUTION; allow_direct_execution; allow_direct_execution\n'),
        ('comment','# _ALLOW_DIRECT_EXECUTION allow_direct_execution\n'),
        ('inline','pass # _ALLOW_DIRECT_EXECUTION allow_direct_execution\n'),
        ('string-comment','x = "#"; allow_direct_execution\n'),('unicode-space','\u2003# _ALLOW_DIRECT_EXECUTION\n'),
        ('nested','def outer():\n def execute_grammar(): pass\n'),
        ('decorated','@deco\nasync def execute_grammar(): pass\n'),
        ('unicode-on','# execute_grammar\ndef ｅxecute_grammar(): pass\n'),
        ('unicode-off','def ｅxecute_grammar(): pass\n'),
        ('syntax','def broken(:\n_ALLOW_DIRECT_EXECUTION\n'),('empty','')]
    for name,source in samples:
        add('references','refs-'+name,source=source)
        add('methods','methods-'+name,source=source)
        add('content','content-'+name,source=source,disk=source)
        add('facts','facts-'+name,source=source)
    for i,(disk,supplied) in enumerate([(a,b) for a in ['pass\n','def execute_grammar(): pass\n',None]
                                      for b in ['pass\n','execute_grammar','_ALLOW_DIRECT_EXECUTION']]):
        add('content','split-'+str(i),source=supplied,disk=disk)
    for path in ['api/a.py','venv/a.py','.hidden/a.py','tests/a.py','test/a.py','tmp/a.py','.venv/a.py',
        '.git/a.py','__pycache__/a.py','a.txt','.py','../outside.py',*sorted(module.ALLOWED_FILES)]:
        add('content','scope-'+path,path=path,source='_ALLOW_DIRECT_EXECUTION\n',disk='pass\n')
    for sep in ['\r','\r\n','\v','\f','\x85','\u2028','\u2029']:
        add('references','separator-'+repr(sep),source='pass'+sep+'_ALLOW_DIRECT_EXECUTION\n')
    for label in ['missing','invalid-utf8','directory']:
        add('methods','file-'+label,source=None if label!='invalid-utf8' else 'ff',file_kind=label)
    for group in ['ordering','allowed','excluded','non-python','duplicates','unreadable-allowed']:
        add('tree','tree-'+group,group=group)
    for group in ['clean','both','methods','references']:
        for args in [[],['--check']]:add('cli','cli-'+group+repr(args),group=group,args=args)
    for arg in ['--help','--bad-flag']:add('cli','cli-'+arg,group='clean',args=[arg])
    add('configuration','configuration')
    return rows


def exercise(module,case,temporary):
    root=temporary/'project';root.mkdir();out,err=io.StringIO(),io.StringIO()
    def write(path,text):
        path.parent.mkdir(parents=True,exist_ok=True)
        if text is not None:path.write_text(text,encoding='utf-8')
    with contextlib.ExitStack() as stack:
        kind=case['kind'];target=root/module.SQL_CONNECTION_FILE
        if kind in {'methods','facts'}:
            path=target
            if case.get('file_kind')=='directory':path.mkdir(parents=True)
            elif case.get('file_kind')=='invalid-utf8':path.parent.mkdir(parents=True);path.write_bytes(b'\xff')
            else:write(path,case['source'])
            action=lambda:module.check_no_public_grammar_methods(root) if kind=='methods' else module._facts_for_files(root,(path,))
        elif kind=='content':
            path=root/case.get('path',module.SQL_CONNECTION_FILE);write(path,case['disk'])
            action=lambda:module.scan_file_content(root,path,case['source'])
        elif kind=='references':
            write(root/'impl/a.py',case['source']);action=lambda:module.check_no_bypass_references(root)
        elif kind=='tree':
            group=case['group'];names={
                'ordering':['z.py','a/z.py','a-.py'], 'allowed':sorted(module.ALLOWED_FILES),
                'excluded':[p+'/a.py' for p in ['.git','.venv','tmp','__pycache__','venv','tests','test','.hidden']],
                'non-python':['a.txt','.py','a.PY','a.py'], 'duplicates':['a.py'],
                'unreadable-allowed':[sorted(module.ALLOWED_FILES)[0],'api/a.py']}[group]
            for name in names:write(root/name,'_ALLOW_DIRECT_EXECUTION\n'*(2 if group=='duplicates' else 1))
            if group=='unreadable-allowed':
                original=Path.read_text;blocked=root/names[0]
                def read(path,*a,**kw):
                    if path==blocked:raise PermissionError('blocked allowed file')
                    return original(path,*a,**kw)
                stack.enter_context(patch.object(Path,'read_text',read))
            action=lambda:module.check_no_bypass_references(root)
        elif kind=='cli':
            group=case['group'];write(target,'def execute_grammar(): pass\n' if group in {'both','methods'} else 'pass\n')
            if group in {'both','references'}:write(root/'api/a.py','_ALLOW_DIRECT_EXECUTION\n')
            stack.enter_context(patch.object(module,'__file__',str(root/'scripts/lint_no_blanket_grammar_bypass.py')))
            stack.enter_context(patch.object(sys,'argv',['lint_no_blanket_grammar_bypass.py',*case['args']]))
            action=module.cli_main
        else:action=lambda:dict(allowed=sorted(module.ALLOWED_FILES),methods=sorted(module.FORBIDDEN_PUBLIC_METHODS),
            sql=module.SQL_CONNECTION_FILE,extractor=canonical(module.NO_BLANKET_GRAMMAR_BYPASS_EXTRACTOR))
        with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
            try:result=dict(value=canonical(action()))
            except BaseException as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args)))
    def normalize(v):
        if isinstance(v,str):return v.replace(str(temporary),'$TMP')
        if isinstance(v,list):return [normalize(x) for x in v]
        if isinstance(v,dict):return {k:normalize(x) for k,x in v.items()}
        return v
    return normalize(dict(result,stdout=out.getvalue(),stderr=err.getvalue()))


def install(backend,module,detectors=False):
    from blanket_bypass_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.blanket-contract/v1',cases,exercise,install,
        ['blanket_contract.py','blanket_bypass_port.py','contract_runner.py'])
