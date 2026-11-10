"""Compare complete async capability scanner, helper, and CLI contracts."""
import contextlib
import io
from pathlib import Path
import sys
from unittest.mock import patch
from dto import canonical

CHECK='hasattr_async'
POSITIVE='hasattr(txn, "select_one_async")\n'


def cases(module):
    rows=[]
    def add(kind,name,**kw):rows.append(dict(kind=kind,id=name,**kw))
    texts=[POSITIVE,'getattr(txn, "fetch_async", None)',
        'hasattr(a,"first_async"); hasattr(b,"second_async"); getattr(a,"last_async", None)',
        'getattr(a,"first_async",None); hasattr(b,"second_async")',
        '# '+POSITIVE,'\t# '+POSITIVE,'pass # '+POSITIVE,'"'+POSITIVE+'"',
        'getattr(txn,"fetch_async",False)','getattr(txn,"fetch_async",none)',
        'hasattr(txn,"_async_helper")','hasattr(txn,"_fetch_async")',
        'hasattr(txn,"é_async")','hasattr(txn,"²_async")','hasattr(txn,"\u0301_async")',
        'hasattr(txn,"😀_async")','hasattr (txn, "fetch_async")','hasattr(txn,\t"fetch_async")',
        'invalid (: '+POSITIVE,'', 'pass\n']
    for i,text in enumerate(texts):add('source','source-'+str(i),source=text)
    for i,text in enumerate(texts[:8]):add('facts','facts-'+str(i),source=text)
    for sep in ['\n','\r\n','\r','\v','\f','\x1c','\x85','\u2028','\u2029']:
        add('source','separator-'+repr(sep),source='pass'+sep+POSITIVE)
    special={r'_has_sql_template_support\b':'_has_sql_template_support',
        r'''hasattr\(txn._conn, ['\"]_execute_template_with_params''':"hasattr(txn._conn, '_execute_template_with_params",
        r'''hasattr\(txn, ['\"]_conn['\"].*hasattr\(txn._conn''':"hasattr(txn, '_conn') and hasattr(txn._conn",
        r'''getattr\(scheme, ['\"]resolve_context_async''':"getattr(scheme, 'resolve_context_async",
        r'''getattr\(graph_scheme, ['\"]resolve_context_async''':"getattr(graph_scheme, 'resolve_context_async"}
    for i,pattern in enumerate(module.ALLOWED_PATTERNS):
        witness=special.get(pattern,pattern)
        add('source','allowed-source-'+str(i),source=POSITIVE.rstrip()+'; '+witness)
        add('allowed','allowed-helper-'+str(i),line=witness)
    for i,line in enumerate(['','ordinary_async','_has_sql_template_support_extra','_has_sql_template_supporté',
        '_has_sql_template_support\u0301','update_async_extra']):add('allowed','allowed-boundary-'+str(i),line=line)
    for i,path in enumerate(['tests/a.py','/repo/tests/a.py','/repo/test/a.py','/repo/Tests/a.py',
        '/repo/__pycache__/a.py','/repo/lint_hasattr_async.py','lint_hasattr_async.py',
        '/repo/lint_hasattr_async.py.extra',r'C:\repo\tests\a.py','/repo/atest/a.py']):add('exclude','exclude-'+str(i),path=path)
    for label,path,source in [('positive','a.py',POSITIVE),('non-python','notes.txt',POSITIVE),
        ('explicit-tests','tests/a.py',POSITIVE),('empty','a.py',''),('missing','missing.py',None),('invalid-utf8','a.py','ff')]:
        add('file','file-'+label,path=path,source=source)
    for group in ['empty','clean','positive','overlap','duplicates','default','excluded-only']:
        add('cli','cli-'+group,group=group)
    for arg in ['--help','--bad-flag']:add('cli','cli-'+arg,group='empty',arg=arg)
    add('configuration','configuration')
    return rows


def exercise(module,case,temporary):
    out,err=io.StringIO(),io.StringIO()
    with contextlib.ExitStack() as stack:
        kind=case['kind']
        if kind=='source':action=lambda:module.check_source(case['source'])
        elif kind=='facts':action=lambda:module._extract_hasattr_async_facts(temporary/'a.py',case['source'])
        elif kind=='allowed':action=lambda:module.is_allowed_line(case['line'])
        elif kind=='exclude':action=lambda:module.should_exclude(Path(case['path']))
        elif kind=='file':
            path=temporary/case['path'];path.parent.mkdir(parents=True,exist_ok=True)
            if case['source'] is not None:
                if case['id']=='file-invalid-utf8':path.write_bytes(bytes.fromhex(case['source']))
                else:path.write_text(case['source'])
            action=lambda:module.check_file(path)
        elif kind=='cli':
            group=case['group'];root=temporary/'project';root.mkdir()
            files=[] if group=='empty' else ['api/z.py','api/sub/a.py','api/a-.py','api/tests/ignored.py',
                'api/lint_hasattr_async.py','api/__pycache__/ignored.py','l1234_sample/a.py']
            if group=='excluded-only':files=['api/tests/ignored.py','api/lint_hasattr_async.py','api/__pycache__/ignored.py']
            for name in files:
                path=root/name;path.parent.mkdir(parents=True,exist_ok=True)
                path.write_text('pass\n' if group=='clean' else POSITIVE+('getattr(txn,"fetch_async",None)\n' if group=='duplicates' else ''))
            argv=['lint_hasattr_async.py','--root',str(root)]
            if group!='default':argv+=['--dirs','api',*(['api/sub','api'] if group=='overlap' else [])]
            if case.get('arg'):argv=[argv[0],case['arg']]
            stack.enter_context(patch.object(sys,'argv',argv));action=module.cli_main
        else:action=lambda:dict(directories=module.DIRECTORIES_TO_CHECK,allowed=module.ALLOWED_PATTERNS,
            exclude=module.EXCLUDE_PATTERNS,patterns=[dict(pattern=p.pattern,flags=p.flags) for p in [module.HASATTR_ASYNC_PATTERN,module.GETATTR_ASYNC_PATTERN]],
            extractor=canonical(module.HASATTR_ASYNC_EXTRACTOR))
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
    from hasattr_port import install as extractor_install
    extractor_install(backend,module)
    from hasattr_port import scan
    for entry in ['check_file','check_source','is_allowed_line','should_exclude']:
        def callback(entry):
            if entry=='check_file':
                def run(filepath):return scan(backend,entry,dict(filepath=filepath))
            elif entry=='check_source':
                def run(source):return scan(backend,entry,dict(source=source))
            elif entry=='is_allowed_line':
                def run(line):return scan(backend,entry,dict(line=line))
            else:
                def run(path):return scan(backend,entry,dict(path=path))
            run._rerail_native_check=CHECK
            return run
        backend.replace(module,entry,callback(entry))
    from text_cli_port import install as cli_install
    cli_install(backend,module,CHECK)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.hasattr-contract/v1',cases,exercise,install,
        ['hasattr_contract.py','hasattr_port.py','text_cli_port.py','contract_runner.py'])
