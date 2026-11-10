"""Record complete dynamic-loading plugin, extraction, and supplied-fact contracts."""
from contextlib import ExitStack,redirect_stdout,redirect_stderr
from dataclasses import fields
import io
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
from dto import canonical
from replay import write_fixture

CHECK='dynamic_module_loading'


def cases(_module):
    rows=[]
    for i,names in enumerate([[],['api','impl'],['impl','data_layer','_internal','l9000_last','l1000_first'],['l١٢٣٤_layer','l1234_a\nb','l1234_\nb'],['api'],['impl']]):
        rows.append(dict(kind='metadata',id='metadata-'+str(i),files=[dict(path=n,directory=True) for n in names]))
    for name,content in [('positive','importlib.import_module("os")\n__import__("x")\n'),('primary','def __getattr__(x): return import_module(x)\n'),('safe','import os\n'),('guarded','if TYPE_CHECKING:\n import_module("os")\n'),('quoted','"""\n__import__("os")\n"""\n')]:
        for allowed in [[],['api/a.py']]:rows.append(dict(kind='extract',id='extract-'+name+'-'+str(bool(allowed)),content=content,allowed=allowed))
    for i,line in enumerate([0,True,False,-1,2**100,None,2.5]):rows.append(dict(kind='project',id='project-'+str(i),line=line))
    hit=dict(file_path='supplied.py',line=4,line_content='not used by reporter',description='caller supplied description')
    values=[None,{},'wrong-container',0,[],[None,0,'wrong-row',[]],[{}],[hit],[hit,hit],
            [dict(hit,line=v) for v in [None,True,False,2.5,'3',-2,0,2**100,-2**100]],
            [dict(hit,description=v,file_path=v) for v in [None,True,2.5,['☃',1],{'a':1}]],
            [dict(hit,file_path=p) for p in ['a/../b.py','/absolute.py','','//network/a.py','./a//./b.py/.','///a/b.py/','//','/','.']],
            [dict(hit,description={'nan':float('nan'),'items':[float('inf'),-0.0,float('-inf')]})]]
    # Tag non-finite values so the on-disk fixture remains strict JSON.
    values[-1]=[dict(hit,description={'nan':'$NAN','items':['$INF',-0.0,'$NEG_INF']})]
    for i,value in enumerate(values):rows.append(dict(kind='report',id='report-'+str(i),bundles=[dict(path='a.py',value=value)]))
    rows.extend([
        dict(kind='report',id='bundle-order',bundles=[dict(path=p,value=[dict(hit,line=l,description=m)]) for p,l,m in [('z.py',2,'z'),('a.py',1,'a'),('a.py',1,'duplicate')]]),
        dict(kind='report',id='fallback-path-order',bundles=[dict(path=p,value=[{}]) for p in ['a/z.py','a-.py','z.py']]),
        dict(kind='report',id='missing-facts',bundles=[dict(path='a.py',facts={})]),
        dict(kind='report',id='foreign-facts',bundles=[dict(path='a.py',facts={'other':[hit]})]),
        dict(kind='report',id='invalid-signature-short-circuit',bundles=None,verifyResult=False),
        dict(kind='report',id='verification-streams',bundles=[dict(path='a.py',value=[hit])],verifyResult=False,verifyStdout='verify output\n',verifyStderr='verify error\n'),
    ])
    for s in ['', 'UNSIGNED','NEEDS_SIGNING','!!!!']:
        rows.append(dict(kind='report',id='real-signature-'+s,bundles=[dict(path='a.py',value=[hit])],signature=s,verifyNative=True))
    return rows


def exercise(module,case,temporary):
    from scripts.lint_lib.linter_plugin import PerFileFactBundle,ProjectFactView
    root=temporary/'repo';root.mkdir();write_fixture(root,case.get('files',[]));out,err=io.StringIO(),io.StringIO();events=[]
    def hydrate(v):
        if isinstance(v,str):return {'$NAN':float('nan'),'$INF':float('inf'),'$NEG_INF':-float('inf')}.get(v,v)
        if isinstance(v,list):return [hydrate(x) for x in v]
        if isinstance(v,dict):return {k:hydrate(x) for k,x in v.items()}
        return v
    def verify(*args,**kwargs):
        events.append(dict(name='verify',args=canonical(args,temporary),kwargs=canonical(kwargs,temporary)))
        print(case.get('verifyStdout',''),end='');err.write(case.get('verifyStderr',''));return case.get('verifyResult',True)
    with ExitStack() as stack:
        if not case.get('verifyNative'):stack.enter_context(patch.object(module,'verify_allowlist',verify))
        if 'signature' in case:stack.enter_context(patch.object(module,'ALLOWLIST_SIGNATURE',case['signature']))
        if 'allowed' in case:stack.enter_context(patch.object(module,'ALLOWLISTED_FILES',case['allowed']))
        with redirect_stdout(out),redirect_stderr(err):
            try:
                if case['kind']=='metadata':
                    plugin=module.dynamic_module_loading_plugin(root)
                    value={f.name:getattr(plugin,f.name) for f in fields(plugin) if f.name!='reporter'}
                    value['registry']=list(module.dynamic_module_loading_fact_extractors(root))
                elif case['kind']=='extract':
                    registry=module.dynamic_module_loading_fact_extractors(root)
                    value=registry[module.DYNAMIC_MODULE_LOADING_EXTRACTOR.extractor_id](root/'api/a.py',case['content'])
                elif case['kind']=='project':value=module._violation_to_fact((Path('a/./b.py'),case['line'],'a snippet','a description'))
                else:
                    plugin=module.dynamic_module_loading_plugin(root)
                    bundles=None if case['bundles'] is None else {str(i):PerFileFactBundle(Path(b['path']),'fixture',facts=hydrate(b.get('facts',{module.DYNAMIC_MODULE_LOADING_EXTRACTOR.extractor_id:b.get('value')}))) for i,b in enumerate(case['bundles'])}
                    value=plugin.reporter(ProjectFactView(root,bundles,plugin.required_facts))
                result=dict(value=canonical(value,temporary))
            except BaseException as error:result=dict(error=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=canonical(error.args,temporary)))
    return dict(result,stdout=out.getvalue(),stderr=err.getvalue(),events=events)


if __name__=='__main__':
    from contract_runner import main
    from dynamic_port import install
    main(CHECK,'rerail.dynamic-plugin-contract/v1',cases,exercise,install,['dynamic_plugin_contract.py','dynamic_port.py','dynamic_cli_port.py','dynamic_plugin_port.py','fact_transport.py','python_value_transport.py'],guard_modules=['scripts.lint_lib.discovery','scripts.lint_lib.signing'])
