"""Record ordered dynamic-loading repository results, including positive files."""
from dto import canonical
from replay import write_fixture

CHECK='dynamic_module_loading'
BAD='importlib.import_module("os")\n'


def cases(_module):
    def file(path,source=BAD):return dict(path=path,source=source)
    def case(name,files,dirs=None,**extra):return dict(kind='discovery',id=name,files=files,directories=dirs,**extra)
    rows=[
        case('legacy-default',[file('api/a.py'),file('impl/b.py'),file('other/c.py')]),
        case('modern',[file(p+'/a.py') for p in ['api','impl','data_layer','_internal','l1000_one','l9000_last','l1000_']]),
        case('empty-modern',[dict(path='data_layer',directory=True),file('api/a.py')]),
        case('skip-trees',[file('impl/kept.py')]+[file('impl/'+p+'/a.py') for p in ['__pycache__','.pytest_cache','node_modules','.git','.venv','tmp','browser_build','tests','test','conform']]),
        case('venv-is-not-excluded',[file('impl/venv/a.py')]),
        case('explicit-files',[file('tests/a.py'),file('tmp/b.py'),file('notes.txt')],['tests/a.py','tmp/b.py','notes.txt','missing.py']),
        case('duplicate-roots',[file('impl/a.py')],['impl','impl']),
        case('overlap-roots',[file('impl/a.py'),file('impl/sub/b.py')],['impl','impl/sub']),
        case('public-deduplicates',[file('impl/a.py'),file('data_layer/b.py'),file('_internal/c.py')],['public','impl','public','_internal']),
        case('explicit-empty',[file('impl/a.py')],[]),
        case('no-directories',[]),
        case('nested-directory-link',[file('pkg/a.py'),file('other/b.py'),dict(path='pkg/alias',symlink='../other')],['pkg']),
        case('explicit-directory-link',[file('other/a.py'),dict(path='alias',symlink='other')],['alias']),
        case('broken-explicit-link',[dict(path='missing.py',symlink='absent.py'),file('impl/a.py')],['missing.py','impl']),
        case('broken-nested-link',[dict(path='impl/missing.py',symlink='absent.py'),file('impl/a.py')]),
        case('file-link',[file('shared.txt'),dict(path='impl/alias.py',symlink='../shared.txt')]),
        case('component-order',[file(p) for p in ['impl/a.py','impl/a/z.py','impl/a-/x.py','impl/a_/y.py']]),
        case('suffixes',[file('impl/'+p) for p in ['.py','..py','...py','.hidden.py','a.py','a.PY']]),
        case('directory-cycle',[file('pkg/a.py'),dict(path='pkg/back',symlink='..')],['pkg']),
        case('normalized-relative',[file('impl/a.py')],['./impl//']),
        case('dot-root',[file('pkg/a.py'),file('tmp/b.py'),file('tests/c.py')],['.']),
        case('unicode-layers',[file(p+'/a.py') for p in ['l١٢٣٤_layer','l1234_a\nb','l1234_\nb','l12345_bad','l1234_']]),
        case('allowlist',[file('impl/a.py'),file('impl/b.py')],allowed=['impl/a.py']),
        case('all-allowlisted',[file('impl/a.py')],allowed=['impl/a.py']),
        case('content-order',[file('impl/a.py','__import__("os")\nimport_module("x")\n'),file('impl/b.py')]),
        case('safe-between-positives',[file('impl/a.py'),file('impl/b.py','x=1\n'),file('impl/c.py')]),
    ]
    return rows


def exercise(module,case,temporary):
    root=temporary/'root';root.mkdir();write_fixture(root,case['files'])
    previous=list(module.ALLOWLISTED_FILES)
    if 'allowed' in case:module.ALLOWLISTED_FILES[:]=case['allowed']
    try:
        value=module.find_violations(root,case['directories'])
        return dict(value=canonical(value,temporary))
    except BaseException as error:
        return dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=canonical(error.args,temporary)))
    finally:
        # The fixture specifies the complete override for each call.
        module.ALLOWLISTED_FILES[:]=previous


if __name__=='__main__':
    from contract_runner import main
    from dynamic_port import install
    main(CHECK,'rerail.dynamic-discovery-contract/v1',cases,exercise,install,['dynamic_discovery_contract.py','dynamic_port.py'])
