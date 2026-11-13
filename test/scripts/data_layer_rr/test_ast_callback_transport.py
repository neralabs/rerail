"""Callback transport must preserve syntax and the caller's child graph."""
import ast
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from ast_transport import columns, context_columns


class CallbackTransportTests(unittest.TestCase):
    def test_enclosing_body_is_not_copied_into_its_callback_header(self):
        tree=ast.parse('with opener() as p:\n'+''.join(' lazy.resolve()\n'for _ in range(1000))).body[0]
        data=columns(tree,omit_root_fields=('body','orelse'))
        self.assertEqual(data['astKinds'],['With','withitem','Call','Name','Load','Name','Store'])
        self.assertNotIn('resolve',data['astAttrs'])
        self.assertEqual(data['astParents'],['-1','0','1','2','3','1','5'])

    def test_callback_values_are_snapshots(self):
        node=ast.parse('lazy.resolve()',mode='eval').body
        before=columns(node);node.func.attr='clean';after=columns(node)
        self.assertIn('resolve',before['astAttrs']);self.assertIn('clean',after['astAttrs'])
        self.assertNotIn('clean',before['astAttrs'])

    def test_context_controls_children_even_outside_the_ast(self):
        node=ast.parse('visible',mode='eval').body
        class Context:
            def children(self,current):return (ast.Name(id='other'),)if current is node else ()
        self.assertEqual(context_columns(node,Context()),dict(astContextKinds=['Name','Name'],astContextIds=['visible','other']))


if __name__=='__main__':unittest.main()
