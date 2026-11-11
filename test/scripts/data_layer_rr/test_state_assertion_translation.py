"""Observer transfers retain assertions and read native scope rows."""
import ast
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from state_assertion_translation import translate


SOURCE='''
def test_function_fact_analysis_excludes_unreachable_functions(monkeypatch):
    source = 'def compile(): pass'
    captured = []
    original = tripwires._collect_function_facts
    def collect(*args, **kwargs):
        function_nodes = kwargs.get('function_nodes')
        assert isinstance(function_nodes, list)
        captured.extend(node.name for node, _class_stack in function_nodes)
        return original(*args, **kwargs)
    monkeypatch.setattr(tripwires, '_collect_function_facts', collect)
    violations = scan_source('input.py', source)
    assert any(v.code == 'SPT-003' for v in violations)
    assert set(captured) == {'compile', '_run_query'}
'''


class StateAssertionTranslationTests(unittest.TestCase):
    def execute(self,rows):
        tree=ast.parse(SOURCE);before=sorted(ast.dump(n) for n in ast.walk(tree) if isinstance(n,ast.Assert))
        self.assertTrue(translate(tree));ast.fix_missing_locations(tree)
        self.assertEqual(before,sorted(ast.dump(n) for n in ast.walk(tree) if isinstance(n,ast.Assert)))
        queries=[]
        def scope(*args):queries.append(args);return rows
        class Finding:code='SPT-003'
        ns=dict(scan_source=lambda *args:[Finding()],_rr_state_analysis_scope=scope)
        exec(compile(tree,'<native-observer-transfer>','exec'),ns)
        ns['test_function_fact_analysis_excludes_unreachable_functions'](None)
        self.assertEqual(queries,[('input.py','def compile(): pass','facts')])

    def test_assertions_observe_native_rows(self):
        self.execute([[1,'compile',''],[3,'_run_query','']])

    def test_broad_analysis_is_rejected(self):
        with self.assertRaises(AssertionError):
            self.execute([[1,'compile',''],[3,'_run_query',''],[5,'unreachable','']])

    def test_empty_analysis_is_rejected(self):
        with self.assertRaises(AssertionError):self.execute([])

    def test_native_input_collection_type_assertion_is_retained(self):
        with self.assertRaises(AssertionError):self.execute(((1,'compile',''),(3,'_run_query','')))

    def test_unreviewed_function_is_not_translated(self):
        tree=ast.parse('def test_other():\n    assert False\n')
        self.assertEqual(translate(tree),[])
