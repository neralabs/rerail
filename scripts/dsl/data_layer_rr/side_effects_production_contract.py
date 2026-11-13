"""Record complete production selection, fact presence, raw payloads, and findings."""
from dataclasses import asdict
from first_ten_production_contract import normalize
from dto import canonical
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
        same_line=[file('impl/a.py','with open_write_plan(): lazy.resolve(); lazy.resolve()\n')],
        batch=[file(f'impl/file{i:03}.py')for i in range(64)],
    )
    return [dict(kind='production',id=name,files=files)for name,files in groups.items()]


def exercise(module,case,scratch):
    root=scratch/'repo';root.mkdir();write_fixture(root,case['files'])
    try:
        if hasattr(module,'_rerail_side_effects_production'):return module._rerail_side_effects_production(root)
        from scripts.lint_lib.fact_engine import FactEngine
        from scripts.lint_lib.fact_store import JsonFactStore
        plugin=module.plan_scope_side_effects_plugin(root)
        engine=FactEngine(root=root,fact_store=JsonFactStore(root/'tmp/facts'),extractors=module.plan_scope_side_effects_fact_extractors(root),enable_fact_cache=False)
        plan=engine.plan([plugin]);view=engine.materialize((plugin,),max_workers=1)
        return normalize(dict(selectedFiles=[path.relative_to(root).as_posix()for path in plan.files],
            facts={path.relative_to(root).as_posix():bundle.facts for path,bundle in sorted(view.files.items())},
            findings=[asdict(value)for value in plugin.reporter(view)],
            plugin=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description)),root)
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from side_effects_production_port import production
    module._rerail_side_effects_production=lambda root:production(backend,root)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.side-effects-production/v1',cases,exercise,install,['side_effects_production_contract.py','side_effects_cli_contract.py','first_ten_production_contract.py'])
