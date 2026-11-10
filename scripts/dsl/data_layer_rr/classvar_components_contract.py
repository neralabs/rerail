"""Record ClassVar metadata, framework extraction, supplied facts and projections."""
from dataclasses import fields
from pathlib import Path
from dto import canonical
from replay import write_fixture

CHECK='classvar_check'


def cases(_module):
    rows=[dict(kind='metadata',id='metadata-'+str(i),files=[dict(path=p,directory=True) for p in names])
          for i,names in enumerate([[],['api'],['impl','data_layer','_internal','l9000_z','l1000_a'],['l١٢٣٤_x','l1234_a\nb','l1234_\nb','l1234_']])]
    for name,text in [('positive','get_origin(x) is ClassVar\n'),('clean','pass\n'),('trigger-only','# ClassVar\n'),
        ('invalid','def (: # get_origin ClassVar\n'),('helper','def is_classvar_annotation(x):\n return get_origin(x) is ClassVar\n'),
        ('async-helper','async def is_classvar_annotation(x):\n return get_origin(x) is ClassVar\n'),
        ('order','@d(get_origin(x) is ClassVar)\ndef f(x=get_origin(x) is ClassVar):\n return get_origin(x) is ClassVar\n'),
        ('chain','get_origin(x) is ClassVar == ClassVar\n')]:
        rows.append(dict(kind='extract',id='extract-'+name,source=text))
    for i,line in enumerate([0,True,False,-1,2**100,None,2.5]):
        rows.append(dict(kind='project',id='project-'+str(i),line=line))
    hit=dict(file='supplied.py',line=4,col=3,message='caller supplied',code='CUSTOM')
    values=[None,{},'wrong',0,[],[None,0,'wrong',[]],[{}],[hit],[hit,hit],
        [dict(hit,line=v) for v in [None,True,False,2.5,'3',-2,0,2**100,-2**100]],
        [dict(hit,message=v,code=v,file=v) for v in [None,True,2.5,['☃',1],{'a':1}]],
        [dict(hit,file=p) for p in ['a/../b.py','/absolute.py','','//network/a.py','./a//./b.py/.','///a/b.py/','//','/','.']],
        [dict(hit,message={'value':'$NAN','items':['$INF',-0.0,'$NEG_INF']})]]
    rows += [dict(kind='report',id='report-'+str(i),bundles=[dict(path='a.py',value=value)]) for i,value in enumerate(values)]
    rows += [dict(kind='report',id='missing-namespace',bundles=[dict(path='a.py',facts={})]),
             dict(kind='report',id='foreign-namespace',bundles=[dict(path='a.py',facts={'other':[hit]})]),
             dict(kind='report',id='fallback-order',bundles=[dict(path=p,value=[{}]) for p in ['a/z.py','a-.py','z.py']]),
             dict(kind='report',id='line-message-order',bundles=[dict(path='a.py',value=[dict(hit,line=l,message=m) for l,m in [(4,'b'),(2,'z'),(4,'a'),(4,'a')]])])]
    return rows


def exercise(module,case,temporary):
    from scripts.lint_lib.fact_engine import materialize_source_facts
    from scripts.lint_lib.linter_plugin import PerFileFactBundle,ProjectFactView
    root=temporary/'repo';root.mkdir();write_fixture(root,case.get('files',[]))
    def hydrate(v):
        if isinstance(v,str):return {'$NAN':float('nan'),'$INF':float('inf'),'$NEG_INF':-float('inf')}.get(v,v)
        if isinstance(v,list):return [hydrate(x) for x in v]
        if isinstance(v,dict):return {k:hydrate(x) for k,x in v.items()}
        return v
    try:
        if case['kind']=='metadata':
            plugin=module.classvar_check_plugin(root)
            value={f.name:getattr(plugin,f.name) for f in fields(plugin) if f.name!='reporter'}
            value['registry']=list(module.classvar_check_fact_extractors(root))
        elif case['kind']=='extract':
            bundle=materialize_source_facts(path=root/'api/a.py',source=case['source'],specs=(module.CLASSVAR_CHECK_EXTRACTOR,),extractors=module.classvar_check_fact_extractors(root))
            value=bundle.facts
        elif case['kind']=='project':
            value=module._issue_to_fact(module.LintIssue(Path('a/./b.py'),case['line'],3,'message','CODE'))
        else:
            plugin=module.classvar_check_plugin(root)
            bundles={str(i):PerFileFactBundle(Path(b['path']),'fixture',facts=hydrate(b.get('facts',{module.CLASSVAR_CHECK_EXTRACTOR.extractor_id:b.get('value')}))) for i,b in enumerate(case['bundles'])}
            value=plugin.reporter(ProjectFactView(root,bundles,plugin.required_facts))
        return dict(value=canonical(value,temporary))
    except BaseException as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,temporary)))


def install(backend,module,detectors=False):
    from classvar_port import install as native_install
    native_install(backend,module,detectors=detectors)


if __name__=='__main__':
    from contract_runner import main
    main(CHECK,'rerail.classvar-components/v1',cases,exercise,install,
         ['classvar_components_contract.py','classvar_port.py','ast_transport.py','python_value_transport.py','fact_transport.py'])
