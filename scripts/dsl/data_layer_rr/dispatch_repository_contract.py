"""Record dispatch repository scope, facts, metadata, and complete reporter results."""
import argparse
from contextlib import ExitStack,redirect_stderr,redirect_stdout
from dataclasses import asdict
import importlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch

from migration import ROOT,PIN,compare,git,sha,write_json
from dto import canonical
from replay import write_fixture


def cases(module):
    base=module._BASE_SERVER_PLAN_FILE
    root=module._GENERIC_DIRS[0]
    witness='def f(ctx):\n if ctx.backend_type == "sqlite": pass\n'
    rows=[]
    def add(name,files=(),**extra):rows.append(dict(id=name,files=list(files),**extra))
    def file(path,source=witness):return dict(path=path,source=source)
    add('empty');add('clean',[file(root+'/clean.py','pass\n')])
    add('generic-roots',[file(p+'/positive.py') for p in module._GENERIC_DIRS])
    add('outside',[file('impl/outside.py'),file('api/a.py'),file('other/a.py')])
    add('base-executor',[file(base,'def f(backend):\n return backend.transport\n')])
    add('base-clean',[file(base,'def f(backend):\n return backend.get_lowering_context().branch_table\n')])
    add('mixed-codes',[file(root+'/all.py','ctx.is_sql_backend\nisinstance(step,QueryStep)\ntype(step).__name__ == "QueryStep"\nif dialect: pass\n')])
    add('excluded-tests',[file(root+'/'+p+'/a.py') for p in ['test','tests','conform','normal']])
    add('test-names',[file(root+'/'+p) for p in ['test_a.py','a_test.py','a.py','conftest.py']])
    add('nested-boundaries',[file(root+'/'+p+'/a.py') for p in ['.git','.venv','venv','tmp','__pycache__','node_modules','.hidden']])
    add('extensions',[file(root+'/'+p) for p in ['.py','.hidden.py','a.py','a.PY','a.pyi','notes.txt']])
    add('ordering',[file(root+'/'+p) for p in ['z.py','a/z.py','a-.py','a.py']])
    add('duplicates',[file(root+'/a.py',witness+witness)])
    add('syntax',[file(root+'/broken.py','def broken(:\nctx.is_sql_backend\n')])
    add('late-boundaries',[file(p) for p in sorted(module._LATE_BOUNDARY_FILES)])
    add('cross-file-type',[file(base,'def f(engine:RenderEngine):\n engine.transport\n'),file(root+'/other.py','RenderEngine=BackendExecutor\n')])
    add('shared-source-scopes',[file(base,'def f(backend):\n backend.transport\n'),file(root+'/same.py','def f(backend):\n backend.transport\n')])
    add('alias-from-base',[file(base,'def f(backend):\n backend.transport\n'),dict(path=root+'/alias.py',symlink='../l2070_server_plan/base_server_plan.py')])
    add('alias-to-base',[file('impl/shared.py','def f(backend):\n backend.transport\n'),dict(path=base,symlink='../../shared.py')])
    inventory='scripts/allowlists/lint_serverplan_dispatch_architecture.txt'
    add('empty-inventory',[file(inventory,'# no suppressions\n \n')])
    add('nonempty-inventory',[file(root+'/a.py'),file(inventory,'entry\n')])
    add('bad-signature',[file(root+'/a.py')],signature='invalid')
    return rows


def observe(module,case):
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.fact_store import JsonFactStore
    with tempfile.TemporaryDirectory(prefix='dispatch-repository-') as temporary,ExitStack() as stack:
        root=Path(temporary);write_fixture(root,case['files'])
        stack.enter_context(patch.object(module,'ROOT',root))
        if 'signature' in case:stack.enter_context(patch.object(module,'INVENTORY_SIGNATURE',case['signature']))
        plugin=module.dispatch_architecture_plugin(root)
        engine=FactEngine(root=root,fact_store=JsonFactStore(root/'tmp/facts'),extractors=module.dispatch_fact_extractors(root),enable_fact_cache=False)
        stdout,stderr=io.StringIO(),io.StringIO()
        with redirect_stdout(stdout),redirect_stderr(stderr):
            plan=engine.plan([plugin]);view=engine.materialize((plugin,),max_workers=1)
            findings=module._findings(view);violations=plugin.reporter(view)
        return dict(selectedFiles=[p.relative_to(root).as_posix() for p in plan.files],
            facts=canonical({p:b.facts for p,b in sorted(view.files.items())},root),
            findings=canonical(tuple(findings),root),violations=canonical(violations,root),
            plugin=canonical(dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description,
                file_scope=asdict(plugin.file_scope),required_facts=tuple(asdict(f) for f in plugin.required_facts)),root),
            stdout=stdout.getvalue().replace(str(root),'$ROOT'),stderr=stderr.getvalue().replace(str(root),'$ROOT'))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True);parser.add_argument('--out',type=Path,required=True)
    args=parser.parse_args();source=args.source.absolute();compare(git(source,'rev-parse','HEAD'),PIN)
    path=source/'scripts/lint_serverplan_dispatch_architecture.py'
    compare(path.read_bytes(),subprocess.check_output(['git','-C',str(source),'show',PIN+':scripts/lint_serverplan_dispatch_architecture.py']))
    sys.path[:0]=[str(source),str(source/'scripts')];module=importlib.import_module('scripts.lint_serverplan_dispatch_architecture')
    paths=[Path(__file__),*[Path(__file__).with_name(n) for n in ['migration.py','dto.py','replay.py']]]
    hashes={p.relative_to(ROOT).as_posix():sha(p.read_bytes()) for p in paths}
    rows=[dict(input=c,expected=observe(module,c)) for c in cases(module)]
    for name,digest in hashes.items():compare(digest,sha((ROOT/name).read_bytes()))
    report=dict(format='rerail.dispatch-repository-source/v1',oracle=True,passed=True,admitted=False,
        sourcePin=PIN,sourceSha256=sha(path.read_bytes()),harnessFiles=hashes,cases=rows)
    write_json(args.out,report);print(dict(cases=len(rows),positives=sum(bool(c['expected']['violations']) for c in rows)))


if __name__=='__main__':main()
