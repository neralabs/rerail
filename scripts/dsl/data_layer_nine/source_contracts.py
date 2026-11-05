# SPDX-License-Identifier: Apache-2.0
"""Read pinned literal contracts, including unpacked set literals, without imports."""
import ast,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from data_layer_eight.contracts import constants as ordinary_constants


def constants(path):
    values=ordinary_constants(path)
    def members(node):
        if isinstance(node,ast.Name):return values[node.id]
        if isinstance(node,(ast.Set,ast.Tuple,ast.List)):
            return [value for item in node.elts for value in (members(item.value) if isinstance(item,ast.Starred) else [ast.literal_eval(item)])]
        raise ValueError('Expected a literal collection')
    for node in ast.parse(path.read_text()).body:
        if not isinstance(node,(ast.Assign,ast.AnnAssign)):continue
        value=node.value
        if not isinstance(value,ast.Call) or not isinstance(value.func,ast.Name) or value.func.id not in {'frozenset','set','tuple','list'} or len(value.args)!=1 or value.keywords:continue
        try:result={'frozenset':frozenset,'set':set,'tuple':tuple,'list':list}[value.func.id](members(value.args[0]))
        except (KeyError,ValueError,TypeError):continue
        for target in node.targets if isinstance(node,ast.Assign) else [node.target]:
            if isinstance(target,ast.Name):values[target.id]=result
    return values


def record_public_contracts(source,repo,output):
    """Observe actual configured discovery and parser behavior without native replay."""
    import hashlib,importlib,json,sys,tempfile
    from dataclasses import asdict
    from pathlib import Path
    sys.path[:0]=[str(source),str(source/'scripts')]
    from data_layer_probe_record import verify_sources
    from scripts.lint_lib.fact_engine import FactEngine,_run_plugin_reporter
    from scripts.lint_lib.fact_store import JsonFactStore
    modules={name:importlib.import_module('scripts.lint_'+name) for name in ('bootstrap_cache_health','bootstrap_lazy_sql','connection_mode_split','schema_architecture_plugin','bootstrap_engine_invariants_plugin')}
    scopes=[]
    for check,prefix in [('bootstrap_cache_health','conform'),('connection_mode_split','api')]:
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            paths=[prefix+'/probe.py']+[prefix+'/'+part+'/probe.py' for part in ['.git','.venv','venv','__pycache__','tmp','tests','test','.pytest_cache','node_modules','_tmp']]+['unrelated/probe.py']
            for path in paths:
                file=root/path;file.parent.mkdir(parents=True,exist_ok=True);file.write_text('pass\n')
            plugin=getattr(modules[check],check+'_plugin')(root)
            engine=FactEngine(root=root,fact_store=JsonFactStore(root/'cache'),extractors={})
            selected={path.relative_to(root).as_posix() for path in engine.plan((plugin,)).files}
            scopes.append(dict(ruleId='data-layer-'+check.replace('_','-'),paths=[dict(path=path,inScope=path in selected) for path in paths]))
    specs=[('bootstrap_cache_health','conform/test_probe.py','create_tables(\n'),('bootstrap_lazy_sql','l3200_metadata/bootstrap_state.py','table_exists_many(\n'),('bootstrap_eager_resolve','l3200_metadata/bootstrap_state.py','table_exists_many(\n'),('connection_mode_split','api/probe.py','class DatabaseConnection(\n'),('schema_architecture','l2000_connection/probe.py','SchemaTransaction = (\n'),('bootstrap_engine_invariants','l3200_metadata/bootstrap_state.py','bootstrap = (\n')]
    requests=[dict(id=check+'-parser',check=check,files=[dict(path=path,source=text)]) for check,path,text in specs]
    positive=dict(path='l3200_metadata/bootstrap_state.py',source='pass\n\n# COLD_APPLICATION\n')
    for identity,extra in [('invalid-coordinator',[dict(path='l3200_metadata/bootstrap_coordinator.py',source='bootstrap = (\n')]),('invalid-enum-with-coordinator',[dict(path='l3200_metadata/bootstrap_coordinator.py',source='pass\n'),dict(path='l3200_metadata/bootstrap_data_plan.py',source='class BootstrapDataPrimitiveKind(\n')]),('invalid-enum-without-coordinator',[dict(path='l3200_metadata/bootstrap_data_plan.py',source='class BootstrapDataPrimitiveKind(\n')])]:
        requests.append(dict(id=identity,check='bootstrap_engine_invariants',files=[positive,*extra]))
    results=[]
    for request in requests:
        check=request['check'];module_name={'bootstrap_eager_resolve':'bootstrap_lazy_sql','schema_architecture':'schema_architecture_plugin','bootstrap_engine_invariants':'bootstrap_engine_invariants_plugin'}.get(check,check)
        module=modules[module_name]
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            for file in request['files']:
                path=root/file['path'];path.parent.mkdir(parents=True,exist_ok=True);path.write_text(file['source'])
            marker=root/'scripts/lint_bootstrap_engine_invariants_plugin.py';marker.parent.mkdir();marker.write_text('# marker\n')
            bootstrap=modules['bootstrap_engine_invariants_plugin'].bootstrap_lint
            old_root=bootstrap.REPO_ROOT;bootstrap.REPO_ROOT=root
            try:
                plugin=getattr(module,check+'_plugin')(root)
                extractor_name='bootstrap_lazy_sql' if check=='bootstrap_eager_resolve' else check
                extractors=getattr(module,extractor_name+'_fact_extractors')(root)
                view=FactEngine(root=root,fact_store=JsonFactStore(root/'cache'),extractors=extractors).materialize((plugin,),max_workers=1)
                findings=_run_plugin_reporter(plugin,view).violations
                results.append(dict(**request,expected=[{**asdict(row),'path':row.path.relative_to(root).as_posix()} for row in findings],enforcement=plugin.enforcement,errors={path.relative_to(root).as_posix():bundle.errors for path,bundle in view.files.items() if bundle.errors}))
            finally:bootstrap.REPO_ROOT=old_root
    files=['scripts/lint_lib/fact_engine.py',*[str(Path(module.__file__).relative_to(source)) for module in modules.values()]]
    shared=dict(sourcePin='2b1504adc4291daaf080b2e932962b134da1a69a',sourceFilesChecked=verify_sources(source,repo,'2b1504adc4291daaf080b2e932962b134da1a69a'),sourceSha256={path:hashlib.sha256((source/path).read_bytes()).hexdigest() for path in sorted(files)})
    output.mkdir(parents=True,exist_ok=True)
    for filename,report in [('batch-09-public-scope-oracle.json',dict(format='rerail.data-layer-public-scope/v1',**shared,rules=scopes)),('batch-09-configured-defaults.json',dict(format='rerail.data-layer-configured-defaults/v1',**shared,cases=results))]:
        (output/filename).write_text(json.dumps(report,indent=2)+'\n')


if __name__=='__main__':
    import argparse,sys
    from pathlib import Path
    sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
    parser=argparse.ArgumentParser(description='Record configured batch-nine public contracts from the original implementation')
    parser.add_argument('--source',type=Path,required=True);parser.add_argument('--repo',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();record_public_contracts(args.source.resolve(),args.repo.resolve(),args.output)
