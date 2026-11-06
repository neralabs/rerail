# SPDX-License-Identifier: Apache-2.0
"""Original results for dense call graphs and referenced static containers."""
import argparse
import importlib
import json
from pathlib import Path
import sys
import tempfile


def record(source, output):
    from data_layer_batch_eight import normalize
    from data_layer_probe_record import verify_sources
    from data_layer_ten.common import PIN
    sys.path[:0] = [str(source), str(source / 'scripts')]
    inventory = [dict(path=f'api/_inventory_{group}.py', source=''.join(f'def target_{i}():\n    return {i}\ndef caller_{i}():\n    return target_{i}()\n' for i in range(group*15,(group+1)*15))) for group in range(300)]
    intents = [dict(path=f'api/_inventory_{group}.py', source='class QueryExecuteIntent:\n    pass\n'+''.join(f'def target_{i}():\n    return QueryExecuteIntent()\ndef caller_{i}():\n    return target_{i}()\n' for i in range(group*15,(group+1)*15))) for group in range(300)]
    unused = [dict(path=f'impl/unused_{group}.py', source='from l2055_lazy_result.purity import pure_lazy\n'+''.join(f'STATE_{i} = {{0: 1}}\n' for i in range(400))+''.join(f'@pure_lazy\ndef derive_{i}(key):\n    return key\n' for i in range(400))) for group in range(2)]
    referenced = [dict(file) for file in unused]
    referenced[0]['source']+='@pure_lazy\ndef probe(key):\n    return STATE_0[key]\n'
    static = 'STATIC = {' + ','.join(f'{i}: ({i}, {i+1})' for i in range(1000)) + '}\n'
    cases = [
        ('lazy_architecture', 'graph-positive', 'scan_files', [*intents, dict(path='api/_probe.py', source='class QueryExecuteIntent:\n    pass\ndef mixed(plan):\n    _ctor = QueryExecuteIntent\n    step = _ctor()\n    plan.execute()\n')]),
        ('lazy_architecture', 'graph-clean', 'scan_files', [*inventory, dict(path='api/_probe.py', source='def mixed(plan):\n    return 1\n')]),
        ('pure_lazy_runtime_purity', 'static-positive', 'scan_paths', [dict(path='impl/static_probe.py', source='from l2055_lazy_result.purity import pure_lazy\n'+static+'@pure_lazy\ndef derive(key):\n    return STATIC[key]\n')]),
        ('pure_lazy_runtime_purity', 'static-clean', 'scan_paths', [dict(path='impl/static_probe.py', source='from l2055_lazy_result.purity import pure_lazy\n'+static+'@pure_lazy\ndef derive(key):\n    return key\n')]),
        ('pure_lazy_runtime_purity', 'references-positive', 'scan_paths', referenced),
        ('pure_lazy_runtime_purity', 'references-clean', 'scan_paths', unused),
    ]
    events = []
    for check, name, entry, files in cases:
        module = importlib.import_module('scripts.lint_' + check)
        with tempfile.TemporaryDirectory(prefix='rerail-global-query-') as directory:
            root = Path(directory); paths=[]
            for file in files:
                path=root/file['path']; path.parent.mkdir(parents=True,exist_ok=True);path.write_text(file['source']);paths.append(path)
            saved=module.ROOT;module.ROOT=root
            try:
                result=module.scan_files(paths) if entry=='scan_files' else module.scan_paths(paths,root=root)
                findings=normalize(check,result,files,lambda path:Path(path).relative_to(root).as_posix() if Path(path).is_absolute() else str(path))
            finally:module.ROOT=saved
        assert bool(findings)==name.endswith('positive'),(name,findings)
        events.append(dict(test='recovery:'+name,check=check,entry=entry,call=1,upstream_passed=True,scope=dict(include=['**/*.py'],exclude=[]),production_scope=False,files=files,findings=findings,oracle_config=dict(options=dict(oracleAnalyzer=True,entryPoint=entry)),**({'compare_fields':['qualname']} if check=='pure_lazy_runtime_purity' else {})))
    output.write_text(json.dumps(dict(format='rerail.data-layer-recording/v1',commit=PIN,source_mismatches=[],pytest_exit_code=0,checks=['lazy_architecture','pure_lazy_runtime_purity'],collected_tests=0,uncaptured_tests=[],events=events,loaded_source_files_checked=verify_sources(source,source,PIN)),indent=2)+'\n')


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--source',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();record(a.source.resolve(),a.output)
