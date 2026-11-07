"""Supplied AST transport retains source positions and visit multiplicity."""
import ast
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
from ast_transport import columns, syntax_columns


class AstTransportTests(unittest.TestCase):
    def test_positions_fields_and_operators_survive_without_printing(self):
        tree = ast.parse('\n\nf.annotation is not None\n')
        rows = columns(tree)
        attribute = rows['astKinds'].index('Attribute')
        self.assertEqual('3', rows['astLines'][attribute])
        self.assertEqual('annotation', rows['astAttrs'][attribute])
        self.assertEqual('left', rows['astFields'][attribute])
        none = rows['astKinds'].index('Constant')
        self.assertEqual('NoneType', rows['astValueTypes'][none])
        self.assertIn('IsNot', rows['astKinds'])
        self.assertEqual({len(rows['astKinds'])}, {len(values) for values in rows.values()})

    def test_shared_expression_has_one_occurrence_per_visit(self):
        expression = ast.parse('get_origin(f.annotation)', mode='eval').body
        tree = ast.Module(body=[ast.Expr(value=expression), ast.Expr(value=expression)], type_ignores=[])
        rows = columns(tree)
        self.assertEqual(2, rows['astKinds'].count('Attribute'))
        self.assertEqual(2, rows['astKinds'].count('Call'))

    def test_syntax_transport_preserves_supplied_positions_and_arguments(self):
        tree = ast.parse('async def f(custom: "ReadTransaction"):\n    custom.query(Model).execute()\n')
        ast.increment_lineno(tree, 40)
        rows = syntax_columns(tree)
        self.assertIn('custom', rows['astArgNames'])
        call = rows['astKinds'].index('Call')
        self.assertEqual('42', rows['astLines'][call])
        self.assertEqual('4', rows['astColumns'][call])
        transported = ast.parse(rows['receiverTransport'])
        self.assertEqual('call' + str(call), transported.body[0].args.args[0].arg)

    def test_literal_json_preserves_surrogates_losslessly(self):
        import json
        tree = ast.parse('async def f(custom: "\\ud800ReadTransaction"):\n    pass\n')
        rows = syntax_columns(tree)
        constant = rows['astKinds'].index('Constant')
        self.assertEqual('\ud800ReadTransaction', json.loads(rows['astStringJson'][constant]))
        rows['astStringJson'][constant].encode('utf-8')
