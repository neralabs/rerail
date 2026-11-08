"""Preserve helper set values and caller AST strings without running a Python detector."""
import ast, json, sys, unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from dto import canonical
from ast_transport import visit_columns


class PatternValueTests(unittest.TestCase):
    def test_helper_sets_preserve_type_members_and_empty_values(self):
        actual=canonical({'category':{'b','a'},'empty':set()})
        self.assertEqual(actual,dict(kind='dict',items=[['category',dict(kind='set',items=['a','b'])],['empty',dict(kind='set',items=[])]]))
        self.assertNotEqual(canonical({'a'}),canonical(['a']))
        self.assertNotEqual(canonical(frozenset({'a'})),canonical({'a'}))

    def test_ast_literal_columns_roundtrip_lone_surrogates_and_keep_exit_order(self):
        tree=ast.parse('def outer(x: "Connection\\ud800"):\n    def inner(y: Connection): pass\n')
        data=visit_columns(tree);literal=data['astKinds'].index('Constant')
        encoded=data['astLiteralJson'][literal]
        self.assertEqual('Connection\ud800',json.loads(encoded))
        self.assertTrue(all(ord(c)<128 for c in encoded))
        outer,inner=[i for i,k in enumerate(data['astKinds']) if k=='FunctionDef']
        self.assertLess(int(data['astExitOrder'][inner]),int(data['astExitOrder'][outer]))
