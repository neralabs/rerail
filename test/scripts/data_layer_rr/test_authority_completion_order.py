"""Completion-order parity preserves exact file groups, duplicate findings, registry order, and stream bytes."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from execution_authority_bypass_production_contract import canonical_completion,render


class CompletionOrderTests(unittest.TestCase):
    def observation(self):
        prefix=[dict(path='registry.py',line=0,code='AUTH004',message='first'),dict(path='registry.py',line=0,code='AUTH005',message='second')]
        files=[dict(requestPath='a.py',rows=[dict(path='alias.py',line=3,code='AUTH001',message='one'),dict(path='alias.py',line=3,code='AUTH002',message='two')]),dict(requestPath='b.py',rows=[dict(path='b.py',line=7,code='AUTH001',message='same')]*2)]
        return self.finish(prefix,files)

    def finish(self,prefix,files):
        return dict(scanGroups=dict(prefix=prefix,files=files),scanRepo=prefix+[r for f in files for r in f['rows']],cli=[dict(argv=['--check'],status=1,stderr='',stdout=''.join(render(r) for r in prefix)+''.join(render(r) for f in files for r in f['rows']),groups=dict(prefix=''.join(render(r) for r in prefix),files=[dict(requestPath=f['requestPath'],text=''.join(render(r) for r in f['rows'])) for f in files]))],facts=[dict(path='a.py',code='AUTH001'),dict(path='b.py',code='AUTH001')])

    def test_whole_completion_groups_can_permute_without_mutating_raw_observations(self):
        original=self.observation();saved=copy.deepcopy(original);groups=original['scanGroups']
        actual=self.finish(groups['prefix'],list(reversed(groups['files'])))
        self.assertEqual(canonical_completion(original),canonical_completion(actual));self.assertEqual(original,saved)

    def test_reordering_a_file_or_registry_and_losing_a_duplicate_still_fails(self):
        original=self.observation()
        for change in ['file','registry','duplicate']:
            with self.subTest(change=change):
                groups=copy.deepcopy(original['scanGroups'])
                if change=='file':groups['files'][0]['rows'].reverse()
                if change=='registry':groups['prefix'].reverse()
                if change=='duplicate':groups['files'][1]['rows'].pop()
                actual=self.finish(groups['prefix'],groups['files'])
                self.assertNotEqual(canonical_completion(original),canonical_completion(actual))

    def test_an_unrepresented_stream_or_row_change_is_rejected(self):
        for key in ['stream','rows']:
            with self.subTest(key=key):
                actual=self.observation()
                if key=='stream':actual['cli'][0]['stdout']+='unexpected\n'
                else:actual['scanRepo'].pop()
                with self.assertRaises(AssertionError):canonical_completion(actual)

    def test_production_order_is_never_relaxed(self):
        original=self.observation();actual=copy.deepcopy(original);actual['facts'].reverse()
        self.assertNotEqual(canonical_completion(original),canonical_completion(actual))
