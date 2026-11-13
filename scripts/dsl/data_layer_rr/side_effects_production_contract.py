"""Record complete production selection, fact presence, raw payloads, and findings."""
from first_ten_production_contract import source_case
from replay import write_fixture
from side_effects_cli_contract import CHECK, BAD, AUDIT


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    groups=dict(
        empty=[],positive=[file('impl/a.py')],audit=[file('impl/a.py',AUDIT)],
        roots=[file(path+'/a.py')for path in ['api','impl','data_layer','_internal','l1234_a','l١٢٣٤_a','l１２３４_a','l1234_','l1234_\nx','l1234_x\nmore','other','tests']],
        nested=[file('impl/'+path+'/a.py')for path in ['normal','tests','test','conform','venv','.venv','tmp','.git','__pycache__','.pytest_cache','.mypy_cache','node_modules','.hidden']],
        suffixes=[file('impl/'+path)for path in ['.py','.hidden.py','a.py','a.pyi','a.PY','a.txt','test_a.py','a_test.py']],
        file_links=[file('impl/a.py'),dict(path='impl/link.py',symlink='a.py')],
        alias_other=[file('other/a.py'),dict(path='impl/alias.py',symlink='../other/a.py')],
        alias_test=[file('impl/tests/a.py'),dict(path='impl/alias.py',symlink='tests/a.py')],
        alias_reverse=[file('impl/a.py'),dict(path='impl/tests/alias.py',symlink='../a.py')],
        root_link=[file('other/a.py'),dict(path='impl',symlink='other')],
        directory_link=[file('impl/sub/a.py'),dict(path='impl/link',symlink='sub')],
        broken_link=[dict(path='impl/broken.py',symlink='absent.py')],
        clean=[file('impl/a.py','pass\n')],trigger_only=[file('impl/a.py','# open_write_plan\n')],
        syntax=[file('impl/a.py','with open_write_plan(:\n')],
        no_trigger=[file('impl/a.py','with open_wr𝚒te_plan(): lazy.resolve()\n')],
        comment_trigger=[file('impl/a.py','# open_write_plan\nwith open_wr𝚒te_plan(): lazy.resolve()\n')],
        duplicate=[file('impl/a.py',BAD+BAD)],mixed=[file('impl/a.py',BAD+AUDIT)],
        order=[file('impl/'+path)for path in ['z.py','a/z.py','a-.py','a.py']],
        unicode=[file('impl/é\nname.py')],empty_file=[file('impl/a.py','')],
    )
    return [dict(kind='production',id=name,files=files)for name,files in groups.items()]


def exercise(module,case,scratch):
    if not hasattr(module,'_rerail_side_effects_production'):return source_case(module,dict(case,check=CHECK))
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    return module._rerail_side_effects_production(root)


def install(backend,module,detectors=False):
    pass


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.side-effects-production/v1',cases,exercise,install,['side_effects_production_contract.py','side_effects_cli_contract.py','first_ten_production_contract.py'])
