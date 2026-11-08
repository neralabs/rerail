"""Canonical formatting must receive raw syntax, never Python's rendered result."""
import ast
import sys
import unittest
from pathlib import Path

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from ast_sources import Sources


class AstSourceTransportTests(unittest.TestCase):
    def test_raw_spacing_and_quotes_reach_the_native_printer(self):
        sources=Sources(ast.parse)
        tree=sources.parse('( a  +  "hello" )',mode='eval')
        self.assertEqual(sources.segment(tree.body),'a  +  "hello"')
        self.assertNotEqual(sources.segment(tree.body),ast.unparse(tree.body))

    def test_nested_source_spans_use_utf8_columns(self):
        sources=Sources(ast.parse)
        tree=sources.parse('é = call( α  +  β )')
        expr=tree.body[0].value.args[0]
        self.assertEqual(sources.segment(expr),'α  +  β')

    def test_multiline_arguments_preserve_source_bytes(self):
        sources=Sources(ast.parse)
        tree=sources.parse('value = (left +\n    right)\n')
        self.assertEqual(sources.segment(tree.body[0].value),'left +\n    right')

    def test_changed_tree_never_reuses_stale_source(self):
        sources=Sources(ast.parse)
        tree=sources.parse('left + right',mode='eval')
        tree.body.right.id='different'
        with self.assertRaisesRegex(NotImplementedError,'changed AST'):
            sources.segment(tree.body)

    def test_constructed_tree_never_invents_source(self):
        sources=Sources(ast.parse)
        with self.assertRaisesRegex(NotImplementedError,'constructed AST'):
            sources.segment(ast.Name(id='value'))

    def test_parse_errors_and_flags_keep_the_original_protocol(self):
        sources=Sources(ast.parse)
        with self.assertRaises(SyntaxError):sources.parse('class :')
        tree=sources.parse('value = 1 # type: int',type_comments=True)
        self.assertEqual(tree.body[0].type_comment,'int')
