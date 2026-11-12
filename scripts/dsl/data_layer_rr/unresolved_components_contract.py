"""Record complete unresolved-annotation metadata, supplied AST extraction and raw reporting."""
import ast
from dataclasses import asdict,fields
from pathlib import Path
from types import SimpleNamespace
from contract_runner import main
from dto import canonical
from async_pattern_components_contract import hydrate

CHECK='unresolved_forward_refs'
BAD='def work():\n    return get_origin(field_info.annotation)\n'


def cases(_module):
    rows=[dict(kind='metadata',id='metadata')]
    texts=dict(positive=BAD,clean='pass\n',trigger_only='# .annotation\n',syntax='def broken(: # .annotation\n',
        direct='get_origin(field_info.annotation)\n',attribute='typing.get_args(f.annotation)\n',
        model_fields='TypeAdapter(Model.model_fields["x"].annotation)\n',
        safe_string='str(field_info.annotation)\n',safe_none='field_info.annotation is None\n',
        safe_detection='isinstance(field_info.annotation, (str, ForwardRef))\n',
        comparison='field_info.annotation is Other\nfield_info.annotation == Other\n',
        fallback='resolved_hints.get("x", field_info.annotation)\n',
        resolved='def work():\n hints2 = get_type_hints(Model)\n return hints2.get("x", field_info.annotation)\n',
        nested='def outer():\n hints2 = get_type_hints(Model)\n def inner():\n  return get_origin(field_info.annotation)\n return hints2.get("x",field_info.annotation)\n',
        decorated='@decorate(get_origin(field_info.annotation))\ndef work(x=get_args(field_info.annotation)):\n return TypeAdapter(field_info.annotation)\n',
        unicode='def 機能():\n return get_origin(ｆｉｅｌｄ_ｉｎｆｏ.annotation)\n',
        duplicates='get_origin(field_info.annotation)\nget_origin(field_info.annotation)\n',
        class_scope='def outer():\n class Inner:\n  value = get_origin(field_info.annotation)\n',
        lambda_scope='def outer():\n task = lambda: get_origin(field_info.annotation)\n')
    for name,source in texts.items():rows.append(dict(kind='extract',id='extract-'+name,path='impl/a.py',source=source))
    for i,path in enumerate(['api/a.py','tests/a.py','other/a.txt','a/../b.py','/external.py']):rows.append(dict(kind='extract',id=f'extract-path-{i}',path=path,source=BAD))
    for name,source,tree in [('matching',BAD,BAD),('source-clean','pass\n',BAD),('different-tree',BAD,'pass\n'),('missing-tree',BAD,None),('none-clean','pass\n',None),('syntax-in-source','def broken(: # .annotation\n',BAD)]:
        rows.append(dict(kind='source',id='source-'+name,source=source,tree=tree))
    for name,source in [('positive',BAD),('clean','pass\n'),('module','get_origin(field_info.annotation)\n')]:
        rows.append(dict(kind='state',id='state-'+name,source=source,findings=[dict(existing=True)],current='seed',lineno=91,resolved=['seed_var']))
    hit=dict(file='supplied.py',line=4,function='caller_function',problem='caller problem',source='ignored source')
    values=[None,{},'bad',0,[],[None,[],0,'bad'],[{}],[hit],[hit,hit],
        [dict(hit,line=v) for v in [None,True,False,2.5,'3',-2,0,2**100,-2**100]],
        [dict(hit,**{key:value}) for key in ['file','function','problem'] for value in [None,True,2.5,['☃',1],{'a':1},'$NAN','$INF']],
        [dict(hit,file=p) for p in ['a/../b.py','/absolute.py','','//network/a.py','./a//b.py/.','///a/b.py/','//','/','.']]]
    rows.extend(dict(kind='report',id=f'report-{i}',bundles=[dict(path='a.py',value=value)]) for i,value in enumerate(values))
    rows.extend([
        dict(kind='report',id='report-missing',bundles=[dict(path='a.py',facts={})]),
        dict(kind='report',id='report-other-family',bundles=[dict(path='a.py',facts={'other':[hit]})]),
        dict(kind='report',id='report-fallback-order',bundles=[dict(path=p,value=[{}]) for p in ['a/z.py','a-.py','z.py']]),
        dict(kind='report',id='report-order',bundles=[dict(path='a.py',value=[dict(hit,line=n,problem=t) for n,t in [(10,'z'),(2,'a'),(2,'b'),(2,'b'),(-2,'☃')]])]),
    ])
    return rows


def exercise(module,case,scratch):
    scratch=scratch.resolve();root=scratch/'repo';root.mkdir()
    try:
        kind=case['kind']
        if kind=='metadata':
            plugin=module.unresolved_forward_refs_plugin(root);scope=asdict(plugin.file_scope)
            for key in ['directories','explicit_files']:scope[key]=[str(p.relative_to(root)) for p in scope[key]]
            value=dict(constants={n:getattr(module,n) for n in ['TYPE_INSPECTION_FUNCTIONS','RESOLVED_PATTERNS']},extractor=asdict(module.UNRESOLVED_FORWARD_REFS_EXTRACTOR),plugin={n:getattr(plugin,n) for n in ['name','enforcement','description']},scope=scope,
                defaults=asdict(module.AnnotationUsageFinder(Path('caller.py'),['caller line'])),dto=[dict(name=f.name,type=str(f.type)) for f in fields(module.AnnotationUsageFinder)])
        elif kind=='extract':
            from scripts.lint_lib.fact_engine import materialize_source_facts
            spec=module.UNRESOLVED_FORWARD_REFS_EXTRACTOR
            result=materialize_source_facts(path=root/case['path'],source=case['source'],specs=(spec,),extractors=module.unresolved_forward_refs_fact_extractors(root))
            value=dict(facts=result.facts,errors=result.errors)
        elif kind=='source':value=module.analyze_source_content(root/'impl/a.py',case['source'],tree=ast.parse(case['tree']) if case['tree'] is not None else None)
        elif kind=='state':
            path=root/'impl/a.py';path.parent.mkdir();path.write_text(case['source'])
            owner=module.AnnotationUsageFinder(path,['caller-owned source']);owner.findings=case['findings'].copy();owner.current_function=case['current'];owner.current_function_lineno=case['lineno'];owner.resolved_hint_vars=set(case['resolved'])
            result=owner.analyze(ast.parse(case['source']));value=dict(returned=result,state=asdict(owner))
        else:
            family=module.UNRESOLVED_FORWARD_REFS_EXTRACTOR.extractor_id
            bundles={str(i):SimpleNamespace(path=Path(b['path']),facts=hydrate(b.get('facts',{family:b.get('value')}))) for i,b in enumerate(case['bundles'])}
            value=module.unresolved_forward_refs_plugin(root).reporter(SimpleNamespace(root=root,files=bundles))
        return dict(value=canonical(value,scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


def install(backend,module,detectors=False):
    from unresolved_port import install as install_native
    install_native(backend,module)


if __name__=='__main__':main(CHECK,'rerail.unresolved-components/v1',cases,exercise,install,['unresolved_components_contract.py','async_pattern_components_contract.py'])
