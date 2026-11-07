"""Storage DTO construction must not exempt detector methods from the source execution guard."""
import dataclasses
import importlib.util
from pathlib import Path
import types
import unittest
import sys

spec = importlib.util.spec_from_file_location('rr_native_guard', Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_audit/native_guard.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class GuardTests(unittest.TestCase):
    def test_bare_and_package_imports_share_guarded_module(self):
        module = types.ModuleType('scripts.lint_rr_alias_fixture')
        alias = 'lint_rr_alias_fixture'
        self.addCleanup(lambda: sys.modules.pop(alias, None))
        guard.bind_aliases({'fixture': module})
        self.assertIs(sys.modules[alias], module)

    def test_import_alias_cannot_hide_another_module(self):
        module = types.ModuleType('scripts.lint_rr_conflict_fixture')
        alias = 'lint_rr_conflict_fixture'
        sys.modules[alias] = types.ModuleType(alias)
        self.addCleanup(lambda: sys.modules.pop(alias, None))
        with self.assertRaisesRegex(RuntimeError, 'alias'):
            guard.bind_aliases({'fixture': module})

    def module(self):
        module = types.ModuleType('_rr_guard_fixture')
        sys.modules[module.__name__] = module
        self.addCleanup(lambda: sys.modules.pop(module.__name__, None))
        # Source-defined methods stay distinguishable from dataclass-generated storage methods.
        exec(compile('''from dataclasses import dataclass
from typing import NamedTuple
from functools import wraps
def closure_detector():
    def inspect_node(): return True
    return inspect_node
def decorate(function):
    @wraps(function)
    def wrapped(): return function()
    return wrapped
@decorate
def decorated_detector(): return True
class AstEventCollector: pass
@dataclass
class Collector(AstEventCollector):
    value: int
    def on_Call(self, node): return bool(node)
@dataclass
class Finding:
    value: int
    def detect(self): return self.value > 0
class TupleFinding(NamedTuple):
    value: int
''', 'pinned-source-linter.py', 'exec'), module.__dict__)
        return module

    def test_nested_detector_callback_is_guarded(self):
        module = self.module()
        callback = module.closure_detector()
        mapped = guard.codes({'check': module})
        self.assertIn(callback.__code__, mapped)
        self.assertFalse(mapped[callback.__code__][2])

    def test_decorated_detector_body_is_guarded(self):
        import inspect
        module = self.module()
        original = inspect.unwrap(module.decorated_detector)
        mapped = guard.codes({'check': module})
        self.assertIn(original.__code__, mapped)
        self.assertFalse(mapped[original.__code__][2])

    def test_dataclass_collectors_are_not_passive_dtos(self):
        module = self.module()
        mapped = guard.codes({'check': module})
        self.assertFalse(mapped[module.Collector.on_Call.__code__][2])

    def test_custom_dto_methods_are_not_semantic_fallbacks(self):
        module = self.module()
        mapped = guard.codes({'check': module})
        self.assertFalse(mapped[module.Finding.detect.__code__][2])

    def test_generated_storage_constructors_are_permitted(self):
        module = self.module()
        mapped = guard.codes({'check': module})
        constructor = module.Finding.__init__.__code__
        self.assertTrue(constructor not in mapped or mapped[constructor][2])

    def test_standard_wrapper_of_generated_repr_is_permitted(self):
        module = self.module()
        mapped = guard.codes({'check': module})
        self.assertTrue(mapped[module.Finding.__repr__.__code__][2])
        constructor = module.TupleFinding.__new__.__code__
        self.assertTrue(constructor not in mapped or mapped[constructor][2])


if __name__ == '__main__':
    unittest.main()
