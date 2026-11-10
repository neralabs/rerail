"""Source contracts for raw backend comparisons, discovery, fact DTOs and CLI batching."""
import contextlib
import io
from pathlib import Path
import sys
from unittest.mock import patch
from dto import canonical

CHECK='backend_type_strings'


def cases(module):
    rows=[]
    templates=['dialect.name == "{name}"','dialect.name in ("{name}",)',
        'backend_type == "{name}"','backend == "{name}"','backend in ("{name}",)',
        '"{name}" in connection_class_name','"{name}" in type(conn).__name__']
    for name in sorted(module.BACKEND_STRINGS):
        for i,template in enumerate(templates):rows.append(dict(kind='content',id=f'backend-{name}-{i}',content=template.format(name=name)))
    for label,text in [('class-inspection','conn.__class__.__name__.lower()'),('clean','pass'),
        ('comments','  # dialect.name == "sqlite"'),('pattern-order','"mysql" in class_name; dialect.name == "sqlite"'),
        ('duplicates','dialect.name == "sqlite"; dialect.name == "sqlite"'),
        ('long-snippet','😀'*81+'; backend_type == "sqlite"'),
        ('tab-classification','backend\t==\t"sqlite"'),('tab-membership','dialect.name\tin\t("mysql",)'),
        ('unicode-column','☃😀 backend == "sqlite"'),('word-mark','\u0301backend == "sqlite"'),
        ('word-letter','ébackend == "sqlite"'),('string-source','x="dialect.name == \'sqlite\'"'),
        ('invalid-python','if (: backend_type == "sqlite"'),('inline-comment','pass # backend_type == "sqlite"')]:
        rows.append(dict(kind='content',id=label,content=text))
    for separator in ['\n','\r\n','\r','\v','\f','\x1c','\x85','\u2028','\u2029']:
        rows.append(dict(kind='content',id='lines-'+repr(separator),content='pass'+separator+'backend_type == "sqlite"'))
    for path in ['impl/a.py','tests/a.py','conform/a.py','a_test.py','test_a.py','docs/a.md','l1400_types/__init__.py','Conform/a.py']:
        rows.append(dict(kind='content',id='path-'+path,path=path,content='backend_type == "sqlite"'))
        rows.append(dict(kind='skip',id='skip-'+path,path=path))
    for label,value in [('positive','backend_type == "sqlite"'),('empty',''),('missing',None),('invalid-utf8','ff')]:
        rows.append(dict(kind='file',id='file-'+label,content=value))
    for group in ['empty','single','overlap','mixed','dotfile','directory-py','relative']:
        rows.append(dict(kind='files',id='files-'+group,group=group))
        rows.append(dict(kind='batch',id='batch-'+group,group=group))
    for value in [dict(file='a.py',line=1,column=2,message='message',code='BT001',snippet='snippet'),
                  dict(file='a/../b.py',line=True,column=2**100,message=['☃',1],code=None,snippet='😀')]:
        rows.append(dict(kind='fact',id='fact-'+str(len(rows)),value=value))
    for group in ['clean','one','duplicates','ordering']:
        for args in [[],['--check'],['a.py','--check']]:rows.append(dict(kind='cli',id='cli-'+group+repr(args),group=group,args=args))
    for args in [['--help'],['--bad']]:rows.append(dict(kind='cli',id='cli-special-'+repr(args),group='clean',args=args))
    rows.append(dict(kind='configuration',id='configuration'))
    return rows


def exercise(module,case,temporary):
    root=temporary/'project';root.mkdir();calls=[]
    def normalize(v):
        if isinstance(v,Path):return normalize(str(v))
        if isinstance(v,str):return v.replace(str(temporary),'$TMP')
        if isinstance(v,(tuple,list)):return [normalize(x) for x in v]
        if isinstance(v,dict):return {k:normalize(x) for k,x in v.items()}
        return v
    def values(rows):return [dict(file=v.file,line=v.line,column=v.column,message=v.message,code=v.code,snippet=v.snippet) for v in rows]
    out,err=io.StringIO(),io.StringIO()
    with contextlib.ExitStack() as stack:
        kind=case['kind']
        if kind in {'content','file','skip'}:
            path=root/case.get('path','impl/a.py');path.parent.mkdir(parents=True,exist_ok=True)
            if kind=='file' and case['content'] is not None:
                if case['id']=='file-invalid-utf8':path.write_bytes(bytes.fromhex(case['content']))
                else:path.write_text(case['content'])
            if kind=='skip':action=lambda:module.should_skip_file(path)
            else:action=lambda:values(module.check_file(path) if kind=='file' else module.check_file_content(path,case['content']))
        elif kind in {'files','batch'}:
            for name in ['impl/a.py','impl/z.py','impl/a/z.py','impl/a-.py','impl/.py','impl/a.txt','impl/tests/a.py']:
                path=root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('backend_type == "sqlite"')
            (root/'impl/dir.py').mkdir()
            paths=dict(empty=[],single=[root/'impl/a.py'],overlap=[root/'impl',root/'impl/a.py',root/'impl'],
                mixed=[root/'impl',root/'impl/a.txt'],dotfile=[root/'impl/.py'],**{'directory-py':[root/'impl/dir.py']},relative=[Path('impl')])[case['group']]
            stack.enter_context(contextlib.chdir(root))
            if kind=='files':action=lambda:module.find_python_files(paths)
            else:action=lambda:values([v for path in module.find_python_files(paths) for v in module.check_file(path)])
        elif kind=='fact':
            raw=case['value'];action=lambda:module._violation_to_fact(module.Violation(**dict(raw,file=Path(raw['file']))))
        elif kind=='cli':
            files=[root/p for p in ['z.py','a/z.py','a-.py']]
            def find(paths):calls.append(dict(name='files',paths=paths));return files
            def check(path):
                calls.append(dict(name='scan',path=path))
                rows=[] if case['group']=='clean' else [module.Violation(path,12 if path==files[0] else 2,3,'message ☃','BT002' if path==files[0] else 'BT003','snippet 😀')]
                return rows+rows if case['group']=='duplicates' else rows
            stack.enter_context(patch.object(module,'find_python_files',find));stack.enter_context(patch.object(module,'check_file',check))
            stack.enter_context(patch.object(sys,'argv',['lint_backend_type_strings.py',*case['args']]))
            action=module.cli_main
        else:action=lambda:dict(backends=sorted(module.BACKEND_STRINGS),skip=module.SKIP_PATTERNS,
            patterns=[dict(pattern=p.pattern,flags=p.flags) for p in module.COMPARISON_PATTERNS],extractor=canonical(module.BACKEND_TYPE_STRINGS_EXTRACTOR))
        with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
            try:result=dict(value=action())
            except BaseException as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args)))
    return normalize(dict(result,stdout=out.getvalue(),stderr=err.getvalue(),calls=calls))


def install(backend,module,detectors=False):
    from backend_types_port import install as scanner_install
    from text_cli_port import install as cli_install
    scanner_install(backend,module,detectors=detectors);cli_install(backend,module,CHECK)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.backend-types-contract/v1',cases,exercise,install,['backend_types_contract.py','backend_types_port.py','text_cli_port.py','contract_runner.py'])
