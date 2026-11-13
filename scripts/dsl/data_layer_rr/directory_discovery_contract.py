"""Compare the plan-scope linter's actual FileScopeSpec plan with native directory discovery."""
from pathlib import Path
from contract_runner import main
from dto import canonical
from replay import write_fixture
CHECK='plan_scope_side_effects'


def cases(_module):
    def file(path):return dict(path=path,source='with open_write_plan():\n lazy.resolve()\n')
    groups=dict(empty=[],roots=[file(p+'/a.py')for p in ['api','impl','data_layer','_internal','l1234_good','other','tests']],
        layer_names=[file(p+'/a.py')for p in ['l1234_','l1234_x','prefixl1234_x','l١٢٣٤_x','l１２３４_x','l123_x','l12345_x','l1234_\nx','l1234_x\nmore']],
        nested=[file('impl/'+p+'/a.py')for p in ['normal','tests','test','Tests','venv','.venv','tmp','.git','__pycache__','.hidden','node_modules','.pytest_cache']],
        suffixes=[file('impl/'+p)for p in ['.py','.hidden.py','a.py','a.PY','a.pyi','a.txt','test_a.py']],
        file_links=[file('impl/a.py'),dict(path='impl/alias.py',symlink='a.py')],
        directory_links=[file('impl/sub/a.py'),dict(path='impl/link',symlink='sub')],
        directory_loop=[file('impl/sub/a.py'),dict(path='impl/sub/up',symlink='..')],
        linked_root=[file('shared/a.py'),dict(path='impl',symlink='shared')],
        broken_file=[dict(path='impl/broken.py',symlink='absent')],
        directory_named_py=[file('impl/sub/a.py'),dict(path='impl/alias.py',symlink='sub')],
        root_files=[dict(path='impl',source='pass\n'),dict(path='data_layer',source='pass\n')],
        ordering=[file('impl/'+p)for p in ['z.py','a/z.py','a-.py','a.py']],
        unicode=[file('impl/é\nname.py'),file('impl/名前.py')])
    return [dict(kind='discovery',id=name,files=files)for name,files in groups.items()]+[dict(
        kind='updates',id='link_target_and_tree_updates',
        files=[file('impl/regular.py'),file('shared/target'),dict(path='impl/alias.py',symlink='../shared/target')],
        steps=[[],[dict(op='remove',path='shared/target'),dict(op='directory',path='shared/target'),dict(op='file',path='shared/target/inside.py')],
            [dict(op='remove',path='shared/target/inside.py'),dict(op='rmdir',path='shared/target'),dict(op='file',path='shared/target')],
            [dict(op='remove',path='shared/target')],[dict(op='remove',path='impl/alias.py')],
            [dict(op='file',path='impl/child/positive.py')],
            [dict(op='remove',path='impl/child/positive.py'),dict(op='rmdir',path='impl/child'),dict(op='link',path='impl/child',target='../shared')],
            [dict(op='remove',path='impl/regular.py'),dict(op='file',path='impl/regular.py/inside.py')],
            [dict(op='file',path='impl/regular.py/inside.py',source='changed = True\n')],
            [dict(op='file',path='impl/line\nbreak/a.py')],
            [dict(op='remove',path='impl/line\nbreak/a.py'),dict(op='file',path='impl/line\nbreak/b.py')],
            [dict(op='remove',path='impl/line\nbreak/b.py'),dict(op='rmdir',path='impl/line\nbreak')],
            [dict(op='file',path='impl/tab\tname/a.py')],
            [dict(op='remove',path='impl/tab\tname/a.py'),dict(op='file',path='impl/tab\tname/b.py')],
            [dict(op='remove',path='impl/tab\tname/b.py'),dict(op='rmdir',path='impl/tab\tname')]])]


def exercise(module,case,scratch):
    root=scratch.resolve()/'repo';root.mkdir();write_fixture(root,case['files'])
    def discover():
        try:
            if hasattr(module,'_rerail_directory_discovery'):files=module._rerail_directory_discovery(root)
            else:
                from scripts.lint_lib.fact_engine import FactEngine
                from scripts.lint_lib.fact_store import JsonFactStore
                plugin=module.plan_scope_side_effects_plugin(root)
                engine=FactEngine(root=root,fact_store=JsonFactStore(root/'tmp/facts'),extractors={},enable_fact_cache=False)
                files=[p.relative_to(root).as_posix()for p in engine.plan((plugin,)).files]
            return dict(value=canonical(files,scratch.resolve()))
        except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch.resolve())))
    if case['kind']=='discovery':return discover()
    results=[]
    for step in case['steps']:
        for change in step:
            path=root/change['path'];op=change['op']
            if op=='remove':path.unlink()
            elif op=='rmdir':path.rmdir()
            elif op=='directory':path.mkdir()
            elif op=='link':path.symlink_to(change['target'])
            elif op=='file':
                path.parent.mkdir(parents=True,exist_ok=True);path.write_text(change.get('source','pass\n'))
            else:raise ValueError(op)
        results.append(discover())
    return dict(stages=results)


def install(backend,module,detectors=False):
    def discover(root):
        rows=backend.native(CHECK,'_directory_discovery',[],root=root)
        if any(r['code']!='PSE_DISCOVERY'for r in rows):raise ValueError('Unexpected discovery response')
        return [r['parameters']['path']for r in sorted(rows,key=lambda r:r['parameters']['path_order'])]
    module._rerail_directory_discovery=discover

if __name__=='__main__':main(CHECK,'rerail.plan-scope-directory-discovery/v1',cases,exercise,install,['directory_discovery_contract.py'])
