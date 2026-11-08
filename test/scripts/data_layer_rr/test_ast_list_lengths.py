"""List cardinality must retain scalar and None members that do not produce AST child edges."""
import ast
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import ast_transport


class AstListLengthTests(unittest.TestCase):
    def test_unpacking_is_not_an_empty_dictionary(self):
        tree=ast.parse('cache = {**{}}\n')
        columns=ast_transport.columns(tree)
        lengths=ast_transport.list_lengths(tree)
        dictionaries=[i for i,k in enumerate(columns['astKinds']) if k=='Dict']
        rows={(int(owner),field):int(length) for owner,field,length in zip(
            lengths['astListOwners'],lengths['astListFields'],lengths['astListLengths'])}
        self.assertEqual(1,rows[dictionaries[0],'keys'])
        self.assertEqual(0,rows[dictionaries[1],'keys'])
        self.assertEqual(1,rows[dictionaries[0],'values'])
