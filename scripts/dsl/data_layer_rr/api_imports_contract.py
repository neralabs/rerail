"""Source oracle for API imports, literal examples, fact DTOs, discovery and CLI."""
import contextlib
import io
import json
from pathlib import Path
import sys
from unittest.mock import patch
from dto import canonical

CHECK='no_api_root_imports'
WITNESS='from api import LENGTH\n'


def cases(_module):
    result=[]
    imports=['from api import X','from api import X as Y, Z','from api import *','from api import (X, Y)',
        'from api import (\n X,\n Y,\n)','from .api import X','# import api\nfrom .api import X',
        'from ..api import X','from   api import X','# import api\nfrom   api import X',
        'from ａｐｉ import X','# import api\nfrom ａｐｉ import X','import api','import api as root',
        'from api.l1400_types import X','from apis import X','from api import ＬENGTH',
        'def f():\n from api import X','if True:\n from api import X','from api import X; from api import Y']
    for i,text in enumerate(imports):result.append(dict(kind='file',id=f'import-{i}',source=text+'\n'))
    values=['from api import X','import api','import api\nimport api\nimport api',
        'from api import X\nimport api\nfrom api import Y','\n\tfrom api import X',
        'import api\u00a0\n','import api as root','not from api import X','from api import ',
        'from api import X\r\nimport api\r\n','😀\nfrom api import X','import api\n\nimport api']
    for prefix in ['', 'r', 'u', 'b', 'f']:
        for i,value in enumerate(values):
            result.append(dict(kind='file',id=f'literal-{prefix}-{i}',source='value = '+prefix+json.dumps(value,ensure_ascii=False)+'\n'))
    for name,text in [('concat','value = ("from api import X\\n"\n "import api")\n'),
        ('docstring','"""line\nfrom api import X\nimport api\n"""\n'),
        ('f-inner', '''value = f"{'from api import X'}"\n'''),
        ('f-order', '''value = f"import api\\n{'from api import X'}\\nimport api"\n'''),
        ('string-order','value = "import api\\nfrom api import X"\n'),
        ('syntax','from api import (\n'),('clean','pass\n'),('missing',None),('invalid-utf8','ff')]:
        result.append(dict(kind='file',id=name,source=text))
    for disk in [WITNESS,'pass\n',None]:
        for supplied in ['pass','from api','import api']:
            result.append(dict(kind='content',id=f'content-{len(result)}',source=disk,supplied=supplied))
    for path in ['tests/a.py','test/a.py','tmp/a.py','docs/a.py','.py','a.txt','../outside.py']:
        result.append(dict(kind='content',id='scope-'+path,path=path,source=WITNESS,supplied='from api'))
    for relative in [True,False]:
        for paths in [[],['impl/a.py'],['impl/a.py','impl/a.py'],['test/a.py'],['tests/a.py'],['a.txt']]:
            result.append(dict(kind='batch',id=f'batch-{len(result)}',paths=paths,relative=relative))
    for excluded in [[],['docs'],['scripts','.git'],['docs/example.py'],['skip.py']]:
        result.append(dict(kind='files',id='files-'+repr(excluded),excluded=excluded))
    for line in [None,True,False,1,2**100,-2,2.5,'3',[],{}]:
        for message in ['message',None]:
            result.append(dict(kind='from-fact',id=f'fact-{len(result)}',raw=dict(line=line,message=message,file_path='./a//b.py')))
    for raw in [{},{'line':2,'message':'ok'},{'line':2,'message':'ok','file_path':None},
                {'line':2,'message':'ok','file_path':['☃',1]}]:
        result.append(dict(kind='from-fact',id=f'fact-{len(result)}',raw=raw))
    for path,line,message in [('impl/a.py',2,'message'),('../a.py',True,['x']),('//network/a.py',2**100,None)]:
        result.append(dict(kind='to-fact',id=f'to-fact-{len(result)}',path=path,line=line,message=message))
    for group in ['clean','one','duplicates','ordering','outside','late-outside']:
        for args in [[],['--check'],['--unknown','--check=yes']]:
            result.append(dict(kind='cli',id='cli-'+group+repr(args),group=group,args=args))
    result.append(dict(kind='configuration',id='configuration'))
    return result


def exercise(module,case,temporary):
    root=temporary/'project';root.mkdir();calls=[]
    def normalize(v):
        if isinstance(v,Path):return normalize(str(v))
        if isinstance(v,str):return v.replace(str(temporary),'$TMP')
        if isinstance(v,(list,tuple)):return [normalize(x) for x in v]
        if isinstance(v,dict):return {k:normalize(x) for k,x in v.items()}
        if isinstance(v,set):return sorted(v)
        return v
    def rows(values):return [dict(path=v.file_path,line=v.line_number,message=v.message) for v in values]
    out,err=io.StringIO(),io.StringIO()
    with contextlib.ExitStack() as stack:
        stack.enter_context(patch.object(module,'get_project_root',lambda:root))
        kind=case['kind']
        if kind in {'file','content'}:
            path=root/case.get('path','impl/a.py');path.parent.mkdir(parents=True,exist_ok=True)
            if case['source'] is not None:
                if case['id']=='invalid-utf8':path.write_bytes(bytes.fromhex(case['source']))
                else:path.write_text(case['source'])
            action=lambda:rows(module.check_file(path) if kind=='file' else module.check_file_content(path,case['supplied']))
        elif kind=='batch':
            for p in case['paths']:
                path=root/p;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(WITNESS)
            paths=tuple(Path(p) if case['relative'] else root/p for p in case['paths'])
            def action():
                groups={}
                for row in rows(module._violations_for_files(root,paths)):groups.setdefault(str(row['path']),[]).append(row)
                return [r for p in sorted(groups) for r in groups[p]]
        elif kind=='files':
            for p in ['z.py','a.py','docs/example.py','scripts/a.py','.git/x.py','skip.py','a.txt']:
                path=root/p;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(WITNESS)
            action=lambda:module.find_python_files(root,set(case['excluded']))
        elif kind=='from-fact':
            action=lambda:None if (value:=module._violation_from_fact(case['raw'],root/'fallback.py')) is None else rows([value])[0]
        elif kind=='to-fact':action=lambda:module._violation_to_fact(module.Violation(Path(case['path']),case['line'],case['message']))
        elif kind=='cli':
            values=[] if case['group']=='clean' else [module.Violation(root/p,l,m) for p,l,m in
                [('z.py',12,'message ☃'),('a/z.py',2,'message 😀'),('a-.py',2,'line\nnext')]]
            if case['group']=='duplicates':values+=values[:1]
            if case['group']=='outside':values=[module.Violation(temporary/'outside.py',1,'outside')]
            if case['group']=='late-outside':values+=[module.Violation(Path(str(root)+'z')/'outside.py',1,'outside')]
            def files(r,excluded):calls.append(dict(name='files',root=r,excluded=excluded));return [root/'a.py']
            def scan(r,files):calls.append(dict(name='scan',root=r,files=files));return values
            stack.enter_context(patch.object(module,'find_python_files',files));stack.enter_context(patch.object(module,'_violations_for_files',scan))
            stack.enter_context(patch.object(sys,'argv',['lint_no_api_root_imports.py',*case['args']]))
            action=module.cli_main
        else:action=lambda:dict(patterns=[dict(pattern=p.pattern,flags=p.flags) for p in [module.ROOT_IMPORT_RE,module.ROOT_IMPORT_TEXT_RE]],
                                extractor=canonical(module.NO_API_ROOT_IMPORTS_EXTRACTOR))
        with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
            try:result=dict(value=action())
            except BaseException as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args)))
    return normalize(dict(result,stdout=out.getvalue(),stderr=err.getvalue(),calls=calls))


def install(backend,module,detectors=False):
    from api_imports_port import install as implementation
    return implementation(backend,module,detectors=detectors)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.api-imports-contract/v1',cases,exercise,install,['api_imports_contract.py','api_imports_port.py','contract_runner.py'])
