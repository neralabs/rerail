"""Record ForwardRef metadata, registered collectors, helpers and supplied facts."""
import ast
from dataclasses import fields
from pathlib import Path
from contract_runner import main
from dto import canonical
from forwardref_scope_contract import install
from replay import write_fixture

CHECK='forwardref'
BAD='annotation = field.annotation\ncallable(annotation)\n'


def cases(_module):
    rows=[dict(kind='metadata',id='metadata-'+str(i),directories=names) for i,names in enumerate([[],['api'],['impl','public'],['data_layer','_internal','l1000_a']])]
    for name,source in [('positive',BAD),('clean','pass\n'),('invalid','def broken(: # ForwardRef\n'),
        ('reset',BAD+'isinstance(annotation, ForwardRef)\n'+BAD),
        ('guarded','annotation = field.annotation\nisinstance(annotation, ForwardRef)\ncallable(annotation)\n'),
        ('sync','annotation = field.annotation\ndef f():\n callable(annotation)\ncallable(annotation)\n'),
        ('async','annotation = field.annotation\nasync def f():\n callable(annotation)\n'),
        ('unicode','ａｎｎｏｔａｔｉｏｎ = field.ａｎｎｏｔａｔｉｏｎ\ncallable(ａｎｎｏｔａｔｉｏｎ)\n')]:
        rows.append(dict(kind='extract',id='extract-'+name,source=source))
    for i,text in enumerate(['ForwardRef','typing.ForwardRef','other.ForwardRef','ForwardRef.other','(str,ForwardRef)',
        '((str,),(typing.ForwardRef,))','[ForwardRef]','str | ForwardRef','"ForwardRef"','Optional[ForwardRef]','str','None']):
        rows.append(dict(kind='helper',id='contains-'+str(i),annotation=text))
    for i,(path,root) in enumerate([('api/a.py','.'),('api/a.py','api'),('api/a.py','api/a.py'),('api/a.py','other'),
        ('/project/api/a.py','/project'),('/project/api/a.py','/'),('//server/a.py','//'),('a/../b.py','a'),('a.py','/project')]):
        rows.append(dict(kind='project',id='project-'+str(i),path=path,root=root,line=2,col=3))
    for i,line in enumerate([True,False,-1,2**100,None,2.5]):rows.append(dict(kind='project',id='project-line-'+str(i),path='a.py',root='.',line=line,col=line))
    hit=dict(file='supplied.py',line=4,col=3,message='caller supplied',code='CUSTOM')
    values=[None,{},'bad',0,[],[None,0,'bad',[]],[{}],[hit],[hit,hit],
        [dict(hit,line=v) for v in [None,True,False,2.5,'3',-2,0,2**100,-2**100]],
        [dict(hit,message=v,code=v,file=v) for v in [None,True,2.5,['☃',1],{'a':1}]],
        [dict(hit,file=p) for p in ['a/../b.py','/absolute.py','','//network/a.py','./a//./b.py/.','///a/b.py/','//','/','.']],
        [dict(hit,message={'value':'$NAN','items':['$INF',-0.0,'$NEG_INF']})],
        [dict(hit,line='$NAN'),hit,dict(hit,line='$INF')]]
    rows.extend(dict(kind='report',id='report-'+str(i),bundles=[dict(path='a.py',value=v)]) for i,v in enumerate(values))
    rows.extend([dict(kind='report',id='missing-namespace',bundles=[dict(path='a.py',facts={})]),
        dict(kind='report',id='foreign-namespace',bundles=[dict(path='a.py',facts={'other':[hit]})]),
        dict(kind='report',id='fallback-order',bundles=[dict(path=p,value=[{}]) for p in ['a/z.py','a-.py','z.py']]),
        dict(kind='report',id='bundle-key',bundles=[dict(key='ignored.py',path='actual.py',value=[{}])]),
        dict(kind='report',id='stable-code-order',bundles=[dict(path='a.py',value=[dict(hit,line=l,code=c,message=m) for l,c,m in [(4,'B','first'),(2,'Z','second'),(4,'A','third'),(4,'A','fourth')]])])])
    return rows


def exercise(module,case,scratch):
    from scripts.lint_lib.fact_engine import materialize_source_facts
    from scripts.lint_lib.linter_plugin import PerFileFactBundle,ProjectFactView
    root=scratch/'repo';root.mkdir()
    for name in case.get('directories',[]):(root/name).mkdir()
    def hydrate(value):
        if isinstance(value,str):return {'$NAN':float('nan'),'$INF':float('inf'),'$NEG_INF':-float('inf')}.get(value,value)
        if isinstance(value,list):return [hydrate(v) for v in value]
        if isinstance(value,dict):return {k:hydrate(v) for k,v in value.items()}
        return value
    try:
        family=module.FORWARDREF_EXTRACTOR.extractor_id
        if case['kind']=='metadata':
            plugin=module.forwardref_plugin(root)
            value={f.name:getattr(plugin,f.name) for f in fields(plugin) if f.name!='reporter'};value['registry']=list(module.forwardref_fact_extractors(root))
        elif case['kind']=='extract':
            bundle=materialize_source_facts(path=root/'api/a.py',source=case['source'],specs=(module.FORWARDREF_EXTRACTOR,),extractors=module.forwardref_fact_extractors(root))
            value=dict(facts=bundle.facts,errors=bundle.errors)
        elif case['kind']=='helper':value=module.ForwardRefCollector(root/'api/a.py')._contains_forwardref(ast.parse(case['annotation'],mode='eval').body)
        elif case['kind']=='project':value=module._issue_to_fact(module.LintIssue(Path(case['path']),case['line'],case['col'],'message','CODE'),Path(case['root']))
        else:
            plugin=module.forwardref_plugin(root)
            bundles={str(b.get('key',i)):PerFileFactBundle(Path(b['path']),'fixture',facts=hydrate(b.get('facts',{family:b.get('value')}))) for i,b in enumerate(case['bundles'])}
            value=plugin.reporter(ProjectFactView(root,bundles,plugin.required_facts))
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


if __name__=='__main__':
    main(CHECK,'rerail.forwardref-components/v1',cases,exercise,install,
        ['forwardref_components_contract.py','forwardref_scope_contract.py','suite.py'])
