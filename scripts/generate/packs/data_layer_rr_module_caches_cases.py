"""Record independent witnesses for cache families, collector contexts, file admission and allowlist errors."""
import argparse
import json
from pathlib import Path

ALLOWLIST = 'scripts/allowlists/lint_module_level_caches_allowlist.json'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api', type=Path, required=True)
    parser.add_argument('--reporters', type=Path, required=True)
    args = parser.parse_args()
    api, reporters = [], []
    path = 'l1000_schema/sample.py'
    def add(identity, source, filename=path):
        api.append(dict(id=identity+'-content',check='module_level_caches',entry='scan_file_content',files=[],
            arguments=dict(rel_path=filename,abs_path=dict(path=filename),source=source,
                tree=dict(pythonAst='pass\n',mode='exec'))))
        api.append(dict(id=identity+'-collector',check='module_level_caches',entry='collector',files=[],
            arguments=dict(rel_path=filename,source='pass\n',tree=dict(pythonAst=source,mode='exec'))))
        reporters.append(dict(id=identity,check='module_level_caches',files=[dict(path=filename,source=source)]))
    values=['{}','dict()','dict(x=1)','dict(*[])','{1:2}','{**{}}','set()','set([])','{1}','[]','list()',
        'list([])','[None]','[x for x in xs]','builtins.dict()','OrderedDict()','WeakKeyDictionary()',
        'pkg.CustomCache(1)','make_cache()','ObjectCache()', '_ObjectCache()', 'IdentityTokenCache()',
        'WeakLocalMetadataCache()', 'ObjectCache(register_with_manager=False)',
        'ObjectCache(register_with_manager=0)','ObjectCache(register_with_manager=None)',
        'ObjectCache(**{"register_with_manager":False})',
        'ObjectCache(register_with_manager=False, register_with_manager=True)',
        'ObjectCache(register_with_manager=True, register_with_manager=False)', '(ObjectCache)(register_with_manager=False)',
        '{\n # comment\n}', '[\n # comment\n]']
    for i,value in enumerate(values):
        add('value-'+str(i),'# cache\n_holder = '+value+'\n')
    for i,name in enumerate(['values','CACHE','CACHE_VALUES','_CACHE','__all__','x_Y','漢字_文','Ä_B','ä_b',
            'cache','holder','memo','POOL','tokens','registry','TOKENS','regiſtry','not_related']):
        add('name-'+str(i),'# cache\n'+name+' = [None]\n')
    for i,source in enumerate(['x = {}\n', 'x = []\n','tokens=[None]\n','CACHE={}\n',
            'Cache={}\n','cache = {}\n','x={} # cache\n','x={} # CACHE\n']):
        add('trigger-'+str(i),source)
    for i,target in enumerate(['a = b','(a, b)','obj.cache','(a)','a: dict[str, int]','_CACHE']):
        add('target-'+str(i),'# cache\n'+target+' = {}\n')
    add('annotation-no-value','# cache\n_holder: dict[str,int]\n')
    add('augassign','# cache\n_holder += {}\n')
    add('nested-assignment','# cache\nif flag:\n    _holder={}\ntry:\n    a=[]\nexcept Exception:\n    b=set()\n')
    add('nonmodule-assignment','# cache\nclass C:\n    x={}\ndef f():\n    y={}\n')
    decorators=['cache','lru_cache()','functools.cache','functools.lru_cache(maxsize=None)',
        'other.cache','functools.sub.cache','cache()()','decorator(cache)','functools.cache[0]']
    for i,deco in enumerate(decorators):
        add('decorator-'+str(i),'# cache\n@'+deco+'\ndef f(): return 1\n')
    add('duplicate-decorators','@cache\n@lru_cache()\n@cache\ndef f(): return 1\n')
    add('class-methods','class C:\n    @cache\n    def f(self): return 1\n    @functools.lru_cache()\n    async def g(self): return 2\n')
    add('nested-callables','def outer():\n    @cache\n    def inner(): return 1\n    class C:\n        @cache\n        def f(self): return 1\n')
    add('nested-class','class Outer:\n    class Inner:\n        @cache\n        def f(self): return 1\n')
    add('conditional-class','if flag:\n    class C:\n        @cache\n        def f(self): return 1\n')
    add('unicode-columns','if flag: 变量 = {}; cache = {}\n')
    add('infrastructure','# cache\n_holder={}\n@cache\ndef f(): return 1\n','l1000_cache/a.py')
    add('legacy-infrastructure','# cache\n_holder={}\n@cache\ndef f(): return 1\n','impl/common/l1000_cache/a.py')
    for i,filename in enumerate(['impl/a.py','data_layer/a.py','_internal/a.py','api/a.py','other/a.py',
            'l１２３４_layer/a.py','l123_foo/a.py','l1000_schema/tests/a.py','l1000_schema/test_a.py',
            'l1000_schema/conform/a.py','l1000_schema/benchmarks/a.py','l1000_schema/examples/a.py',
            'l1000_schema/.pytest_cache/a.py','l1000_schema/node_modules/a.py','l1000_schema/.venv/a.py',
            'l1000_schema/tmp/a.py','l1000_schema/browser_build/a.py']):
        source='# cache\n_holder={}\n'
        add('scope-'+str(i),source,filename)
        for include in [False,True]:
            api.append(dict(id='scan-'+str(i)+'-'+str(include),check='module_level_caches',entry='scan_codebase',
                files=[dict(path=filename,source=source)],arguments=dict(root=dict(path=''),include_tests=include)))
    for i,dirs in enumerate([['api'],['api','api'],['api','api/sub'],['.'],['missing']]):
        api.append(dict(id='directory-batch-'+str(i),check='module_level_caches',entry='scan_codebase',
            files=[dict(path='api/a.py',source='# cache\na={}\n'),dict(path='api/sub/b.py',source='# cache\nb=[]\n')],
            arguments=dict(root=dict(path=''),dirs=dirs)))
    invalid=[('syntax','def bad(:\n'),('decode',None),('clean','pass\n')]
    for identity,source in invalid:
        f=dict(path=path,source=source) if source is not None else dict(path=path,bytesHex='fffe')
        api.append(dict(id='count-'+identity,check='module_level_caches',entry='scan_codebase',files=[f],arguments=dict(root=dict(path=''))))
    entry=dict(path=path,pattern_type='dict_cache',name='_holder',classification='local_metadata_memo',reason='Reviewed cache.')
    allowlists=[('missing',None),('empty',{}),('valid',dict(entries=[entry])),
        ('ordered',dict(entries=[dict(entry,name='z'),dict(entry,name='a')])),
        ('missing-reason',dict(entries=[dict(entry,reason='')])),
        ('blank-classification',dict(entries=[dict(entry,classification='\u001c ')])),
        ('nonobject',[]),('bad-entries',dict(entries={})),('bad-item',dict(entries=[False])),
        ('missing-name',dict(entries=[{k:v for k,v in entry.items() if k!='name'}])),
        ('duplicate',dict(entries=[entry,dict(entry,reason='Another reason.')])),
        ('whitespace',dict(entries=[dict(entry,reason='  retained\n ',classification=' retained ')]))]
    for identity,value in allowlists:
        files=[] if value is None else [dict(path=ALLOWLIST,source=json.dumps(value,ensure_ascii=True))]
        api.append(dict(id='allow-'+identity,check='module_level_caches',entry='_load_allowlist_entries',files=files,
            arguments=dict(allowlist_path=dict(path=ALLOWLIST))))
        if identity!='missing-name':
            reporters.append(dict(id='allow-'+identity,check='module_level_caches',files=[
                dict(path=path,source='# cache\n_holder={}\n'),*files]))
    for i,source in enumerate(['# cache\na={}\nb=[]\nc=set()\n','a=CustomCache()\n',
            '@cache\ndef f(): pass\n','class C:\n    @cache\n    def f(self): pass\n']):
        api.append(dict(id='format-'+str(i),check='module_level_caches',entry='scan_and_format',files=[],
            arguments=dict(rel_path=path,abs_path=dict(path=path),source=source)))
    for i,severities in enumerate([[],[1],[2,3,2,1,3,3]]):
        api.append(dict(id='counts-'+str(i),check='module_level_caches',entry='counts',files=[],arguments=dict(severities=severities)))
    for target,rows in [(args.api,api),(args.reporters,reporters)]:
        assert len({r['id'] for r in rows})==len(rows)
        target.parent.mkdir(parents=True,exist_ok=True)
        target.write_text(json.dumps(dict(cases=rows),indent=2,ensure_ascii=False)+'\n')
    print(len(api),'API cases;',len(reporters),'reporter cases')


if __name__=='__main__':
    main()
