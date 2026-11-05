# SPDX-License-Identifier: Apache-2.0
"""Record structural counterexamples using the original public detector APIs."""
import ast
import importlib
import tempfile
from pathlib import Path


def record(source):
    from data_layer_nine.suite import normalize

    cases = []
    matching = importlib.import_module('scripts.lint_matching_architecture')

    def match(label, path, text, application=False):
        expected = normalize(matching.scan_sources({Path(path): text}, application_paths=frozenset({Path(path)}) if application else frozenset()), source)
        parameters = {'applicationPaths': [path]} if application else {}
        cases.append(dict(test='structural-matching-' + label, check='matching_architecture',
                          entry='scan_sources', parameters=parameters, production=False,
                          files=[dict(path=path, source=text)], expected=expected))
        if label in {'annotation-none', 'annotation-call-attribute', 'nested-validation',
                     'multiline-counter', 'assignment-and-counter', 'keyword-and-assignment'} or label.startswith('order-'):
            cases.append(dict(test='structural-tree-' + label, check='matching_architecture',
                              entry='_scan_tree', parameters=parameters, production=False,
                              files=[dict(path=path, source=text)],
                              expected=normalize(matching._scan_tree(Path(path), ast.parse(text), application_path_override=application), source)))

    for label, annotation in [
        ('none', 'None'), ('none-union', 'None | str'), ('none-subscript', 'list[None]'),
        ('qualified', 'typing.Any'), ('call-attribute', 'factory().str'),
        ('constant-attribute', '"value".str'), ('subscript-attribute', 'items[0].str'),
        ('qualified-subscript', 'typing.List[str]'), ('union-attribute', '(str | int).str'),
        ('subscript-union', 'list[str] | int'), ('nested-union', 'int | (bytes | str)'),
    ]:
        match('annotation-' + label, 'l1470_matching_contracts/probe.py',
              'class MatchProbe:\n    field: ' + annotation + '\n')
        match('dispatch-' + label, 'l5250_matching/probe.py',
              'def dispatch(value: ' + annotation + '):\n    return value\n')
    for label, decorator in [('bare', 'dataclass'), ('qualified', 'dataclasses.dataclass'),
                             ('called', 'dataclass(frozen=True)'), ('double-call', 'dataclass()()'),
                             ('attribute-after-call', 'factory().dataclass'),
                             ('attribute-after-subscript', 'items[0].dataclass')]:
        match('decorator-' + label, 'l1470_matching_contracts/probe.py',
              '@' + decorator + '\nclass MatchProbe:\n    pass\n')
    for label, base in [('protocol', 'Protocol'), ('called-protocol', 'factory().Protocol'),
                       ('subscript-protocol', 'Protocol[T]'), ('model', 'BaseModel'),
                       ('called-model', 'factory().BaseModel')]:
        for config_label, config in [('missing', 'pass'),
                                     ('frozen', 'model_config = ConfigDict(frozen=True)'),
                                     ('numeric', 'model_config = ConfigDict(frozen=1)'),
                                     ('call-prefix', 'model_config = factory().ConfigDict(frozen=True)')]:
            match('base-' + label + '-' + config_label, 'l1470_matching_contracts/probe.py',
                  'class MatchProbe(' + base + '):\n    ' + config + '\n')
    for label, module in [('ascii', 'l1700_other'), ('arabic', 'l١٧٠٠_other'),
                          ('fullwidth', 'l１７００_other'), ('mixed', 'l1٧０0_other')]:
        for form in ['import ' + module, 'from ' + module + ' import Thing']:
            match('conformance-import-' + label + '-' + form.split()[0],
                  'conform/l5250_matching/probe.py', form + '\n')
    for label, text in [
        ('none-handler', 'try:\n    work()\nexcept:\n    return fallback()\n'),
        ('different-handlers', 'try:\n    work()\nexcept Exception:\n    pass\nexcept ValueError:\n    return fallback()\n'),
        ('call-handler', 'try:\n    work()\nexcept factory().MatchError:\n    return fallback()\n'),
        ('tuple-handler', 'try:\n    work()\nexcept (Exception, ValueError):\n    return fallback()\n'),
        ('nested-helper', 'def helper(spec):\n    def inner():\n        return match(txn, spec)\n    return inner\nhelper("lookup")\n'),
        ('duplicate-helper', 'def helper(spec):\n    return match(txn, spec)\ndef helper(other):\n    return other\nhelper("lookup")\n'),
        ('last-delegate-argument', 'def helper(first, second):\n    match(txn, first)\n    match(txn, second)\nhelper("lookup", value)\n'),
        ('nested-validation', 'def outer():\n    def inner():\n        return model.model_copy(update={})\n    return inner\n'),
        ('multiline-counter', 'def count():\n    return MatchDatabaseRoundTripCount(\n        len(STAGES_SQL_STAGES)\n    )\n'),
        ('assignment-and-counter', 'MatchStageOperationCount(hydration_operations); planned_database_round_trips = 1\n'),
        ('keyword-and-assignment', 'receipt(database_round_trips=len(database_receipts)); database_round_trips = len(database_receipts)\n'),
    ]:
        if 'handler' in label:
            text = 'def run():\n' + ''.join('    ' + line + '\n' for line in text.splitlines())
        match(label, 'application/probe.py' if 'helper' in label or label == 'last-delegate-argument'
              else 'impl/common/l5250_matching/probe.py', text)
    match('order-application-aggregates', 'application/probe.py', 'physical = "hnsw"\nbackend = BackendType.POSTGRES\n')
    match('order-implementation-aggregates', 'impl/common/l5250_matching/probe.py', 'SqlRaw("sql")\nbackend = BackendType.POSTGRES\n')
    match('order-assignment-and-counter', 'impl/common/l5250_matching/probe.py',
          'database_round_trips = planned_database_round_trips = len(database_receipts)\n')
    for first, last in [(False, True), (True, False), (False, False), (True, True)]:
        match('first-model-config-' + str(first) + '-' + str(last), 'l1470_matching_contracts/probe.py',
              'class MatchProbe(BaseModel):\n    model_config = ConfigDict(frozen=' + str(first) + ')\n    model_config = ConfigDict(frozen=' + str(last) + ')\n')
    match('duplicate-delegate-parameter', 'application/probe.py',
          'def helper(spec, spec):\n    return match(txn, spec)\nhelper(value, "lookup")\n')
    for owner, path, application in [('conformance', 'conform/l5250_matching/probe.py', False),
                                     ('consumer-conformance', 'conform/l5250_matching/probe.py', True),
                                     ('consumer-facade', 'data_layer/matching.py', True)]:
        for label, text in [('separate-imports', 'import data_layer_testing, matching_operator\n'),
                            ('from-import', 'from data_layer_testing import matching_operator\n'),
                            ('multiple-boundaries', 'import os, impl.common, data_layer_testing, matching_operator\n')]:
            match('order-' + owner + '-' + label, path, text, application)

    hnsw = importlib.import_module('scripts.lint_hnsw_architecture')
    for label, text in [('render-no-trigger', 'def render():\n    pass\n'),
                        ('render-trigger', '# hnsw\ndef render():\n    pass\n'),
                        ('import-no-trigger', 'import sql_dialect\n'),
                        ('name-no-trigger', 'scored_pairs_query()\n'),
                        ('json-case', 'state = "jSoN"\n'),
                        ('module-docstring', '"json"\n'),
                        ('nested-string', 'def f():\n    if flag:\n        "json"\n')]:
        path = 'impl/common/l2100_sql_grammar/domain/sql_table_hnsw/algorithm.py'
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / path
            target.parent.mkdir(parents=True)
            target.write_bytes(text.encode())
            expected = normalize(hnsw.find_hnsw_architecture_findings((target,)), root)
        cases.append(dict(test='structural-hnsw-' + label, check='hnsw_architecture',
                          entry='find_hnsw_architecture_findings', parameters={}, production=False,
                          files=[dict(path=path, source=text)], expected=expected))
    # The original standalone inventory check is relative to cwd, not to an
    # arbitrary temporary root used to transport an absolute input filename.
    for label, text in [('clean', 'pass\n'), ('positive', '# hnsw\nscored_pairs_query()\n')]:
        path = 'impl/common/l2100_sql_grammar/domain/sql_table_hnsw/frontier.py'
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / path
            target.parent.mkdir(parents=True)
            target.write_bytes(text.encode())
            expected = normalize(hnsw.find_hnsw_architecture_findings((target,)), root)
        cases.append(dict(test='structural-hnsw-outside-cwd-' + label, check='hnsw_architecture',
                          entry='find_hnsw_architecture_findings',
                          parameters={'cwdExternalPaths': [path]}, production=False,
                          files=[dict(path=path, source=text)], expected=expected))
    return cases
