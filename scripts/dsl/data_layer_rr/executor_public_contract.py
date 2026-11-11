"""Record raw executor text, path predicates, read warnings and Unicode handling."""
import contextlib
import io
from pathlib import Path
from contract_runner import main
from dto import canonical
from executor_cli_contract import CHECK,BAD,install


def cases(module):
    rows=[]
    for entry in ['check_source','check_file']:
        for i,source in enumerate([BAD,'pass\n','run_in_executor(\n',BAD+BAD,'_run_in_executor()\n','run_in_executor2()\n',BAD.rstrip()+' # noqa: async-executor\n','run_in_executor() # noqa: async-executor-other\n','run_in_executor()\u2028run_in_executor()\n','\u2003run_in_executor\u2003( )\u2003\n','run_in_executor()\r\n', '"run_in_executor()"\n']):rows.append(dict(kind='scan',id=entry+'-'+str(i),entry=entry,source=source))
    for entry in ['should_check_file','is_allowed']:
        for i,path in enumerate([*[p+'/a.py' for p in module.STRICT_DIRECTORIES],'outside.py','impl/sqlite/a.pyi','impl/sqlite/.py','prefix-impl/sqlite/a.py','impl/common/a.py','impl/sqlite/native_async_wrapper.py','other/tests/a.py','conform/a.py','l2000_connection/connection.py','/root/impl/common/project/impl/sqlite/a.py']):rows.append(dict(kind='predicate',id=entry+'-'+str(i),entry=entry,path=path))
    rows.extend([dict(kind='scan',id='missing',entry='check_file',missing=True),dict(kind='scan',id='binary',entry='check_file',binary='ff'),dict(kind='scan',id='directory',entry='check_file',directory=True)])
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();path=scratch/'source.py';out=io.StringIO();err=io.StringIO()
    if case.get('binary'):path.write_bytes(bytes.fromhex(case['binary']))
    elif case.get('directory'):path.mkdir()
    elif not case.get('missing'):path.write_text(case.get('source',''))
    with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
        try:
            value=getattr(module,case['entry'])(Path(case['path']) if case['kind']=='predicate' else case['source'] if case['entry']=='check_source' else path)
            result=dict(value=canonical(value,scratch))
        except Exception as error:result=dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))
    result.update(stdout=out.getvalue().replace(str(scratch),'$SCRATCH'),stderr=err.getvalue().replace(str(scratch),'$SCRATCH'));return result


if __name__=='__main__':
    main(CHECK,'rerail.executor-public/v1',cases,exercise,install,['executor_public_contract.py','executor_cli_contract.py'])
