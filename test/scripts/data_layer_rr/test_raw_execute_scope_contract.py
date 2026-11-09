"""Combined acceptance compares complete diagnostics, including order and Unicode paths."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import raw_execute_scope_contract as contract


class RawExecuteScopeContractTests(unittest.TestCase):
    def expected(self):
        return [['l١٢٣٤_unicode/a.py','1','removed "raw" call',str(i)] for i in range(2)]

    def diagnostics(self):
        return [dict(policy='data-layer-raw-execute-regression',clause='RRERemoved',code='RER001',
                     path=p,line=int(line),at=0,tag=None,message=message,
                     parameters=dict(ordinal=int(ordinal),detail=message))
                for p,line,message,ordinal in self.expected()]

    def test_reads_engine_json_lines_without_a_tsv_escape_layer(self):
        rows=self.diagnostics()
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'rr_diagnostic.csv'
            path.write_text(''.join(json.dumps(r)+'\n' for r in rows))
            self.assertEqual(contract.read_diagnostics(path),rows)

    def test_complete_diagnostics_preserve_duplicate_occurrences(self):
        self.assertTrue(contract.matches(self.expected(),self.expected(),self.diagnostics()))
        self.assertTrue(contract.matches(self.expected(),self.expected(),list(reversed(self.diagnostics()))))

    def test_code_and_count_cannot_hide_wrong_diagnostic_fields(self):
        changes=[('path','other.py'),('line',2),('message','wrong'),('code','OTHER'),
                 ('policy','other'),('clause','Other'),('at',9),('tag','other'),
                 ('parameters',dict(ordinal=0,detail='wrong'))]
        for key,value in changes:
            with self.subTest(key=key):
                rows=self.diagnostics();rows[0][key]=value
                self.assertFalse(contract.matches(self.expected(),self.expected(),rows))
        rows=self.diagnostics();rows[1]=copy.deepcopy(rows[0])
        self.assertFalse(contract.matches(self.expected(),self.expected(),rows))
        self.assertFalse(contract.matches(self.expected(),self.expected()[:1],self.diagnostics()))
