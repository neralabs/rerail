"""Caller syntax and fixture paths must survive transaction assertion transport."""
import ast,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from transaction_contract import syntax,Contract


class TransactionTransportTests(unittest.TestCase):
    def test_zero_columns_and_missing_module_locations_are_numeric(self):
        values=syntax(ast.parse('items.append(1)\n'))
        self.assertEqual('0',values['astColumns'][0])
        self.assertEqual('0',values['astColumns'][1])
        self.assertTrue(all(c.isdecimal() for c in values['astColumns']))

    def test_exact_fixture_root_string_is_normalized_without_rewriting_neighbors(self):
        contract=Contract.__new__(Contract);contract.root=lambda:Path('/fixture/one')
        self.assertEqual('<fixture>',contract.value('/fixture/one'))
        self.assertEqual('<fixture>/a.py',contract.value('/fixture/one/a.py'))
        self.assertEqual('missing <fixture>/a.py',contract.value('missing /fixture/one/a.py'))
        self.assertEqual('/fixture/one-other/a.py',contract.value('/fixture/one-other/a.py'))
