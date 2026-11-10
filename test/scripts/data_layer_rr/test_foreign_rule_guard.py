"""A shared suite must not silently execute an unselected Python linter."""
from pathlib import Path
import sys
import tempfile
import threading
import unittest

ROOT=Path(__file__).absolute().parents[3];sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))


class ForeignRuleGuardTests(unittest.TestCase):
    def test_imported_passive_dataclass_can_be_defined(self):
        from suite import Suite
        with tempfile.TemporaryDirectory() as temporary:
            suite=Suite.__new__(Suite);suite.source=Path(temporary);suite.checks=[]
            suite.semantic_fallbacks=[];suite.host_protocol_calls=[];suite.item=None
            namespace={'__name__':'inspection_test_dto'}
            from dataclasses import dataclass
            namespace['dataclass']=dataclass
            try:
                suite.pytest_configure()
                exec(compile('@dataclass\nclass Value:\n text: str\n',str(suite.source/'scripts/lint_neighbor.py'),'exec'),namespace)
                self.assertEqual('value',namespace['Value']('value').text)
            finally:sys.setprofile(None);threading.setprofile(None)
            self.assertEqual([],suite.semantic_fallbacks)

    def test_unselected_linter_is_rejected(self):
        from suite import Suite
        with tempfile.TemporaryDirectory() as temporary:
            suite=Suite.__new__(Suite);suite.source=Path(temporary);suite.checks=[]
            suite.semantic_fallbacks=[];suite.host_protocol_calls=[];suite.item=None
            namespace={}
            exec(compile('def scan():\n return []\n',str(suite.source/'scripts/lint_neighbor.py'),'exec'),namespace)
            try:
                suite.pytest_configure()
                with self.assertRaisesRegex(AssertionError,'Unselected original linter'):
                    namespace['scan']()
            finally:sys.setprofile(None);threading.setprofile(None)
            self.assertEqual(['neighbor:scan'],suite.semantic_fallbacks)


if __name__=='__main__':unittest.main()
