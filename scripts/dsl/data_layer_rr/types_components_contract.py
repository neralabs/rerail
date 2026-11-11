"""Record type-safety metadata, registered collectors and caller-supplied facts."""
import ast
from dataclasses import fields
from pathlib import Path
from contract_runner import main
from dto import canonical
from types_cli_contract import install,BAD

CHECK='type_safety_patterns'


def cases(_module):
    rows=[dict(kind='metadata',id='metadata-'+str(i),directories=dirs) for i,dirs in enumerate([[],['api'],['impl','public'],['data_layer','_internal','l1000_a','l１２３４_a','l1234_']])]
    for name,source in [('positive',BAD),('clean','pass\n'),('invalid','def broken(: # str | Expression\n'),('async','async def where(expression: str):\n pass\n'),('nested','def f():\n def select(columns: str):\n  pass\n'),('body-window','def select():\n x: str | Expression\n'),('unicode','def ｏｒｄｅｒ＿ｂｙ(column: str):\n pass\n')]:
        rows.append(dict(kind='extract',id='extract-'+name,source=source))
    for name,source in [('positive',BAD),('clean','pass\n'),('async','async def where(expression: str):\n pass\n'),('nested','def order_by(column: str):\n def select(columns: str):\n  pass\n'),('unicode','def ｏｒｄｅｒ＿ｂｙ(column: str):\n pass\n')]:rows.append(dict(kind='state',id='state-'+name,source=source))
    for kind in ['start','callback']:
        for name,source in [('order',BAD),('where','def where(expression: str):\n pass\n'),('select','def select(columns: str):\n pass\n'),('async','async def group_by(columns: str):\n pass\n'),('clean','def other():\n pass\n')]:rows.append(dict(kind=kind,id=kind+'-'+name,source=source))
    for name,paths in [('order',['z.py','a/z.py','a-.py','a.py']),('duplicates',['a.py','a.py']),('missing',['missing.py']),('scope',['tests/a.py','test/a.py','a.py','.py','a.pyi']),('binary',['a.py'])]:rows.append(dict(kind='batch',id='batch-'+name,paths=paths,binary=name=='binary'))
    hit=dict(category='STR_UNION_EXPRESSION',file_path='a.py',line_number=3,column=2,code_snippet='snippet',suggestion='suggestion')
    for i,number in enumerate([1,True,False,-1,2**100,None,2.5,'3']):rows.append(dict(kind='project',id='project-'+str(i),fact=dict(hit,line_number=number,column=number)))
    for i,value in enumerate([None,{},hit,dict(hit,category='UNKNOWN'),*[dict(hit,**{key:value}) for key in hit for value in [None,True,2.5,[],{}]],dict(hit,file_path='./a//./b.py/.')]):rows.append(dict(kind='load',id='load-'+str(i),fact=value))
    values=[None,{},'bad',0,[],[None,0,'bad',[]],[{}],[hit],[hit,hit],
        [dict(hit,line_number=v) for v in [None,True,False,2.5,'3',-2,0,2**100,-2**100]],
        [dict(hit,category=v,suggestion=v,file_path=v) for v in [None,True,2.5,['☃',1],{'a':1}]],
        [dict(hit,file_path=p) for p in ['a/../b.py','/absolute.py','','//network/a.py','./a//./b.py/.','///a/b.py/','//','/','.']]]
    rows.extend(dict(kind='report',id='report-'+str(i),bundles=[dict(path='bundle.py',value=value)]) for i,value in enumerate(values))
    rows.extend([dict(kind='report',id='missing-namespace',bundles=[dict(path='a.py',facts={})]),dict(kind='report',id='foreign-namespace',bundles=[dict(path='a.py',facts={'other':[hit]})]),dict(kind='report',id='stable-message-order',bundles=[dict(path='bundle.py',value=[dict(hit,suggestion=s) for s in ['b','a','a']])])])
    return rows


def exercise(module,case,scratch):
    from scripts.lint_lib.fact_engine import materialize_source_facts
    from scripts.lint_lib.linter_plugin import PerFileFactBundle,ProjectFactView
    root=scratch/'repo';root.mkdir()
    for name in case.get('directories',[]):(root/name).mkdir()
    try:
        family=module.TYPE_SAFETY_PATTERNS_EXTRACTOR.extractor_id
        if case['kind']=='metadata':
            plugin=module.type_safety_patterns_plugin(root);value={f.name:getattr(plugin,f.name) for f in fields(plugin) if f.name!='reporter'};value.update(registry=list(module.type_safety_patterns_fact_extractors(root)),categories=[(c.name,c.value) for c in module.IssueCategory],allowlist=module.ALLOWLIST,triggers=module.PATTERN_SOURCE_TRIGGERS,signatureTriggers=module.SIGNATURE_SOURCE_TRIGGERS)
        elif case['kind']=='extract':
            path=root/'impl/a.py'
            bundle=module._rerail_types_extract(path,case['source'],root) if hasattr(module,'_rerail_types_extract') else materialize_source_facts(path=path,source=case['source'],specs=(module.TYPE_SAFETY_PATTERNS_EXTRACTOR,),extractors=module.type_safety_patterns_fact_extractors(root))
            value=dict(facts=bundle.facts,errors=bundle.errors)
        elif case['kind']=='state':
            factory=module.type_safety_patterns_fact_extractors(root)[family];created=[]
            def capture(path,source):
                collector=factory(path,source);created.append(collector);return collector
            bundle=materialize_source_facts(path=root/'impl/a.py',source=case['source'],specs=(module.TYPE_SAFETY_PATTERNS_EXTRACTOR,),extractors={family:capture})
            value=dict(facts=bundle.facts,lines=created[0].lines,issues=created[0].issues)
        elif case['kind'] in {'start','callback'}:
            collector=module.TypeSafetyPatternsCollector(root/'impl/a.py',root);collector.start_file(root/'impl/a.py',case['source'],None)
            if case['kind']=='callback':
                node=next(n for n in ast.walk(ast.parse(case['source'])) if isinstance(n,(ast.FunctionDef,ast.AsyncFunctionDef)));getattr(collector,'on_'+type(node).__name__)(node,None)
            value=dict(issues=collector.issues,lines=collector.lines)
        elif case['kind']=='batch':
            for path in set(case['paths']):
                if path!='missing.py':
                    p=root/path;p.parent.mkdir(parents=True,exist_ok=True)
                    if case.get('binary'):p.write_bytes(b'\xff')
                    else:p.write_text(BAD)
            values=module._issues_for_files(root,tuple(root/path for path in case['paths']));groups={}
            for issue in values:groups.setdefault(issue.file_path,[]).append(issue)
            value=[issue for path in sorted(groups) for issue in groups[path]]
        elif case['kind']=='project':
            fact=case['fact'];value=module._issue_to_fact(module.TypeSafetyIssue(module.IssueCategory[fact['category']],Path(fact['file_path']),fact['line_number'],fact['column'],fact['code_snippet'],fact['suggestion']))
        elif case['kind']=='load':value=module._issue_from_fact(case['fact'])
        else:
            plugin=module.type_safety_patterns_plugin(root);bundles={str(i):PerFileFactBundle(Path(b['path']),'fixture',facts=b.get('facts',{family:b.get('value')})) for i,b in enumerate(case['bundles'])};value=plugin.reporter(ProjectFactView(root,bundles,plugin.required_facts))
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


if __name__=='__main__':
    main(CHECK,'rerail.type-safety-components/v1',cases,exercise,install,['types_components_contract.py','types_cli_contract.py','types_port.py'])
