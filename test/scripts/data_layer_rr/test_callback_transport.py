"""Supplied collector syntax must retain literals, identity and completion order."""
import ast,json,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from callback_wrappers_port import ast_input


class CallbackTransportTests(unittest.TestCase):
    def test_string_literal_survives_the_argument_transport(self):
        options,_=ast_input(ast.parse('getattr(txn, "versioned_read_async", None)\n'))
        values=[json.loads(s) for s in options['astStringJson'] if s]
        self.assertEqual(['versioned_read_async'],values)

    def test_postorder_and_shared_ast_identity_are_preserved(self):
        value=ast.Name(id='x',ctx=ast.Load());tree=ast.Module(body=[ast.Expr(value=value),ast.Expr(value=value)],type_ignores=[])
        options,_=ast_input(tree)
        names=[i for i,k in enumerate(options['astKinds']) if k=='Name']
        self.assertEqual(options['astIdentity'][names[0]],options['astIdentity'][names[1]])
        self.assertNotEqual(options['astExitOrder'][names[0]],options['astExitOrder'][names[1]])
        self.assertEqual(str(len(options['astKinds'])-1),options['astExitOrder'][0])
