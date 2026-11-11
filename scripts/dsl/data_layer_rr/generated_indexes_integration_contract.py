"""Record generated-index plugin, helper, CLI, and production contracts from source."""
import argparse
import ast
import contextlib
from dataclasses import asdict
import importlib
import io
from pathlib import Path
import sys
from types import SimpleNamespace
from unittest.mock import patch
from contract_runner import main
from dto import canonical
from generated_indexes_contract import CHECK,OWNER
from generated_indexes_port import install
from first_ten_production_contract import source_case,normalize
from replay import write_fixture

BAD='def build():\n return Index()\n'


def cases(_module):
    result=[dict(id='metadata',kind='metadata'),dict(id='custom-scope',kind='metadata',custom=True),
            dict(id='registered-plugin',kind='registration')]
    for name,text in [('positive',BAD),('clean','pass\n'),('trigger-only','# Index(\n'),('invalid','Index(\n'),
                      ('helper','def _generated_index(*, source: IndexSource):\n return Index(source=source)\n'),
                      ('order','def _generated_index(*, source: str=MODEL_DECLARED):\n Index()\n'),
                      ('aliases','from l1000_schema.schema_def import Index as Idx\nIdx()\n# Index(\n')]:
        result.append(dict(id='extract-'+name,kind='extract',source=text))
    hit=dict(line=4,message='supplied finding')
    values=[None,{},'bad',0,[],[None,1,'bad',[]],[{}],[hit],[hit,hit],
            [dict(line=x,message='line') for x in [True,False,0,-1,2**100,-2**100,1.5,'3',None]],
            [dict(line=3,message=x) for x in [None,False,1,1.5,[],{},'']],
            [dict(hit,file_path='ignored.py',code='IGNORED')],
            [dict(line=l,message=m) for l,m in [(9,'b'),(1,'z'),(9,'a'),(9,'a')]]]
    result.extend(dict(id='report-'+str(i),kind='report',bundles=[dict(key='a.py',path='different.py',value=v)]) for i,v in enumerate(values))
    result.extend([dict(id='report-family-missing',kind='report',bundles=[dict(key='a.py',facts={})]),
        dict(id='report-bundle-order',kind='report',bundles=[dict(key=p,value=[dict(line=2,message=p)]) for p in ['z.py','a/z.py','a-.py','a.py']]),
        dict(id='report-absolute',kind='report',bundles=[dict(key='$ROOT/absolute.py',value=[hit])])])
    numbers=['0','-2','+03','  4  ','1_000','١٢','１２','-0000',str(2**100),str(-2**100),'x','1.5','1__0','', '1\x1c', '9'*4301]
    for i,n in enumerate(numbers):result.append(dict(id='parse-'+str(i),kind='parse',value='a:b.py:'+n+': message: detail'))
    for i,v in enumerate(['bad','a: message','a:x: message','a.py:1:','a: b.py:1: message','a.py:1: first\nsecond']):
        result.append(dict(id='parse-invalid-'+str(i),kind='parse',value=v))
    for i,line in enumerate([1,True,False,2**100,None,1.5,'value']):result.append(dict(id='format-'+str(i),kind='format',line=line))
    result.append(dict(id='unavailable-extractor',kind='unavailable'))
    for name,files in [
        ('empty',[]),('clean',[dict(path=OWNER,source='pass\n')]),('positive',[dict(path=OWNER,source=BAD)]),
        ('helper',[dict(path=OWNER,source='def _generated_index(*, source: IndexSource):\n return Index(source=source)\n')]),
        ('duplicates',[dict(path=OWNER,source=BAD+BAD)]),('order',[dict(path=OWNER,source='def _generated_index(*, source: str=MODEL_DECLARED):\n Index()\n')]),
        ('invalid',[dict(path=OWNER,source='Index(\n')]),('trigger-only',[dict(path=OWNER,source='# Index(\n')]),
        ('other-owner',[dict(path='impl/other.py',source=BAD)]),
        ('file-link',[dict(path='shared/real.py',source=BAD),dict(path=OWNER,symlink='../../../shared/real.py')]),
        ('directory-link',[dict(path='shared/generic_sparse.py',source=BAD),dict(path='impl/common/l7245_versioned_backends',symlink='../../shared')]),
        ('utf8',[dict(path=OWNER,rawBytes='ff')])]:
        result.append(dict(id='production-'+name,kind='production',files=files))
    for name,arguments,files in [
        ('clean',[],[]),('positive',[],[dict(path=OWNER,source=BAD)]),('check-positive',['--check'],[dict(path=OWNER,source=BAD)]),
        ('check-clean',['--check'],[dict(path=OWNER,source='pass\n')]),('invalid',[],[dict(path=OWNER,source='Index(\n')]),
        ('other-owner',['--check'],[dict(path='impl/other.py',source=BAD)]),('help',['--help'],[]),('bad-option',['--bad'],[])]:
        result.append(dict(id='cli-'+name,kind='cli',arguments=arguments,files=files))
    for name,text in [('positive',BAD),('clean','pass\n'),('no-trigger-alias','from l1000_schema.schema_def import Index as Idx\nIdx()\n')]:
        result.append(dict(id='registry-extract-'+name,kind='registry-extract',source=text))
    for name,paths in [('duplicates',['b.py','a.py','b.py']),('extensions',['a.py','.py','a.txt']),
                       ('tests',['tests/a.py','test/a.py']),('relative',['a.py','b.py']),('empty',[])]:
        result.append(dict(id='batch-'+name,kind='batch',paths=paths,relative=name=='relative'))
    return result


def exercise(module,case,scratch):
    root=scratch/'repo';root.mkdir();kind=case['kind']
    try:
        if kind=='metadata':
            scope=module.FileScopeSpec(directories=(root/'custom',),explicit_files=(Path('extra.py'),),include_tests=True,broad_repo_scope=True)
            plugin=(module._generated_index_sources_plugin(scope) if case.get('custom') else module.generated_index_sources_plugin(root))
            value=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description,scope=asdict(plugin.file_scope),specs=[asdict(s) for s in plugin.required_facts],registry=list(module.generated_index_sources_fact_extractors(root)))
        elif kind=='registration':
            from scripts import lint_all
            registration=next(r for r in lint_all.DL_SUITE.linter_plugin_factories if CHECK in r.names)
            plugin=registration.factory()
            value=dict(names=registration.names,name=plugin.name,enforcement=plugin.enforcement,description=plugin.description,scope=asdict(plugin.file_scope),specs=[asdict(s) for s in plugin.required_facts])
        elif kind in {'extract','registry-extract'}:
            from scripts.lint_lib.fact_engine import materialize_source_facts
            bundle=(module._rerail_generated_extract(root/'a.py',case['source']) if kind=='extract' and hasattr(module,'_rerail_generated_extract') else
                materialize_source_facts(path=root/'a.py',source=case['source'],specs=(module.GENERATED_INDEX_SOURCES_EXTRACTOR,),extractors=module.generated_index_sources_fact_extractors(root)))
            value=dict(facts=bundle.facts,errors=bundle.errors)
        elif kind=='report':
            from scripts.lint_lib.linter_plugin import PerFileFactBundle,ProjectFactView
            plugin=module.generated_index_sources_plugin(root)
            bundles={Path(b['key'].replace('$ROOT',str(root))):PerFileFactBundle(Path(b.get('path',b['key'])),'fixture',facts=b.get('facts',{module.GENERATED_INDEX_SOURCES_EXTRACTOR.extractor_id:b.get('value')})) for b in case['bundles']}
            value=plugin.reporter(ProjectFactView(root,bundles,plugin.required_facts))
        elif kind=='batch':
            for name in set(case['paths']):
                p=root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(BAD)
            with patch.object(module,'REPO_ROOT',root):
                value=module._run_fact_engine_for_files([Path(p) if case.get('relative') else root/p for p in case['paths']])
        elif kind=='parse':value=module._parse_violation_line(case['value'])
        elif kind=='format':value=module._format_finding(module.StructuredViolation(CHECK,'missing_source',Path('a.py'),case['line'],'message'))
        elif kind=='unavailable':value=module._extract_generated_index_source_facts(root/'a.py','Index()\n',ast.parse('pass'))
        elif kind=='production':
            if hasattr(module,'_rerail_generated_production'):
                write_fixture(root,[f for f in case['files'] if 'rawBytes' not in f])
                for f in case['files']:
                    if 'rawBytes' in f:
                        p=root/f['path'];p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(bytes.fromhex(f['rawBytes']))
                value=module._rerail_generated_production(root)
            else:
                if any('rawBytes' in f for f in case['files']):
                    from scripts.lint_lib.fact_engine import FactEngine
                    from scripts.lint_lib.fact_store import JsonFactStore
                    for f in case['files']:
                        p=root/f['path'];p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(bytes.fromhex(f['rawBytes']))
                    plugin=module.generated_index_sources_plugin(root)
                    value=FactEngine(root=root,fact_store=JsonFactStore(root/'cache'),extractors=module.generated_index_sources_fact_extractors(root)).materialize((plugin,),max_workers=1)
                else:value=source_case(module,dict(case,check=CHECK))
        elif kind=='cli':
            write_fixture(root,case['files']);out=io.StringIO();err=io.StringIO()
            with patch.object(sys,'argv',['lint_generated_index_sources.py','--root',str(root),*case['arguments']]),contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
                try:status=module.cli_main();raised=None
                except SystemExit as error:status=error.code;raised='SystemExit'
            value=dict(status=status,raised=raised,stdout=out.getvalue(),stderr=err.getvalue())
        else:raise AssertionError(kind)
        return dict(value=normalize(canonical(value,scratch),scratch))
    except Exception as error:return dict(error=dict(type=type(error).__name__,args=canonical(error.args,scratch)))


if __name__=='__main__':
    main(CHECK,'rerail.generated-indexes-integration/v1',cases,exercise,install,
         ['generated_indexes_integration_contract.py','generated_indexes_port.py','first_ten_production_contract.py'])
