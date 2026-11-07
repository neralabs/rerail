"""Storage DTO construction must not exempt detector methods from the source execution guard."""
import dataclasses
import importlib.util
from pathlib import Path
import types
import unittest

spec = importlib.util.spec_from_file_location('rr_native_guard', Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_audit/native_guard.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class GuardTests(unittest.TestCase):
    def module(self):
        module = types.ModuleType('source_linter')
        # Source-defined methods stay distinguishable from dataclass-generated storage methods.
        exec(compile('''from dataclasses import dataclass
from typing import NamedTuple
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
        constructor = module.TupleFinding.__new__.__code__
        self.assertTrue(constructor not in mapped or mapped[constructor][2])


if __name__ == '__main__':
    unittest.main()
