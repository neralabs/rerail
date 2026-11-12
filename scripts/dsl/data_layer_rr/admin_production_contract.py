"""Record complete admin production discovery, raw facts, alias ownership and findings."""
from first_ten_production_contract import source_case
from dto import canonical
from replay import write_fixture
from admin_public_contract import CHECK,BAD


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    base='l8000_admin/tools/'
    groups=dict(empty=[],roots=[file(p+'/a.py') for p in ['l8000_admin/tools','l8000_admin/other','other','api','impl']],
        nested=[file(base+p+'/a.py') for p in ['normal','tests','test','Tests','venv','.venv','tmp','.git','__pycache__','.hidden','vendor','latest_folder']],
        suffixes=[file(base+p) for p in ['.py','.hidden.py','a.py','a.PY','a.pyi','a.txt','test_a.py','latest_a.py']],
        file_links=[file(base+'a.py'),dict(path=base+'link.py',symlink='a.py')],
        alias_skip=[file(base+'test_a.py'),dict(path=base+'alias.py',symlink='test_a.py')],
        alias_reverse=[file(base+'a.py'),dict(path=base+'test_alias.py',symlink='a.py')],
        directory_link=[file(base+'sub/a.py'),dict(path=base+'link',symlink='sub')],
        clean=[file(base+'a.py','pass\n')],trigger_only=[file(base+'a.py','# run_in_executor\n')],
        prefilter=[file(base+'a.py','asyncio . to_thread(work)\n')],
        invalid=[file(base+'a.py','def broken(: asyncio.to_thread(work)\n')],
        duplicate=[file(base+'a.py',BAD+BAD)],
        order=[file(base+p) for p in ['z.py','a/z.py','a-.py','a.py']],
        line_order=[file(base+'a.py','run_in_executor(work)\n'*12)],
        unicode=[file(base+'a.py','☃run_in_executor(work)\né_run_in_executor(work)\nrun_in_executor☃(work)\n')],
        splitlines=[file(base+'a.py','asyncio.to_thread(work)\x85run_in_executor(work)\u2028disable_sync_in_async_check()\n')],
        strings=[file(base+'a.py','value = "asyncio.to_thread(work)"\n')],
        inline_comment=[file(base+'a.py','pass # run_in_executor(work)\n')],
        docstring=[file(base+'a.py','"""\n'+BAD+'"""\n'+BAD)],
        quoted_comment=[file(base+'a.py','# """\n'+BAD+'# \'\'\'\n'+BAD)],
        same_line_duplicates=[file(base+'a.py','run_in_executor(a); run_in_executor(b)\n')])
    return [dict(kind='production',id=name,files=files) for name,files in groups.items()]


def exercise(module,case,scratch):
    if not hasattr(module,'_rerail_admin_production'):return source_case(module,dict(case,check=CHECK))
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    try:return module._rerail_admin_production(root)
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from admin_production_port import production
    module._rerail_admin_production=lambda root:production(backend,root)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.admin-production/v1',cases,exercise,install,['admin_production_contract.py','first_ten_production_contract.py'])
