# SPDX-License-Identifier: Apache-2.0
"""Record structural counterexamples using the original public detector APIs."""
import importlib
import tempfile
from pathlib import Path


def record(source):
    from data_layer_nine.suite import normalize

    cases = []
    matching = importlib.import_module('scripts.lint_matching_architecture')

    def match(label, path, text):
        expected = normalize(matching.scan_sources({Path(path): text}), source)
        cases.append(dict(test='structural-matching-' + label, check='matching_architecture',
                          entry='scan_sources', parameters={}, production=False,
                          files=[dict(path=path, source=text)], expected=expected))

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
    ]:
        if 'handler' in label:
            text = 'def run():\n' + ''.join('    ' + line + '\n' for line in text.splitlines())
        match(label, 'application/probe.py' if 'helper' in label or label == 'last-delegate-argument'
              else 'impl/common/l5250_matching/probe.py', text)

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
    return cases
