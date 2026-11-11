"""Record inspection plugin metadata, registered extraction, and supplied reports."""
from dataclasses import asdict
from pathlib import Path
from types import SimpleNamespace

from contract_runner import main
from dto import canonical
from inspection_port import install

CHECK='no_source_inspection'


def cases(_module):
    result=[]
    def add(name,**values):result.append(dict(id=name,kind='plugin',**values))
    add('descriptor')
    add('registered-extraction',extract='inspect.getsource(f)\nfrom inspect import getsourcelines\n')
    add('excluded-extraction',extract='inspect.getsource(f)\n',path='tests/__pycache__/a.py')
    add('empty')
    add('missing-family',missingFamily=True)
    for name,value in [('null',None),('mapping',{}),('string','text'),('number',0),('empty-list',[])]:
        add('container-'+name,values=value)
    add('malformed-rows',values=[None,False,1,[],{},dict(file_path='a.py',line=4,pattern_name='positive')])
    add('defaults',values=[{}])
    add('fallback-bundle-path',values=[dict(pattern_name='fallback')])
    add('line-types',values=[dict(line=v,pattern_name='line') for v in [None,True,False,1.5,'3',-2,0,10**70,-10**70]])
    add('pattern-types',values=[dict(pattern_name=v) for v in [None,True,2.5,['snowman',1],dict(a=1)]])
    add('path-types',values=[dict(file_path=v) for v in ['',None,True,2.5,'./a//b.py/.','//network/a.py','///a/b.py','a/../b.py']])
    add('duplicates',values=[dict(file_path='a.py',line=4,pattern_name='same')]*2)
    add('ordering',values=[dict(file_path=p,line=l,pattern_name=n) for p,l,n in [
        ('z.py',1,'a'),('a.py',4,'z'),('a.py',4,'a'),('a.py',2,'b')]])
    add('bundle-order',values=[{}],second=[dict(pattern_name='second')])
    add('absolute-bundle',values=[{}],absoluteBundle=True)
    add('outside-bundle',values=[{}],outsideBundle=True)
    return result


def exercise(module,case,scratch):
    root=scratch/'project';root.mkdir()
    plugin=module.no_source_inspection_plugin(root)
    metadata=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description,
        scope=asdict(plugin.file_scope),required_facts=[asdict(spec) for spec in plugin.required_facts])
    result=dict(metadata=canonical(metadata,scratch))
    if 'extract' in case:
        registry=module.no_source_inspection_fact_extractors(root)
        result['registry']=list(registry)
        result['extracted']=canonical(registry[plugin.required_facts[0].extractor_id](root/case.get('path','tests/a.py'),case['extract']),scratch)
    if case['id']!='descriptor':
        path=Path('relative/a.py')
        if case.get('absoluteBundle'):path=root/'absolute/a.py'
        if case.get('outsideBundle'):path=scratch/'outside/a.py'
        family=plugin.required_facts[0].extractor_id
        bundles={path:SimpleNamespace(path=path,facts={} if case.get('missingFamily') else {family:case.get('values',[])})}
        if 'second' in case:bundles[Path('second.py')]=SimpleNamespace(path=Path('second.py'),facts={family:case['second']})
        result['findings']=canonical(plugin.reporter(SimpleNamespace(root=root,files=bundles)),scratch)
    return result


if __name__=='__main__':
    main(CHECK,'rerail.inspection-plugin-contract/v1',cases,exercise,install,
         ['inspection_plugin_contract.py','inspection_port.py','fact_transport.py','text_cli_port.py'])
