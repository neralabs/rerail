"""Require successful native execution to expose each disabled source-graph behavior."""
import argparse
import json
from pathlib import Path
import re

from migration import ROOT, write_json
from replay import execute_native
from survey_source_contract import native_rows, check_target_contracts


def inline(path, seen=None):
    seen = set() if seen is None else seen
    path = path.resolve()
    if path in seen:
        return ''
    seen.add(path)
    return re.sub(r'^#include "(data-layer/[^\"]+)"$',
                  lambda m: inline(ROOT/'docs/rule-syntax/lib'/m[1], seen),
                  path.read_text(), flags=re.M)


def specs():
    # Every replacement applies to one named production clause. The unchanged
    # source recording is the expected result; errors do not count as detection.
    return [
        ('self-target', 'member-self-and-super', 'sgr_target(sid, i, target) :- sgr_query(sid, i, "self_attr", _, attr), sgr_owner(sid, c), cm_lookup', ' :- ', ' :- 1 = 0, '),
        ('super-target', 'member-self-and-super', 'sgr_target(sid, i, target) :- sgr_query(sid, i, "super_attr"', ' :- ', ' :- 1 = 0, '),
        ('mixin-cobase-order', 'member-cobase-order', 'sgr_mixin_candidate(c, sub, attr, target, i) :-', 'i = min j', 'i = max j'),
        ('parameter-first', 'member-last-class-first-parameter', 'sgr_first_parameter(sid, name, has, text) :-', 'i = min j', 'i = max j'),
        ('exact-unannotated-blocks-fallback', 'member-unannotated-exact-inherited-hint', 'sgr_init_hint(c, attr, text) :-', ', !sgr_first_parameter(init, attr, _, _)', ''),
        ('init-before-class-hint', 'member-constructor-and-class-annotations', 'sgr_hint_here(c, attr, text) :- sgr_class_hint', ', !sgr_init_hint(c, attr, _)', ''),
        ('hint-caller-module', 'member-inherited-hint-caller-module', 'sgr_target(sid, i, target) :- sgr_query(sid, i, "chained_self_attr"', 'sgn_context(_, sid, module, _)', 'module = "base"'),
        ('import-module-blocks', 'member-import-module-blocks-parameter', 'sgr_after_module(sid, base, attr, module) :-', ', !sgr_import_module(sid, base, attr, _)', ''),
        ('import-class-blocks', 'member-import-class-blocks-local', 'sgr_after_import_class(sid, base, attr, module) :-', ', !sgr_import_class(sid, base, attr, _)', ''),
        ('local-class-blocks', 'member-local-class-blocks-parameter', 'sgr_after_local_class(sid, base, attr) :-', ', !sgr_local_class(sid, base, attr, _)', ''),
        ('class-call-separation', 'edges-function-class-dedup', 'sge_call(sid, target) :- sgr_target', 'k != "class"', 'k = "class"'),
        ('class-use', 'edges-function-class-dedup', 'sge_class_use(sid, target) :-', ' :- ', ' :- 1 = 0, '),
        ('caller-inverse', 'edges-function-class-dedup', 'sge_caller(target, sid) :-', ' :- ', ' :- 1 = 0, '),
        ('legacy-sync', 'edges-legacy-sync-async', 'sge_legacy_name(sid, cat(name, "_lazy")) :-', ' :- ', ' :- 1 = 0, '),
        ('legacy-async-suffix', 'edges-legacy-sync-async', 'sge_legacy_name(sid, cat(@replace_all', '_async', '_absent'),
        ('legacy-sync-precedence', 'edges-legacy-sync-wins', 'sge_legacy_name(sid, cat(@replace_all', ', !sge_legacy(sid, "sync")', ''),
        ('legacy-inherited', 'edges-legacy-inherited-first', 'sge_call(sid, target) :- sge_legacy_name', 'cm_lookup(c, name, target)', 'sgc_method(c, name, _, target)'),
        ('legacy-property', 'edges-legacy-property-missing-free', 'sge_call(sid, target) :- sge_legacy_name', 'cm_lookup(c, name, target)', 'cm_lookup(c, name, target), !cm_property(target)'),
        ('exports-first-all', 'exports-first-all-only', 'sgx_all(d, a) :-', 'earlier(other, a)', 'earlier(a, other)'),
        ('exports-annotated-all', 'exports-annotated-all-ignored', 'sgx_all_candidate(d, a) :-', ', !assign_ann(a, _)', ''),
        ('exports-unsupported-all-blocks', 'exports-unsupported-all-blocks', 'sgx_export_name(d, i, name) :- sgx_name', '!sgx_all(d, _)', '!sgx_all_item(d, _, _)'),
        ('exports-duplicate-occurrences', 'exports-explicit-duplicates', 'sgx_export_name(d, i, name) :- sgx_all_item', 'sgx_export_name(d, i, name)', 'sgx_export_name(d, 0, name)'),
        ('exports-import-precedence', 'exports-import-over-definition', 'sgx_fq(d, name, fq) :- sgx_defined', ', !sgx_import(d, name, _)', ''),
        ('exports-top-level-imports', 'exports-top-level-only', 'sgx_import_candidate(d, a, name, fq) :-', 'sgx_top(d, i)', 'sgx_file(d, _, _)'),
        ('exports-direct-files', 'exports-direct-child-files-only', 'sgx_file(d, path, module) :-', '[^/]+', '.+'),
        ('exports-file-order', 'exports-sorted-paths', 'sgx_export(ordinal, module, name, sid) :-', '@str_cmp(other_path, path) < 0', '@str_cmp(other_path, path) > 0'),
        ('exports-public-roots', 'exports-public-eligibility', 'sgx_root(method) :-', 'cm_public(sid, _, method)', 'cm_dunder(sid, _, method)'),
        ('exports-class-roots', 'exports-public-eligibility', 'sgx_class(sid) :-', ' :- ', ' :- 1 = 0, '),
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    baseline = json.loads(args.baseline.read_text())
    check_target_contracts(baseline['cases'])
    cases = {c['id']:c for c in baseline['cases']}
    program = inline(ROOT/'test/fixtures/rr/survey-source.rr')
    result = dict(format='rerail.survey-source-controls/v1', fullMigrationAcceptance=False, controls=[])
    for name,label,prefix,before,after in specs():
        changed = []
        lines = []
        for line in program.splitlines():
            replacement = line.replace(before, after, 1) if line.startswith(prefix) else line
            if replacement != line:
                changed.append(dict(before=line, after=replacement))
            lines.append(replacement)
        if len(changed) != 1:
            raise ValueError(f'{name}: expected exactly one changed clause, got {len(changed)}')
        path = args.output.parent/'controls'/name/'rule.rr'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text('\n'.join(lines)+'\n')
        case = cases[label]
        native = execute_native(args.engine, path, case['files'], {})
        actual = native_rows(native['diagnostics'])
        if actual == case['expected']:
            raise AssertionError(f'{name}: mutation survived')
        result['controls'].append(dict(name=name, case=label, changed=changed, program=path.read_text(),
                                       expected=case['expected'], actual=actual, native=native))
        write_json(args.output, result)
        print(f'{name}: complete row mismatch through successful native execution', flush=True)
    result['passed'] = len(result['controls']) == len(specs())
    write_json(args.output, result)


if __name__ == '__main__':
    main()
