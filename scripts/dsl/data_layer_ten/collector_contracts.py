# SPDX-License-Identifier: Apache-2.0
"""Exercise collector ownership, replacement, and source-trigger boundaries."""
import ast
import importlib
import tempfile
from pathlib import Path


def record(source):
    from data_layer_nine.suite import normalize
    cases = []

    def observe(check, label, path, text, entry, files=None):
        module = importlib.import_module('scripts.lint_' + check)
        files = files or [dict(path=path, source=text)]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for file in files:
                target = root / file['path']
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(file['source'].encode())
            if entry == '_violations_for_sources':
                findings = module._violations_for_sources({f['path']: f['source'] for f in files})
            elif entry == '_collect_violations':
                findings = module._collect_violations(source / path, ast.parse(text))
            elif entry == 'collect_violations':
                findings = module.collect_violations(root)
            elif entry == 'check_file':
                findings = module.check_file(root / path)
            else:
                findings = getattr(module, entry)(Path(path), text)
            expected = normalize(findings, root if entry in {'collect_violations', 'check_file'} else source)
            if entry == 'check_file':
                expected = [finding.removeprefix(str(root) + '/') for finding in expected]
        cases.append(dict(test='collector-' + check + '-' + label, check=check, entry=entry,
                          parameters={}, production=False, files=files, expected=expected))

    graph_path = 'l7200_versioned_transactions/versioned_lazy_mixin.py'
    for label, text in [
        ('decorator-call', 'class VersionedReadTransactionLazyMixin:\n    @pure_lazy()\n    def graph_lazy(self): pass\n'),
        ('decorator-double-call', 'class VersionedReadTransactionLazyMixin:\n    @pure_lazy()()\n    def graph_lazy(self): pass\n'),
        ('last-method', 'class VersionedReadTransactionLazyMixin:\n    @pure_lazy\n    def graph_lazy(self): pass\n    def graph_lazy(self): pass\n'),
        ('duplicate-class', 'class VersionedReadTransactionLazyMixin:\n    @pure_lazy\n    def graph_lazy(self): pass\nclass VersionedReadTransactionLazyMixin:\n    pass\n'),
        ('conditional-method', 'class VersionedReadTransactionLazyMixin:\n    if flag:\n        @pure_lazy\n        def graph_lazy(self): pass\n'),
        ('nested-class', 'def outer():\n    class VersionedReadTransactionLazyMixin:\n        @pure_lazy\n        def graph_lazy(self): pass\n'),
    ]:
        observe('graph_lazy_sql_plan', label, graph_path, text, '_violations_for_sources')
    optimizer_path = 'impl/common/l2065_plan_core/passes.py'
    for label, text in [
        ('late-class', 'class PhysicalPass:\n    phase: object = Phase.PHYSICAL\nDEFAULT_V2_PASSES = (PhysicalPass(), NormalizePass())\nclass NormalizePass:\n    phase: object = Phase.NORMALIZE\n'),
        ('replacement-phase', 'class FirstPass:\n    phase: object = Phase.NORMALIZE\nclass FirstPass:\n    phase: object = Phase.PHYSICAL\nclass SecondPass:\n    phase: object = Phase.NORMALIZE\nDEFAULT_V2_PASSES = (FirstPass(), SecondPass())\n'),
        ('protocol-replacement', 'class FirstPass:\n    phase: object = Phase.PHYSICAL\nclass FirstPass(Protocol): pass\nclass SecondPass:\n    phase: object = Phase.NORMALIZE\nDEFAULT_V2_PASSES = (FirstPass(), SecondPass())\n'),
        ('first-phase', 'class FirstPass:\n    phase: object = Phase.PHYSICAL\n    phase: object = Phase.NORMALIZE\nclass SecondPass:\n    phase: object = Phase.NORMALIZE\nDEFAULT_V2_PASSES = (FirstPass(), SecondPass())\n'),
        ('nested-init', 'def __init__():\n    def inner():\n        obj.value = 1\n'),
        ('nested-hint', 'def outer():\n    raise ValueError("bad")\n    def inner():\n        return "Fix producer"\n'),
        ('relative-imports', 'from .requests import get\nfrom requests import get\n'),
    ]:
        observe('plan_optimizer_v2', label, optimizer_path, text, 'scan_file')
    for label, text in [
        ('aliases-fixed-point', 'def read(scope: PlanScope):\n    second = first\n    first = scope\n    return second._private\n'),
        ('tuple-target', 'def read(scope: PlanScope):\n    alias, other = scope\n    return alias._private\n'),
        ('attribute-target', 'def read(scope: PlanScope):\n    obj.alias = scope\n    return obj.alias._private\n'),
        ('nested-decorator', 'def read(scope: PlanScope):\n    @scope._private\n    def inner(): pass\n'),
        ('lambda', 'def read(scope: PlanScope):\n    return lambda: scope._private\n'),
        ('class-field', 'def read(scope: PlanScope):\n    @reusable_template()\n    class Record:\n        field: PlanScope = scope._private\n'),
        ('authority-in-string', 'def read(scope: "list[PlanScope]"):\n    return scope._private\n'),
        ('authority-prefix', 'def read(scope: OtherPlanScopeSuffix):\n    return scope._private\n'),
        ('subscript-receiver', 'def read(scope: PlanScope):\n    return scope[0]._private\n'),
    ]:
        observe('graphql_plan_ownership', label, 'l9750_graphql/probe.py', text, 'scan_source')
    for label, text in [
        ('session-chain', 'conn.session().session().read()\n'),
        ('unknown-session', 'other.session().read()\n'),
        ('attribute-connection', 'factory()._conn.read()\n'),
        ('nested-lazy', 'def outer_lazy():\n    def inner_lazy():\n        value.resolve()\n'),
        ('nested-nonlazy', 'def outer_lazy():\n    def inner():\n        value.resolve()\n'),
        ('nested-class', 'def outer_lazy():\n    class Inner:\n        def inner(self):\n            conn.read()\n            value.resolve()\n'),
        ('query-shadow', 'txn.query().query().resolve()\n'),
        ('query-chain', 'txn.query().filter().resolve()\n'),
        ('attribute-txn', 'owner._txn.query().execute()\n'),
        ('session-txn', 'txn.session().query().execute()\n'),
        ('publish-nested', 'def _publish_worker_scalar_int_ref():\n    def inner():\n        plan._add_server_step()\n'),
    ]:
        observe('embedding_vector_orchestration', label, 'l5200_vector_index/embedding_worker.py', text, 'scan_source')
    introspection = importlib.import_module('scripts.lint_schema_introspection_data_paths')
    marker = introspection.FORBIDDEN_SQL_TEXT[0]
    call = sorted(introspection.FORBIDDEN_CALL_NAMES)[0]
    for label, text in [
        ('standalone-strings', repr(marker) + '\nif flag:\n    ' + repr(marker) + '\n'),
        ('assigned-string', 'value = ' + repr(marker.upper()) + '\n'),
        ('joined-string', 'value = f"' + marker + ' {name}"\n'),
        ('nul-before-marker', 'value = "\\x00' + marker.upper() + '"\n'),
        ('nul-after-marker', 'value = "' + marker.upper() + '\\x00"\n'),
        ('nested-authority', 'def schema_bootstrap():\n    def inner():\n        ' + call + '()\n'),
        ('unrelated-authority', 'def unrelated():\n    ' + call + '()\n'),
        ('lambda-authority', 'def schema_bootstrap():\n    return lambda: ' + call + '()\n'),
    ]:
        observe('schema_introspection_data_paths', label, 'impl/common/l7245_versioned_backends/probe.py', text, '_collect_violations')
    for label, text in [
        ('nested-compile', 'class ConditionalStep:\n    def outer(self):\n        def compile():\n            child.compile(ctx)\n'),
        ('duplicate-init', 'class ConditionalStep:\n    def __init__(self):\n        _validate_branch_content()\n    def __init__(self):\n        _validate_branch_content()\n'),
        ('duplicate-class', 'class ConditionalStep:\n    def compile(self): raise TypeError\nclass ConditionalStep:\n    pass\n'),
        ('nested-class', 'class ConditionalStep:\n    class Other:\n        def compile(self):\n            child.compile(ctx)\n'),
        ('lambda-compile', 'class ConditionalStep:\n    def compile(self):\n        return lambda: child.compile(ctx)\n'),
    ]:
        observe('conditional_architecture', label, 'l2070_batching/conditional_step.py', text, 'collect_violations')
    for label, text in [
        ('source-posonly', 'def _generated_index(source: IndexSource, /):\n    return Index(source=source)\n'),
        ('source-vararg', 'def _generated_index(*source: IndexSource):\n    return Index(source=source)\n'),
        ('source-kwargs', 'def _generated_index(**source: IndexSource):\n    return Index(source=source)\n'),
        ('source-keyword-gap', 'def _generated_index(*, before, source: IndexSource = IndexSource.MODEL_DECLARED):\n    return Index(source=source)\n'),
        ('source-positional-default', 'def _generated_index(source: IndexSource = IndexSource.MODEL_DECLARED):\n    return Index(source=source)\n'),
        ('source-other-annotation', 'def _generated_index(*, source: owner.IndexSource):\n    return Index(source=source)\n'),
        ('nested-function', 'def _generated_index(*, source: IndexSource):\n    def inner():\n        return Index(source=source)\n    return inner\n'),
        ('import-in-function', 'def outer():\n    from l1000_schema.schema_def import Index as Ix\nIx()\n# _generated_index\n'),
    ]:
        observe('generated_index_sources', label, 'impl/common/l7245_versioned_backends/generic_sparse.py', text, 'check_file')
    return cases
