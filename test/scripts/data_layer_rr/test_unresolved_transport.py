"""External AST identity and live CLI inputs must survive native test transport."""
import ast
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts/dsl/data_layer_rr'))
from unresolved_port import identity_graph, transfer_cli


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

    def test_transferred_cli_observes_later_discovery_monkeypatches(self):
        source = '''def cli_main():
    root_dir, directories = resolve_root_and_dirs()
    scan_paths = []
    all_findings = []
    for scan_path in scan_paths:
        all_findings.extend(analyze_file(scan_path))
    return root_dir, all_findings
'''
        module = types.ModuleType('fixture_cli')
        module.__file__ = '/fixture_cli.py'
        module.resolve_root_and_dirs = lambda: (Path('/before'), [])
        exec(source, module.__dict__)
        class Transport:
            def __init__(self):
                self.entries = []
            def replace(self, owner, name, value):
                setattr(owner, name, value)
            def native(self, check, entry, files, options):
                self.entries.append(entry)
                return []
        transport = Transport()
        with patch('unresolved_port.inspect.getsource', return_value=source):
            transfer_cli(transport, module)
        module.resolve_root_and_dirs = lambda: (Path('/after'), [])
        self.assertEqual((Path('/after'), []), module.cli_main())
        self.assertEqual(['cli_selection', 'cli_scan'], transport.entries)
