"""External AST identity and live CLI inputs must survive native test transport."""
import ast
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
from unresolved_port import identity_graph
from unresolved_cli_port import install as install_cli


class UnresolvedTransportTests(unittest.TestCase):
    def test_graph_retains_shared_node_identity_and_distinct_equal_nodes(self):
        first = ast.parse('f(x)', mode='eval').body
        own = identity_graph(first, first.args[0])
        other = identity_graph(first, ast.parse('x', mode='eval').body)
        self.assertEqual('0', own['containerId'])
        self.assertEqual('1', own['targetId'])
        self.assertIn(('0', '1'), list(zip(own['graphParents'], own['graphChildren'])))
        self.assertNotIn('1', other['graphChildren'])
        same = identity_graph(first, first)
        self.assertEqual(same['containerId'], same['targetId'])

    def test_native_cli_observes_later_discovery_monkeypatches(self):
        module = types.ModuleType('fixture_cli')
        module.__file__ = '/fixture_cli.py'
        module.resolve_root_and_dirs = lambda *_args,**_kwargs: (Path('/before'), [])
        module.analyze_file = lambda path: self.fail('Empty native selection must not call a detector')
        module.analyze_file._rerail_unresolved_file = True
        class Transport:
            def __init__(self):
                self.entries = []
            def replace(self, owner, name, value):
                setattr(owner, name, value)
            def native(self, check, entry, files, options):
                self.entries.append((entry,options))
                if entry=='_uf_cli_report':return [dict(code='UF_CLI_EXIT',parameters=dict(status=0,raised=1))]
                return []
        transport = Transport()
        schema=dict(description='fixture',epilog='',arguments=[
            dict(names=['paths'],kwargs=dict(nargs='*')),
            *[dict(names=['--'+n],kwargs=dict(action='store_true'))for n in ['check','verbose']]])
        install_cli(transport,module,dict(cli_schema=schema))
        module.resolve_root_and_dirs = lambda *_args,**_kwargs: (Path('/after'), ['impl'])
        with patch.object(sys,'argv',['fixture_cli.py']),self.assertRaises(SystemExit) as error:module.cli_main()
        self.assertEqual(error.exception.code,0)
        self.assertEqual(transport.entries[0],('_uf_cli_paths',dict(ufCliPaths=[],ufCliDefaultPaths=['/after/impl'])))
        self.assertEqual([e for e,_ in transport.entries],['_uf_cli_paths','cli_selection','cli_scan','_uf_cli_report'])
