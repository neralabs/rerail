"""Record executor configuration, registered extraction and supplied-fact reporting."""
from dataclasses import fields
from pathlib import Path
from contract_runner import main
from dto import canonical
from executor_cli_contract import CHECK,BAD,install


def cases(_module):
    rows=[dict(kind='metadata',id='metadata')]
    for name,source in [('positive',BAD),('clean','pass\n'),('suppressed',BAD.rstrip()+' # noqa: async-executor\n'),('invalid','run_in_executor(\n'),('unicode',BAD.rstrip()+' # '+'☃'*100+'\n'),('duplicate',BAD+BAD),('line-separators','run_in_executor()\u2028run_in_executor()\n')]:rows.append(dict(kind='extract',id='extract-'+name,source=source))
    hit=dict(line=2,source='fixture')
    values=[None,{},'bad',0,[],[None,0,'bad',[]],[{}],[hit],[hit,hit],
        [dict(hit,line=v) for v in [None,True,False,2.5,'3',-2,0,2**100,-2**100]],
        [dict(hit,source=v) for v in [None,True,2.5,['x'],{'a':1},'☃'*100]],
        [dict(hit,line_number=7)], [dict(line_number=7,source='foreign line field')]]
    rows.extend(dict(kind='report',id='report-'+str(i),bundles=[dict(key='impl/sqlite/a.py',path='foreign.py',value=v)]) for i,v in enumerate(values))
    for i,key in enumerate(['impl/common/a.py','impl/sqlite/native_async_wrapper.py','outside.py','impl/postgres/a.py','prefix-impl/sqlite/a.py','impl/sqlite/a.pyi','impl/sqlite/.py','impl/sqlite/tests/a.py']):rows.append(dict(kind='report',id='scope-'+str(i),bundles=[dict(key=key,path='impl/sqlite/ignored.py',value=[hit])]))
    rows.extend([dict(kind='report',id='path-order',bundles=[dict(key=p,path='ignored.py',value=[hit]) for p in ['impl/sqlite/z.py','impl/sqlite/a/z.py','impl/sqlite/a-.py','impl/sqlite/a.py']]),
        dict(kind='report',id='missing-namespace',bundles=[dict(key='impl/sqlite/a.py',path='a.py',facts={})]),
        dict(kind='report',id='foreign-namespace',bundles=[dict(key='impl/sqlite/a.py',path='a.py',facts={'other':[hit]})])])
    return rows


def exercise(module,case,scratch):
    from scripts.lint_lib.fact_engine import materialize_source_facts
    from scripts.lint_lib.linter_plugin import PerFileFactBundle,ProjectFactView
    root=scratch/'repo';root.mkdir();family=module.ASYNC_EXECUTOR_EXTRACTOR.extractor_id
    try:
        if case['kind']=='metadata':
            plugin=module.async_executor_plugin(root);value={f.name:getattr(plugin,f.name) for f in fields(plugin) if f.name!='reporter'};value.update(registry=list(module.async_executor_fact_extractors(root)),strict=module.STRICT_DIRECTORIES,allowed=module.ALLOWED_PATTERNS,patterns=[(p.pattern,p.flags) for p in [module.EXECUTOR_PATTERN,module.NOQA_PATTERN]])
        elif case['kind']=='extract':
            bundle=module._rerail_executor_extract(root/'impl/sqlite/a.py',case['source']) if hasattr(module,'_rerail_executor_extract') else materialize_source_facts(path=root/'impl/sqlite/a.py',source=case['source'],specs=(module.ASYNC_EXECUTOR_EXTRACTOR,),extractors=module.async_executor_fact_extractors(root))
            value=dict(facts=bundle.facts,errors=bundle.errors,direct=module._extract_async_executor_facts(root/'impl/sqlite/a.py',case['source']))
        else:
            plugin=module.async_executor_plugin(root);bundles={Path(b['key']):PerFileFactBundle(Path(b['path']),'fixture',facts=b.get('facts',{family:b.get('value')})) for b in case['bundles']};value=plugin.reporter(ProjectFactView(root,bundles,plugin.required_facts))
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


if __name__=='__main__':
    main(CHECK,'rerail.executor-components/v1',cases,exercise,install,['executor_components_contract.py','executor_cli_contract.py'])
