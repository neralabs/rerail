"""Record complete plugin metadata and untrusted supplied-fact reporting."""
from dataclasses import asdict
from pathlib import Path
from types import SimpleNamespace
from contract_runner import main
from dto import canonical
from replay import write_fixture

CHECK='plan_scope_side_effects'
HIT=dict(code='PSE-020',file_path='impl/a.py',line_number=7,message='caller finding',severity='error')


def cases(_module):
    rows=[]
    for i,dirs in enumerate([[],['api'],['impl','data_layer','_internal','l9999_z','l1000_a','other'],
            ['l١٢٣٤_x','l１２３４_x','l1234_','l1234_\nx','l1234_x\nmore']]):
        rows.append(dict(kind='metadata',id='production-'+str(i),files=[dict(path=n,directory=True)for n in dirs]))
    rows.extend(dict(kind='metadata',id='explicit-'+str(i),explicit=values)for i,values in enumerate([[],['a.py','a.py','../other.py'],['/elsewhere/a.py','tests/a.py']]))
    data=[('empty',[]),('positive',[HIT]),('duplicates',[HIT,HIT]),('defaults',[{}]),('skip-rows',[None,False,3,'bad',[],HIT]),
        ('dict-container',dict(a=HIT)),('none-container',None),('string-container','bad'),('tuple-container',[]),
        ('order',[dict(HIT,file_path=p,line_number=l,code=c)for p,l,c in [('z.py',1,'A'),('a.py',10,'A'),('a.py',-1,'B'),('a.py',-1,'A')]]),
        ('cross-bundle',[HIT]),('missing-family',[]),('foreign-family',[])]
    for name,value in data:rows.append(dict(kind='report',id=name,bundles=[dict(path='fallback.py',value=value)]))
    rows += [dict(kind='report',id='cross-bundle-stable',bundles=[dict(path=p,value=[dict(HIT,message=m)])for p,m in [('z.py','first'),('a.py','second'),('a.py','third')]])]
    for label,value in [('none',None),('true',True),('false',False),('int',4),('big',10**80),('negative',-10**80),
            ('float',1.5),('zero',-0.0),('small',1e-10),('large',1e20),('text','é\n"quoted\''),
            ('list',[1,None,True,'x']),('dict',dict(a=[False,dict(b='é')]))]:
        for key in ['code','file_path','message','line_number','severity']:
            rows.append(dict(kind='report',id=key+'-'+label,bundles=[dict(path='fallback.py',value=[dict(HIT,**{key:value})])]))
    for i,path in enumerate(['','.', './a//./b.py/.','a/../b.py','/absolute.py','//network/a.py','///a/b.py/']):
        rows.append(dict(kind='report',id='path-'+str(i),bundles=[dict(path='fallback.py',value=[dict(HIT,file_path=path)])]))
    rows += [dict(kind='report',id='bool-and-large-line-order',bundles=[dict(path='a.py',value=[dict(HIT,line_number=n)for n in [True,False,10**80,-10**80,-2,0,2]])]),
        dict(kind='report',id='path-parts-order',bundles=[dict(path=p,value=[dict(HIT,file_path=p)])for p in ['a/z.py','a-.py','/z.py','//a.py','.','/','//']]),
        dict(kind='report',id='tuple-field',bundles=[dict(path='a.py',value=[HIT])]),
        dict(kind='report',id='float-specials',bundles=[dict(path='a.py',value=[HIT])])]
    return rows


def exercise(module,case,scratch):
    root=scratch.resolve()/'repo';root.mkdir();write_fixture(root,case.get('files',[]))
    try:
        if case['kind']=='metadata':
            scope=None
            if 'explicit'in case:
                scope=module.FileScopeSpec(explicit_files=tuple(Path(p)for p in case['explicit']),include_tests=True)
                plugin=module._plan_scope_side_effects_plugin(root,scope)
            else:plugin=module.plan_scope_side_effects_plugin(root)
            return dict(value=canonical(dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description,
                scope=asdict(plugin.file_scope),required=[asdict(s)for s in plugin.required_facts],same_scope=scope is None or plugin.file_scope is scope),scratch.resolve()))
        family=module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR.extractor_id;bundles={}
        for i,b in enumerate(case['bundles']):
            value=b['value']
            if case['id']=='tuple-container':value=tuple(value)
            if case['id']=='tuple-field':value=[dict(HIT,message=(1,'x',(False,)))]
            if case['id']=='float-specials':value=[dict(HIT,message=[float('nan'),float('inf'),-float('inf'),-0.0])]
            facts={family:value}
            if case['id']=='missing-family':facts={}
            if case['id']=='foreign-family':facts={'foreign':[HIT]}
            bundles[str(i)]=SimpleNamespace(path=Path(b['path']),facts=facts)
        plugin=module._plan_scope_side_effects_plugin(root,module.FileScopeSpec())
        return dict(value=canonical(plugin.reporter(SimpleNamespace(root=root,files=bundles)),scratch.resolve()))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch.resolve())))


def install(backend,module,detectors=False):
    from side_effects_plugin_port import install as plugin
    plugin(backend,module)


if __name__=='__main__':main(CHECK,'rerail.side-effects-plugin/v1',cases,exercise,install,['side_effects_plugin_contract.py','side_effects_plugin_port.py','value_columns.py'])
