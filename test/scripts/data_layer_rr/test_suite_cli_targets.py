"""Repeated test-list arguments must not silently discard original assertions."""
import argparse
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import suite


class Parsed(Exception):
    def __init__(self,value):self.value=value


def parse_targets(extra):
    original=argparse.ArgumentParser.parse_args
    argv=['--source','/source','--engine','/engine','--checks','no_source_inspection','--out','/report',*extra]
    def capture(parser):raise Parsed(original(parser,argv))
    with patch.object(argparse.ArgumentParser,'parse_args',capture):
        try:suite.main()
        except Parsed as parsed:return parsed.value.tests
    raise AssertionError('Suite did not parse its command line')


class SuiteCliTargetsTests(unittest.TestCase):
    def test_one_list_retains_both_modules(self):
        self.assertEqual(parse_targets(['--tests','primary.py','shared.py']),['primary.py','shared.py'])

    def test_repeated_lists_retain_every_module_and_node(self):
        self.assertEqual(parse_targets(['--tests','primary.py','--tests','shared.py','other.py::test_positive']),
                         ['primary.py','shared.py','other.py::test_positive'])

    def test_no_list_preserves_direct_suite_default(self):
        self.assertIsNone(parse_targets([]))
