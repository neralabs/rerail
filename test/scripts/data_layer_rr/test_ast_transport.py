"""Supplied AST transport retains source positions and visit multiplicity."""
import ast
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
from ast_transport import columns


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
