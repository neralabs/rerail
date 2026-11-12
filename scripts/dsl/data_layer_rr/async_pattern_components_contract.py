"""Record async-pattern registration, collection, projections, coercions, and raw reporting."""
import ast
from dataclasses import fields
from pathlib import Path
from contract_runner import main
from dto import canonical

CHECK='async_patterns'
BAD='async def work(txn):\n    return txn.select_one()\n'
CONSTANTS=['VIOLATIONS','SYNC_WRITE_METHODS','SYNC_READ_METHODS','SYNC_INTERNAL_IO_METHODS','SYNC_CONNECTION_METHODS','SYNC_POLYMORPHIC_METADATA_METHODS','ALL_SYNC_TRANSACTION_METHODS','SCAN_PATHS','EXCLUDE_PATTERNS','TRANSACTION_PARAM_NAMES','TRANSACTION_TYPE_NAMES','QUERY_SOURCE_METHODS','SOURCE_TRIGGERS']


def hydrate(value):
    if isinstance(value,str) and value in {'$NAN','$INF','$NEG_INF'}:return {'$NAN':float('nan'),'$INF':float('inf'),'$NEG_INF':float('-inf')}[value]
    if isinstance(value,list):return [hydrate(v) for v in value]
    if isinstance(value,dict):return {k:hydrate(v) for k,v in value.items()}
    return value


def cases(_module):
    rows=[dict(kind='metadata',id='metadata')]
    texts=dict(positive=BAD,clean='pass\n',trigger_only='# async def\n',invalid='async def broken(:\n',
        duplicate=BAD+BAD,abstract='class ExampleQuery:\n    async def work(self):\n        return x.to_thread(work)\n',
        marked='# ASYNC_FALLBACK:\n'+BAD,connection='async def work(self):\n    return self._conn.read(callback)\n',
        decorated='@s.write_async()\nasync def work():\n    other.insert(x)\n',nested='async def outer(txn):\n    def inner():\n        txn.select_one()\n    txn.select_one()\n')
    rows.extend(dict(kind='extract',id='extract-'+name,source=text) for name,text in texts.items())
    for path in ['api/a.py','impl/common/a.py','other/a.py','api/test_a.py']:
        rows.append(dict(kind='collector',id='collector-'+path,path=path,source=BAD,tree=BAD))
    rows.extend([dict(kind='collector',id='collector-ignores-prefilter',path='api/a.py',source='pass\n',tree=BAD),
                 dict(kind='collector',id='collector-disagreeing-source',path='api/a.py',source='# ASYNC_FALLBACK: async def\n',tree=BAD)])
    for i,path in enumerate(['a.py','a/../b.py','/external.py','.','//network/a.py','./a//b.py','///a/b.py','repo/a.py']):
        rows.append(dict(kind='project',id='project-'+str(i),path=path,line=[0,True,False,-1,2**100,1,2,3][i]))
    hit=dict(path='supplied.py',line=4,code='CUSTOM',message='supplied',context='context',is_fallback=False)
    values=[None,{},'wrong',0,[],[hit],hit]
    values.extend({k:v for k,v in hit.items() if k!=missing} for missing in ['path','line','code','message'])
    samples=[None,True,False,0,-1,2**100,2.9,-2.9,'  +００２  ','bad',[],[1],{},dict(a=1),'$NAN','$INF','$NEG_INF']
    for key in ['path','line','code','message','context','is_fallback']:
        values.extend(dict(hit,**{key:v}) for v in samples)
    rows.extend(dict(kind='load',id='load-'+str(i),raw=v) for i,v in enumerate(values))
    for i,root in enumerate(['.','relative','/','//network','/external']):rows.append(dict(kind='load',id='load-root-'+str(i),root=root,raw=hit))
    containers=[None,{},'wrong',0,[],[None,0,'wrong',[]],[{}],[hit],[hit,hit],
        [dict(hit,path=p) for p in ['a/z.py','a-.py','a.py','/external.py','a/../b.py']],
        [dict(hit,line=v) for v in [True,False,-2,0,2**100,-2**100,'3',2.5]],
        [dict(hit,is_fallback=v) for v in [None,False,0,'',[],{},True,1,'False',[0],dict(a=0)]],
        [dict(hit,context=v,code=v,message=v) for v in [None,True,2.5,['☃',1],dict(a=1)]],
        [dict(hit,line='$INF')], [dict(hit,path='z.py'),dict(hit,path='a.py',line='$INF')]]
    rows.extend(dict(kind='report',id='report-'+str(i),bundles=[dict(key='ignored.py',path='foreign.py',value=v)]) for i,v in enumerate(containers))
    rows.extend([dict(kind='report',id='view-root-does-not-bind-plugin',view_root='foreign',bundles=[dict(key='ignored.py',path='other.py',value=[hit])]),
        dict(kind='report',id='missing-namespace',bundles=[dict(key='a.py',path='a.py',facts={})]),
        dict(kind='report',id='foreign-namespace',bundles=[dict(key='a.py',path='a.py',facts={'other':[hit]})]),
        dict(kind='report',id='bundle-order',bundles=[dict(key=p,path='ignored.py',value=[dict(hit,path='same.py',message=p)]) for p in ['z.py','a/z.py','a-.py','a.py']])])
    return rows


def exercise(module,case,scratch):
    from scripts.lint_lib.fact_engine import materialize_source_facts
    from scripts.lint_lib.linter_plugin import PerFileFactBundle,ProjectFactView
    from scripts.lint_lib.ast_collectors import AstTraversalContext
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir();family=module.ASYNC_PATTERNS_EXTRACTOR.extractor_id
    try:
        if case['kind']=='metadata':
            plugin=module.async_patterns_plugin(root);value={f.name:getattr(plugin,f.name) for f in fields(plugin) if f.name!='reporter'}
            value.update(registry=list(module.async_patterns_fact_extractors(root)),constants={n:getattr(module,n) for n in CONSTANTS})
        elif case['kind']=='extract':
            bundle=materialize_source_facts(path=root/'api/a.py',source=case['source'],specs=(module.ASYNC_PATTERNS_EXTRACTOR,),extractors=module.async_patterns_fact_extractors(root))
            value=dict(facts=bundle.facts,errors=bundle.errors)
        elif case['kind']=='collector':
            path=root/case['path'];owner=module.AsyncPatternCollector(path,case['source'],root);tree=ast.parse(case['tree'])
            module._collect_async_pattern_node(tree,owner,AstTraversalContext(path,case['source'],{},{}));value=dict(violations=owner.violations,facts=owner.finish_file())
        elif case['kind']=='project':
            owner=module.Violation(Path(case['path']),case['line'],'CODE','message ☃','context',False);value=module._violation_to_fact(owner,root)
        elif case['kind']=='load':value=module._violation_from_fact(hydrate(case['raw']),Path(case['root']) if 'root' in case else root)
        else:
            plugin=module.async_patterns_plugin(root);bundles={Path(b['key']):PerFileFactBundle(Path(b['path']),'fixture',facts=hydrate(b.get('facts',{family:b.get('value')}))) for b in case['bundles']}
            value=plugin.reporter(ProjectFactView(root/case.get('view_root','.'),bundles,plugin.required_facts))
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from async_pattern_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':
    main(CHECK,'rerail.async-pattern-components/v1',cases,exercise,install,['async_pattern_components_contract.py'])
