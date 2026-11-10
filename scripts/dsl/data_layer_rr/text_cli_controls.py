"""Require source-derived positive CLI cases to reject disabled native behavior."""
import argparse
import importlib
import inspect
from pathlib import Path
import sys
import tempfile

from migration import ROOT, PIN, compare, git, read_json, sha, write_json
from replay import execute_native
from text_cli_contract import exercise
from text_cli_port import install
from grammar_surface_port import install as surface_install


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['source', 'engine', 'baseline', 'out']:
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args(); source = args.source.absolute(); compare(git(source, 'rev-parse', 'HEAD'), PIN)
    baseline = read_json(args.baseline); assert baseline['passed']; compare(baseline['sourcePin'], PIN)
    sys.path[:0] = [str(source), str(source/'scripts')]
    library = ROOT/'docs/rule-syntax/lib/data-layer/text-cli.rr'; original = library.read_text()
    controls = [
        ('backend-counts', 'backend_type_strings', 'duplicates-check',
         'to_string(n)," backend type string violations:', 'to_string(n+1)," backend type string violations:'),
        ('source-unicode-preview', 'no_source_inspection', 'duplicates-check', '@slice(t,0,80)', '@slice(t,0,79)'),
        ('async-tuple-sort', 'hasattr_async', 'ordering', '1,0,n,cat("  ",path', '1,0,i,cat("  ",path'),
        ('surface-documentation', 'serverplan_grammar_surface', 'one',
         ':- tcli_kind("surface"), tcli_count(n), n > 0.',
         ':- tcli_kind("surface"), tcli_count(n), n > 0, option("disabled-control","true").'),
        ('backend-check-exit', 'backend_type_strings', 'one-check',
         'TextCliExit(@place("",0),1)', 'TextCliExit(@place("",0),0)'),
    ]
    report = dict(format='rerail.text-cli-controls/v1', sourcePin=PIN, engineSha256=sha(args.engine.read_bytes()),
        baselineSha256=sha(args.baseline.read_bytes()), harnessSha256=sha(Path(__file__).read_bytes()), passed=False, controls=[])
    with tempfile.TemporaryDirectory(prefix='text-cli-controls-', dir=ROOT/'.rerail') as folder:
        for name, check, case_id, before, after in controls:
            compare(original.count(before), 1)
            path = ROOT/'gate/data-layer/packs/data-layer/candidates/rr'/(check.replace('_','-')+'.rr')
            mutated = path.read_text().replace('#include "data-layer/text-cli.rr"', original.replace(before, after))
            policy = Path(folder)/(name+'.rr'); policy.write_text(mutated)
            class Backend:
                def __init__(self): self.calls=[]
                def replace(self, module, attr, value): setattr(module, attr, value)
                def native(self, _check, entry, files, options=None):
                    result = execute_native(args.engine.absolute(), policy, files, dict(entryPoint=entry, **(options or {})))
                    self.calls.append(result); return result['diagnostics']
            backend = Backend(); module = importlib.reload(importlib.import_module('scripts.lint_'+check))
            source_codes = {v.__code__ for v in vars(module).values() if inspect.isfunction(v) and v.__module__ == module.__name__}
            if check == 'serverplan_grammar_surface': surface_install(backend, module)
            else: install(backend, module, check)
            def guard(frame, event, _arg):
                if event == 'call' and frame.f_code in source_codes:
                    raise AssertionError('Source semantics executed by a control')
            row = next(r for r in baseline['cases'] if r['input']['check'] == check and r['input']['id'] == case_id)
            with tempfile.TemporaryDirectory(prefix='text-cli-') as temporary:
                sys.setprofile(guard)
                actual = exercise(module, row['input'], Path(temporary))
                sys.setprofile(None)
            report['controls'].append(dict(name=name, input=row['input'], expected=row['expected'], actual=actual,
                rejected=actual != row['expected'] and 'error' not in actual, mutation=dict(before=before, after=after),
                basePolicySha256=sha(path.read_bytes()), librarySha256=sha(library.read_bytes()),
                policy=mutated, native=backend.calls))
            write_json(args.out, report)
    report['passed'] = all(r['rejected'] for r in report['controls']); write_json(args.out, report)
    print(dict(controls=len(controls), passed=report['passed'])); raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__': main()
